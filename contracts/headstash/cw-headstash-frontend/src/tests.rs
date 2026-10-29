use super::*;
use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
use cosmwasm_std::{from_json, HexBinary};

fn sha(b: &[u8]) -> HexBinary {
    HexBinary::from(Sha256::digest(b).to_vec())
}

fn manifest(version: u32, data: &[u8], chunk: usize, activate: bool) -> (Manifest, Vec<Vec<u8>>) {
    let chunks: Vec<Vec<u8>> = data.chunks(chunk).map(|c| c.to_vec()).collect();
    let m = Manifest {
        version,
        entry: "app.html".into(),
        files: vec![FileEntry {
            path: "app.html".into(),
            content_type: "text/html".into(),
            encoding: "identity".into(),
            size: data.len() as u64,
            sha256: sha(data),
            stored_size: data.len() as u64,
            chunks: chunks.iter().map(|c| sha(c)).collect(),
        }],
        note: None,
        activate,
    };
    (m, chunks)
}

fn setup(max: u32) -> (cosmwasm_std::OwnedDeps<cosmwasm_std::testing::MockStorage, cosmwasm_std::testing::MockApi, cosmwasm_std::testing::MockQuerier>, cosmwasm_std::Addr, cosmwasm_std::Addr) {
    let mut deps = mock_dependencies();
    let gov = deps.api.addr_make("gov");
    let anyone = deps.api.addr_make("anyone");
    instantiate(deps.as_mut(), mock_env(), message_info(&anyone, &[]), InstantiateMsg { owner: Some(gov.to_string()), max_chunk_bytes: Some(max) }).unwrap();
    (deps, gov, anyone)
}

#[test]
fn publish_upload_activate_and_query() {
    let (mut deps, gov, anyone) = setup(8);
    let data = b"<p>hello headstash frontend</p>".to_vec();
    let (m, chunks) = manifest(1, &data, 8, true);
    // non-owner cannot publish
    assert!(execute(deps.as_mut(), mock_env(), message_info(&anyone, &[]), ExecuteMsg::PublishManifest { manifest: m.clone() }).is_err());
    execute(deps.as_mut(), mock_env(), message_info(&gov, &[]), ExecuteMsg::PublishManifest { manifest: m.clone() }).unwrap();
    // wrong data rejected
    assert!(execute(deps.as_mut(), mock_env(), message_info(&anyone, &[]), ExecuteMsg::UploadChunk { version: 1, file: 0, index: 0, data: Binary::from(chunks[1].clone()) }).is_err());
    for (i, c) in chunks.iter().enumerate().rev() {
        execute(deps.as_mut(), mock_env(), message_info(&anyone, &[]), ExecuteMsg::UploadChunk { version: 1, file: 0, index: i as u32, data: Binary::from(c.clone()) }).unwrap();
    }
    // duplicate rejected
    assert!(execute(deps.as_mut(), mock_env(), message_info(&anyone, &[]), ExecuteMsg::UploadChunk { version: 1, file: 0, index: 0, data: Binary::from(chunks[0].clone()) }).is_err());
    let cfg: ConfigResponse = from_json(query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap()).unwrap();
    assert_eq!(cfg.current, Some(1));
    assert_eq!(cfg.next_version, 2);
    let got: Manifest = from_json(query(deps.as_ref(), mock_env(), QueryMsg::Manifest { version: None }).unwrap()).unwrap();
    assert_eq!(got, m);
    let mut out = vec![];
    for i in 0..chunks.len() as u32 {
        let b: Binary = from_json(query(deps.as_ref(), mock_env(), QueryMsg::Chunk { version: 1, file: 0, index: i }).unwrap()).unwrap();
        out.extend_from_slice(b.as_slice());
    }
    assert_eq!(out, data);
    let st: StatusResponse = from_json(query(deps.as_ref(), mock_env(), QueryMsg::Status { version: 1 }).unwrap()).unwrap();
    assert!(st.complete);
}

