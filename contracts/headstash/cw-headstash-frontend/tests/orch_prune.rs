//! cw-orch Mock (multi-test): migrate + publish v2 + prune v1 in batches through the real entry points.
#![cfg(feature = "interface")]
use cosmwasm_std::Binary;
use cw_headstash_frontend::interface::*;
use cw_headstash_frontend::msg::*;
use cw_orch::prelude::*;

fn files(tag: &str, n: usize) -> Vec<ReleaseFile> {
    let raw: Vec<u8> = (0..n).map(|i| (i * 31 % 251) as u8).chain(tag.bytes()).collect();
    vec![ReleaseFile { path: "app.html".into(), content_type: "text/html".into(), raw, gzip: false }]
}

#[test]
fn migrate_publish_v2_prune_v1_mock() {
    let chain = Mock::new("gov");
    let fe = FrontendContract::new(chain.clone());
    fe.upload().unwrap();
    fe.instantiate(&InstantiateMsg { owner: None, max_chunk_bytes: Some(64) }, Some(&chain.sender_addr()), &[]).unwrap();
    let r1 = build_release(1, "app.html", files("v1", 64 * 9), 64, true, None);
    upload_release(&fe, &r1, true).unwrap();
    // migrate (same code id; cw2 written at instantiate, equal version is allowed)
    fe.migrate(&MigrateMsg {}, fe.code_id().unwrap()).unwrap();
    let r2 = build_release(2, "app.html", files("v2", 64 * 3), 64, false, None);
    upload_release(&fe, &r2, true).unwrap();
    assert!(fe.execute(&ExecuteMsg::PruneVersion { version: 1, limit: Some(4) }, &[]).is_err(), "v1 is current");
    fe.execute(&ExecuteMsg::SetCurrent { version: 2 }, &[]).unwrap();
    let mut calls = 0;
    loop {
        calls += 1;
        fe.execute(&ExecuteMsg::PruneVersion { version: 1, limit: Some(4) }, &[]).unwrap();
        let s: PruneStatusResponse = fe.query(&QueryMsg::PruneStatus { version: 1 }).unwrap();
        if s.state == "pruned" {
            assert_eq!(s.removed, r1.chunks.len() as u32);
            break;
        }
        assert_eq!(s.state, "pruning");
    }
    assert_eq!(calls, (r1.chunks.len() + 3) / 4);
    for (f, i, _) in &r1.chunks {
        let q: Result<Binary, _> = fe.query(&QueryMsg::Chunk { version: 1, file: *f, index: *i });
        assert!(q.is_err());
    }
    let got = fetch_verified(&fe, None).unwrap();
    assert_eq!(got[0].1, files("v2", 64 * 3).remove(0).raw);
}

#[test]
fn keep_last_prune_old_mock() {
    let chain = Mock::new("gov");
    let fe = FrontendContract::new(chain.clone());
    fe.upload().unwrap();
    fe.instantiate(&InstantiateMsg { owner: None, max_chunk_bytes: Some(64) }, None, &[]).unwrap();
    let mut n = vec![];
    for v in 1..=4u32 {
        let r = build_release(v, "app.html", files(&format!("v{v}"), 64 * 5), 64, true, None);
        n.push(r.chunks.len());
        upload_release(&fe, &r, true).unwrap();
    }
    let cfg: ConfigResponse = fe.query(&QueryMsg::Config {}).unwrap();
    assert_eq!(cfg.current, Some(4));
    fe.execute(&ExecuteMsg::SetKeepLast { keep_last: Some(2) }, &[]).unwrap();
    let anyone = chain.addr_make("anyone");
    let mut calls = 0;
    loop {
        let k: KeepLastResponse = fe.query(&QueryMsg::KeepLast {}).unwrap();
        if k.next_target.is_none() { break; }
        fe.call_as(&anyone).execute(&ExecuteMsg::PruneOld { limit: Some(3) }, &[]).unwrap();
        calls += 1;
    }
    assert_eq!(calls, (n[0] + 2) / 3 + (n[1] + 2) / 3);
    let vs: VersionsResponse = fe.query(&QueryMsg::Versions { start_after: None, limit: None }).unwrap();
    assert_eq!(vs.versions.iter().map(|s| s.version).collect::<Vec<_>>(), vec![3, 4]);
    let got = fetch_verified(&fe, None).unwrap();
    assert_eq!(got[0].1, files("v4", 64 * 5).remove(0).raw);
}
