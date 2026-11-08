//! cw-orch Mock: real assets (env HSFE_ASSET_DIR) gzip -> chunk -> upload -> verify.
#![cfg(feature = "interface")]
use cw_headstash_frontend::interface::*;
use cw_headstash_frontend::msg::*;
use cw_orch::prelude::*;

#[test]
fn upload_and_verify_real_assets_mock() {
    let dir = std::env::var("HSFE_ASSET_DIR")
        .expect("HSFE_ASSET_DIR (dir with zk_headstash.js + zk_headstash_bg.wasm)");
    let mut files = vec![];
    for (p, ct) in [
        ("zk_headstash.js", "text/javascript"),
        ("zk_headstash_bg.wasm", "application/wasm"),
    ] {
        let raw = std::fs::read(format!("{dir}/{p}"))
            .unwrap_or_else(|e| panic!("read {dir}/{p}: {e} (set HSFE_ASSET_DIR)"));
        files.push(ReleaseFile {
            path: p.replace('_', "-"),
            content_type: ct.into(),
            raw,
            gzip: true,
        });
    }
    let raws: Vec<Vec<u8>> = files.iter().map(|f| f.raw.clone()).collect();
    let chain = Mock::new("gov");
    let fe = FrontendContract::new(chain.clone());
    fe.upload().unwrap();
    fe.instantiate(
        &InstantiateMsg {
            owner: None,
            max_chunk_bytes: Some(128 * 1024),
        },
        None,
        &[],
    )
    .unwrap();
    let r = build_release(
        1,
        "zk-headstash.js",
        files,
        128 * 1024,
        true,
        Some("test".into()),
    );
    let st = upload_release(&fe, &r, true).unwrap();
    assert!(st.complete);
    let got = fetch_verified(&fe, None).unwrap();
    assert_eq!(got.iter().map(|g| g.1.clone()).collect::<Vec<_>>(), raws);
    // resumable: re-running uploads nothing and still verifies
    upload_release(&fe, &r, false).unwrap();
}
