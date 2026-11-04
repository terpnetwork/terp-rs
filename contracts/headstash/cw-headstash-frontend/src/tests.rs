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

// ---------------- v2: prune + migrate ----------------

fn ex(deps: &mut cosmwasm_std::OwnedDeps<cosmwasm_std::testing::MockStorage, cosmwasm_std::testing::MockApi, cosmwasm_std::testing::MockQuerier>, who: &cosmwasm_std::Addr, msg: ExecuteMsg) -> StdResult<Response> {
    execute(deps.as_mut(), mock_env(), message_info(who, &[]), msg)
}

fn publish_full(deps: &mut cosmwasm_std::OwnedDeps<cosmwasm_std::testing::MockStorage, cosmwasm_std::testing::MockApi, cosmwasm_std::testing::MockQuerier>, gov: &cosmwasm_std::Addr, v: u32, data: &[u8], chunk: usize, activate: bool, upload: usize) -> usize {
    let (m, chunks) = manifest(v, data, chunk, activate);
    ex(deps, gov, ExecuteMsg::PublishManifest { manifest: m }).unwrap();
    for (i, c) in chunks.iter().enumerate().take(upload) {
        ex(deps, gov, ExecuteMsg::UploadChunk { version: v, file: 0, index: i as u32, data: Binary::from(c.clone()) }).unwrap();
    }
    chunks.len()
}

fn ps(deps: &cosmwasm_std::OwnedDeps<cosmwasm_std::testing::MockStorage, cosmwasm_std::testing::MockApi, cosmwasm_std::testing::MockQuerier>, v: u32) -> PruneStatusResponse {
    from_json(query(deps.as_ref(), mock_env(), QueryMsg::PruneStatus { version: v }).unwrap()).unwrap()
}

fn attr(r: &Response, k: &str) -> String {
    r.attributes.iter().find(|a| a.key == k).unwrap().value.clone()
}

fn version_keys(storage: &dyn Storage, v: u32) -> usize {
    let mut n = (0..1000u32).filter(|i| storage.get(&chunk_key(v, 0, *i)).is_some()).count();
    n += MANIFESTS.has(storage, v) as usize + PROGRESS.has(storage, v) as usize + PRUNING.has(storage, v) as usize;
    n
}