#[test]
fn versions_are_immutable_and_set_current_requires_complete() {
    let (mut deps, gov, anyone) = setup(16);
    let (m1, c1) = manifest(1, b"version-one-bytes", 16, false);
    execute(deps.as_mut(), mock_env(), message_info(&gov, &[]), ExecuteMsg::PublishManifest { manifest: m1.clone() }).unwrap();
    // republishing version 1 (overwrite) rejected; skipping to 3 rejected
    assert!(execute(deps.as_mut(), mock_env(), message_info(&gov, &[]), ExecuteMsg::PublishManifest { manifest: m1.clone() }).is_err());
    let (m3, _) = manifest(3, b"x", 16, false);
    assert!(execute(deps.as_mut(), mock_env(), message_info(&gov, &[]), ExecuteMsg::PublishManifest { manifest: m3 }).is_err());
    assert!(execute(deps.as_mut(), mock_env(), message_info(&gov, &[]), ExecuteMsg::SetCurrent { version: 1 }).is_err());
    for (i, c) in c1.iter().enumerate() {
        execute(deps.as_mut(), mock_env(), message_info(&anyone, &[]), ExecuteMsg::UploadChunk { version: 1, file: 0, index: i as u32, data: Binary::from(c.clone()) }).unwrap();
    }
    let cfg: ConfigResponse = from_json(query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap()).unwrap();
    assert_eq!(cfg.current, None, "activate=false must not auto-activate");
    assert!(execute(deps.as_mut(), mock_env(), message_info(&anyone, &[]), ExecuteMsg::SetCurrent { version: 1 }).is_err());
    execute(deps.as_mut(), mock_env(), message_info(&gov, &[]), ExecuteMsg::SetCurrent { version: 1 }).unwrap();
    let v: VersionsResponse = from_json(query(deps.as_ref(), mock_env(), QueryMsg::Versions { start_after: None, limit: None }).unwrap()).unwrap();
    assert_eq!(v.versions.len(), 1);
}

#[test]
fn size_and_shape_limits() {
    assert!(instantiate(mock_dependencies().as_mut(), mock_env(), message_info(&cosmwasm_std::Addr::unchecked("a"), &[]), InstantiateMsg { owner: None, max_chunk_bytes: Some(HARD_MAX_CHUNK + 1) }).is_err());
    let (mut deps, gov, anyone) = setup(4);
    let (good, _) = manifest(1, b"abcdefgh", 4, false);
    let bad = |f: &dyn Fn(&mut Manifest)| { let mut m = good.clone(); f(&mut m); validate_manifest(&m, 1, 4).is_err() };
    assert!(bad(&|m| m.files[0].path = "../x".into()));
    assert!(bad(&|m| m.files[0].path = "/abs".into()));
    assert!(bad(&|m| m.files[0].path = "UPPER".into()));
    assert!(bad(&|m| m.entry = "missing".into()));
    assert!(bad(&|m| m.files[0].encoding = "br".into()));
    assert!(bad(&|m| m.files[0].sha256 = HexBinary::from(vec![0u8; 31])));
    assert!(bad(&|m| m.files[0].stored_size = 9)); // 2 chunks * 4 < 9
    assert!(bad(&|m| m.files[0].chunks = vec![HexBinary::from(vec![0u8; 32]); MAX_CHUNKS_PER_FILE + 1]));
    assert!(bad(&|m| { let f = m.files[0].clone(); m.files.push(f); }));
    assert!(bad(&|m| { let f = m.files[0].clone(); m.files = (0..=MAX_FILES).map(|i| { let mut g = f.clone(); g.path = format!("f{i}"); g }).collect(); m.entry = "f0".into(); }));
    assert!(validate_manifest(&good, 1, 4).is_ok());
    // oversized chunk rejected even if its hash were listed
    let big = vec![7u8; 5];
    let mut m = good.clone();
    m.files[0].chunks[0] = sha(&big);
    m.files[0].encoding = "gzip".into();
    execute(deps.as_mut(), mock_env(), message_info(&gov, &[]), ExecuteMsg::PublishManifest { manifest: m }).unwrap();
    assert!(execute(deps.as_mut(), mock_env(), message_info(&anyone, &[]), ExecuteMsg::UploadChunk { version: 1, file: 0, index: 0, data: Binary::from(big) }).is_err());
}

#[test]
fn renounce_freezes_current() {
    let (mut deps, gov, _) = setup(64);
    execute(deps.as_mut(), mock_env(), message_info(&gov, &[]), ExecuteMsg::UpdateOwnership(cw_ownable::Action::RenounceOwnership)).unwrap();
    let (m, _) = manifest(1, b"z", 64, false);
    assert!(execute(deps.as_mut(), mock_env(), message_info(&gov, &[]), ExecuteMsg::PublishManifest { manifest: m }).is_err());
}