#[test]
fn prune_in_batches_resumes_and_removes_every_key() {
    let (mut deps, gov, anyone) = setup(4);
    let n1 = publish_full(&mut deps, &gov, 1, b"version-one-has-28-bytes!!!!", 4, true, 7);
    assert_eq!(n1, 7);
    let n2 = publish_full(&mut deps, &gov, 2, b"v2 page bytes", 4, false, 4);
    ex(&mut deps, &gov, ExecuteMsg::SetCurrent { version: 2 }).unwrap();
    let total_before = deps.storage.range(None, None, Order::Ascending).count();
    assert_eq!(version_keys(&deps.storage, 1), 7 + 2);
    assert_eq!(ps(&deps, 1).state, "live");
    // owner only
    assert!(ex(&mut deps, &anyone, ExecuteMsg::PruneVersion { version: 1, limit: Some(3) }).is_err());
    // batch 1
    let r = ex(&mut deps, &gov, ExecuteMsg::PruneVersion { version: 1, limit: Some(3) }).unwrap();
    assert_eq!((attr(&r, "visited"), attr(&r, "removed"), attr(&r, "done")), ("3".into(), "3".into(), "false".into()));
    let s = ps(&deps, 1);
    assert_eq!((s.state.as_str(), s.next_file, s.next_index, s.removed, s.remaining), ("pruning", 0, 3, 3, 4));
    assert!(query(deps.as_ref(), mock_env(), QueryMsg::Chunk { version: 1, file: 0, index: 2 }).is_err());
    assert!(query(deps.as_ref(), mock_env(), QueryMsg::Chunk { version: 1, file: 0, index: 3 }).is_ok());
    // a version being pruned accepts no uploads and cannot become current
    let (_, c1) = manifest(1, b"version-one-has-28-bytes!!!!", 4, true);
    let e = ex(&mut deps, &anyone, ExecuteMsg::UploadChunk { version: 1, file: 0, index: 0, data: Binary::from(c1[0].clone()) }).unwrap_err();
    assert!(e.to_string().contains("being pruned"), "{e}");
    assert!(ex(&mut deps, &gov, ExecuteMsg::SetCurrent { version: 1 }).unwrap_err().to_string().contains("being pruned"));
    // batches 2 and 3 resume from the cursor
    let r = ex(&mut deps, &gov, ExecuteMsg::PruneVersion { version: 1, limit: Some(3) }).unwrap();
    assert_eq!(attr(&r, "done"), "false");
    let r = ex(&mut deps, &gov, ExecuteMsg::PruneVersion { version: 1, limit: Some(3) }).unwrap();
    assert_eq!((attr(&r, "visited"), attr(&r, "removed"), attr(&r, "done")), ("1".into(), "7".into(), "true".into()));
    let s = ps(&deps, 1);
    assert_eq!((s.state.as_str(), s.removed, s.remaining), ("pruned", 7, 0));
    // every v1 key is gone (7 chunks + manifest + progress; cursor removed), tombstone added
    assert_eq!(version_keys(&deps.storage, 1), 0);
    assert_eq!(deps.storage.range(None, None, Order::Ascending).count(), total_before - 9 + 1);
    assert!(query(deps.as_ref(), mock_env(), QueryMsg::Manifest { version: Some(1) }).is_err());
    assert!(query(deps.as_ref(), mock_env(), QueryMsg::Status { version: 1 }).is_err());
    assert!(ex(&mut deps, &gov, ExecuteMsg::PruneVersion { version: 1, limit: None }).unwrap_err().to_string().contains("already pruned"));
    // v2 still serves; versions list only has v2; version numbers are not reused
    let m: Manifest = from_json(query(deps.as_ref(), mock_env(), QueryMsg::Manifest { version: None }).unwrap()).unwrap();
    assert_eq!(m.version, 2);
    assert_eq!(version_keys(&deps.storage, 2), n2 + 2);
    let v: VersionsResponse = from_json(query(deps.as_ref(), mock_env(), QueryMsg::Versions { start_after: None, limit: None }).unwrap()).unwrap();
    assert_eq!(v.versions.iter().map(|s| s.version).collect::<Vec<_>>(), vec![2]);
    let (m1, _) = manifest(1, b"x", 4, false);
    assert!(ex(&mut deps, &gov, ExecuteMsg::PublishManifest { manifest: m1 }).is_err());
    assert_eq!(ps(&deps, 9).state, "unknown");
}

#[test]
fn never_prunes_current() {
    let (mut deps, gov, _) = setup(4);
    publish_full(&mut deps, &gov, 1, b"abcdefgh", 4, true, 2);
    let e = ex(&mut deps, &gov, ExecuteMsg::PruneVersion { version: 1, limit: Some(250) }).unwrap_err();
    assert!(e.to_string().contains("current"), "{e}");
    assert_eq!(version_keys(&deps.storage, 1), 4);
    assert!(ex(&mut deps, &gov, ExecuteMsg::PruneVersion { version: 7, limit: None }).unwrap_err().to_string().contains("unknown version"));
}

#[test]
fn pending_activation_target_refused_until_superseded() {
    let (mut deps, gov, _) = setup(4);
    publish_full(&mut deps, &gov, 1, b"abcdefgh", 4, true, 2); // current = 1
    publish_full(&mut deps, &gov, 2, b"0123456789ab", 4, true, 1); // activate, 1/3 uploaded
    let e = ex(&mut deps, &gov, ExecuteMsg::PruneVersion { version: 2, limit: None }).unwrap_err();
    assert!(e.to_string().contains("pending activation target"), "{e}");
    publish_full(&mut deps, &gov, 3, b"newer", 4, false, 2);
    ex(&mut deps, &gov, ExecuteMsg::SetCurrent { version: 3 }).unwrap();
    // superseded: v2 can no longer activate, so it may be pruned (incomplete: removes only stored keys)
    let r = ex(&mut deps, &gov, ExecuteMsg::PruneVersion { version: 2, limit: None }).unwrap();
    assert_eq!((attr(&r, "visited"), attr(&r, "removed"), attr(&r, "done")), ("3".into(), "1".into(), "true".into()));
    assert_eq!(version_keys(&deps.storage, 2), 0);
    // the old current (1) is now prunable too
    ex(&mut deps, &gov, ExecuteMsg::PruneVersion { version: 1, limit: None }).unwrap();
    let cfg: ConfigResponse = from_json(query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap()).unwrap();
    assert_eq!(cfg.current, Some(3));
}

#[test]
fn auto_activation_never_moves_current_backwards() {
    let (mut deps, gov, _) = setup(4);
    let (m1, c1) = manifest(1, b"abcdefgh", 4, true);
    ex(&mut deps, &gov, ExecuteMsg::PublishManifest { manifest: m1 }).unwrap();
    publish_full(&mut deps, &gov, 2, b"v2", 4, false, 1);
    ex(&mut deps, &gov, ExecuteMsg::SetCurrent { version: 2 }).unwrap();
    for (i, c) in c1.iter().enumerate() {
        ex(&mut deps, &gov, ExecuteMsg::UploadChunk { version: 1, file: 0, index: i as u32, data: Binary::from(c.clone()) }).unwrap();
    }
    let cfg: ConfigResponse = from_json(query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap()).unwrap();
    assert_eq!(cfg.current, Some(2));
}

#[test]
fn prune_limit_is_clamped() {
    let (mut deps, gov, _) = setup(1);
    let data: Vec<u8> = (0..300u32).map(|i| i as u8).collect();
    publish_full(&mut deps, &gov, 1, &data, 1, false, 300);
    publish_full(&mut deps, &gov, 2, b"x", 1, true, 1);
    let r = ex(&mut deps, &gov, ExecuteMsg::PruneVersion { version: 1, limit: Some(100_000) }).unwrap();
    assert_eq!(attr(&r, "visited"), MAX_PRUNE_LIMIT.to_string());
    let r = ex(&mut deps, &gov, ExecuteMsg::PruneVersion { version: 1, limit: Some(0) }).unwrap();
    assert_eq!(attr(&r, "visited"), "1");
    let r = ex(&mut deps, &gov, ExecuteMsg::PruneVersion { version: 1, limit: None }).unwrap();
    assert_eq!((attr(&r, "visited"), attr(&r, "done")), ("49".into(), "true".into()));
    assert_eq!(ps(&deps, 1).removed, 300);
}

#[test]
fn migrate_from_v1_state_and_version_guards() {
    let (mut deps, gov, _) = setup(4);
    publish_full(&mut deps, &gov, 1, b"abcdefgh", 4, true, 2);
    // simulate code 106 (v1): no cw2 entry
    deps.storage.remove(b"contract_info");
    assert!(cw2::CONTRACT.may_load(&deps.storage).unwrap().is_none());
    let mi = MigrateInfo { sender: gov.clone(), old_migrate_version: None };
    let r = migrate(deps.as_mut(), mock_env(), MigrateMsg {}, mi.clone()).unwrap();
    assert_eq!(attr(&r, "from"), "v1-no-cw2");
    let v = cw2::get_contract_version(&deps.storage).unwrap();
    assert_eq!((v.contract.as_str(), v.version.as_str()), (CONTRACT_NAME, CONTRACT_VERSION));
    // state preserved and still served
    let m: Manifest = from_json(query(deps.as_ref(), mock_env(), QueryMsg::Manifest { version: None }).unwrap()).unwrap();
    assert_eq!(m.version, 1);
    // same version re-migrate ok; downgrade / foreign contract refused
    migrate(deps.as_mut(), mock_env(), MigrateMsg {}, mi.clone()).unwrap();
    cw2::set_contract_version(&mut deps.storage, CONTRACT_NAME, "99.0.0").unwrap();
    assert!(migrate(deps.as_mut(), mock_env(), MigrateMsg {}, mi.clone()).unwrap_err().to_string().contains("cannot migrate from version"));
    cw2::set_contract_version(&mut deps.storage, "crates.io:cw20-base", "0.0.1").unwrap();
    assert!(migrate(deps.as_mut(), mock_env(), MigrateMsg {}, mi.clone()).unwrap_err().to_string().contains("cannot migrate from contract"));
    // an unrelated empty state is refused
    let mut empty = mock_dependencies();
    assert!(migrate(empty.as_mut(), mock_env(), MigrateMsg {}, mi).unwrap_err().to_string().contains("not a cw-headstash-frontend v1 state"));
}

fn kl_pub(deps: &mut cosmwasm_std::OwnedDeps<cosmwasm_std::testing::MockStorage, cosmwasm_std::testing::MockApi, cosmwasm_std::testing::MockQuerier>, gov: &cosmwasm_std::Addr, v: u32, activate: bool) -> usize {
    let data: Vec<u8> = (0..40u32).map(|i| (i * 7 + v) as u8).collect();
    let (m, chunks) = manifest(v, &data, 8, activate);
    execute(deps.as_mut(), mock_env(), message_info(gov, &[]), ExecuteMsg::PublishManifest { manifest: m }).unwrap();
    for (i, c) in chunks.iter().enumerate() {
        execute(deps.as_mut(), mock_env(), message_info(gov, &[]), ExecuteMsg::UploadChunk { version: v, file: 0, index: i as u32, data: Binary::from(c.clone()) }).unwrap();
    }
    chunks.len()
}

fn kl(deps: &cosmwasm_std::OwnedDeps<cosmwasm_std::testing::MockStorage, cosmwasm_std::testing::MockApi, cosmwasm_std::testing::MockQuerier>) -> KeepLastResponse {
    from_json(query(deps.as_ref(), mock_env(), QueryMsg::KeepLast {}).unwrap()).unwrap()
}

#[test]
fn keep_last_prunes_oldest_beyond_n_permissionlessly_never_current_or_staged() {
    let (mut deps, gov, anyone) = setup(8);
    for v in 1..=5 { kl_pub(&mut deps, &gov, v, false); }
    // v6 staged above current, incomplete, activate=true: must never be a candidate.
    let (m6, _) = manifest(6, &[1u8; 24], 8, true);
    execute(deps.as_mut(), mock_env(), message_info(&gov, &[]), ExecuteMsg::PublishManifest { manifest: m6 }).unwrap();
    execute(deps.as_mut(), mock_env(), message_info(&gov, &[]), ExecuteMsg::SetCurrent { version: 4 }).unwrap();
    // policy disabled -> nothing
    assert!(execute(deps.as_mut(), mock_env(), message_info(&anyone, &[]), ExecuteMsg::PruneOld { limit: None }).is_err());
    // only owner sets policy, bounded
    assert!(execute(deps.as_mut(), mock_env(), message_info(&anyone, &[]), ExecuteMsg::SetKeepLast { keep_last: Some(2) }).is_err());
    assert!(execute(deps.as_mut(), mock_env(), message_info(&gov, &[]), ExecuteMsg::SetKeepLast { keep_last: Some(0) }).is_err());
    assert!(execute(deps.as_mut(), mock_env(), message_info(&gov, &[]), ExecuteMsg::SetKeepLast { keep_last: Some(MAX_KEEP_LAST + 1) }).is_err());
    execute(deps.as_mut(), mock_env(), message_info(&gov, &[]), ExecuteMsg::SetKeepLast { keep_last: Some(2) }).unwrap();
    // keep current(4) + one older (3): 1 and 2 obsolete, oldest first, batched by `limit`.
    assert_eq!(kl(&deps).next_target, Some(1));
    let mut calls = 0;
    while kl(&deps).next_target.is_some() {
        execute(deps.as_mut(), mock_env(), message_info(&anyone, &[]), ExecuteMsg::PruneOld { limit: Some(2) }).unwrap();
        calls += 1;
        assert!(calls < 20);
    }
    assert_eq!(calls, 3 + 3, "5 chunks each, 2 per call");
    for v in [1u32, 2] {
        let s: PruneStatusResponse = from_json(query(deps.as_ref(), mock_env(), QueryMsg::PruneStatus { version: v }).unwrap()).unwrap();
        assert_eq!(s.state, "pruned");
    }
    for v in [3u32, 4, 5, 6] {
        let s: PruneStatusResponse = from_json(query(deps.as_ref(), mock_env(), QueryMsg::PruneStatus { version: v }).unwrap()).unwrap();
        assert_ne!(s.state, "pruned", "v{v}");
    }
    assert!(execute(deps.as_mut(), mock_env(), message_info(&anyone, &[]), ExecuteMsg::PruneOld { limit: None }).is_err());
    // keep_last = 1: only current survives below-or-equal; v3 goes, v5/v6 (above current) stay.
    execute(deps.as_mut(), mock_env(), message_info(&gov, &[]), ExecuteMsg::SetKeepLast { keep_last: Some(1) }).unwrap();
    assert_eq!(kl(&deps).next_target, Some(3));
    execute(deps.as_mut(), mock_env(), message_info(&anyone, &[]), ExecuteMsg::PruneOld { limit: Some(250) }).unwrap();
    assert_eq!(kl(&deps).next_target, None);
    let cfg: ConfigResponse = from_json(query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap()).unwrap();
    assert_eq!(cfg.current, Some(4));
    assert_eq!(cfg.keep_last, Some(1));
    let b: Binary = from_json(query(deps.as_ref(), mock_env(), QueryMsg::Chunk { version: 4, file: 0, index: 0 }).unwrap()).unwrap();
    assert!(!b.is_empty());
    // disabling the policy stops PruneOld
    execute(deps.as_mut(), mock_env(), message_info(&gov, &[]), ExecuteMsg::SetKeepLast { keep_last: None }).unwrap();
    assert_eq!(kl(&deps).keep_last, None);
}

#[test]
fn keep_last_resumes_owner_started_prune_and_no_current_means_no_target() {
    let (mut deps, gov, anyone) = setup(8);
    kl_pub(&mut deps, &gov, 1, false);
    execute(deps.as_mut(), mock_env(), message_info(&gov, &[]), ExecuteMsg::SetKeepLast { keep_last: Some(3) }).unwrap();
    assert_eq!(kl(&deps).next_target, None, "no current yet");
    kl_pub(&mut deps, &gov, 2, false);
    execute(deps.as_mut(), mock_env(), message_info(&gov, &[]), ExecuteMsg::SetCurrent { version: 2 }).unwrap();
    assert_eq!(kl(&deps).next_target, None, "within keep_last");
    // owner starts pruning v1 explicitly; PruneOld finishes it for them.
    execute(deps.as_mut(), mock_env(), message_info(&gov, &[]), ExecuteMsg::PruneVersion { version: 1, limit: Some(1) }).unwrap();
    assert_eq!(kl(&deps).next_target, Some(1));
    execute(deps.as_mut(), mock_env(), message_info(&anyone, &[]), ExecuteMsg::PruneOld { limit: None }).unwrap();
    let s: PruneStatusResponse = from_json(query(deps.as_ref(), mock_env(), QueryMsg::PruneStatus { version: 1 }).unwrap()).unwrap();
    assert_eq!(s.state, "pruned");
}
