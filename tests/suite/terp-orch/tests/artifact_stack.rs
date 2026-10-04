//! Upload and instantiate every wasm under `crates/terp-rs/artifacts/`.
//!
//! Host entry points (Mock). `cw-ics08-wasm-zap1` uses a JSON `{}` stub here
//! because that crate is still on crates.io cosmwasm-std 1.5:
//!
//! ```text
//! cargo test -p terp-orch --features stack --test artifact_stack
//! ```
//!
//! Live local chain. Upload is the wasmvm `compile_module` check, including zap1:
//!
//! ```text
//! TERP_IMAGE_REPO=terp-core TERP_IMAGE_VERSION=v630-arm64 \
//!   cargo test -p terp-orch --features stack,ict --test artifact_stack live -- --ignored --nocapture
//! ```

#![cfg(feature = "stack")]

use cw_orch::prelude::*;
use terp_orch::TerpArtifactStack;

const ARTIFACTS: usize = 20;

fn assert_bound<Chain: CwEnv>(stack: &TerpArtifactStack<Chain>) -> Vec<(&'static str, u64, String)> {
    let rows = stack.bound().expect("code ids and addresses");
    assert_eq!(rows.len(), ARTIFACTS, "{rows:?}");
    for (id, code_id, address) in &rows {
        assert!(*code_id > 0, "{id} has no code id");
        assert!(!address.is_empty(), "{id} has no address");
        println!("{id} code_id={code_id} address={address}");
    }
    rows
}

#[test]
fn mock_upload_and_instantiate_all() {
    let mock = Mock::new("admin");
    let admin = mock.sender_addr();
    let stack = TerpArtifactStack::deploy_on(mock, admin.into()).expect("mock deploy");
    assert_bound(&stack);
}

/// Public BIP39 test mnemonic. Funded at genesis. Not an operator key.
#[cfg(feature = "ict")]
const LAB_MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

#[cfg(feature = "ict")]
#[tokio::test]
#[ignore = "docker: TERP_IMAGE_REPO=terp-core TERP_IMAGE_VERSION=v630-arm64"]
async fn live_local_terp_upload_and_instantiate() {
    let mut cfg = ict_rs::chain::terp::terp_chain_config();
    // v6.3 terpd: `init --chain-id`, `genesis add-genesis-account`, `genesis collect-gentxs`.
    cfg.genesis_style = ict_rs::chain::GenesisStyle::Legacy;
    cfg.faucet = Some(ict_rs::chain::FaucetConfig {
        mnemonic: Some(LAB_MNEMONIC.to_string()),
        start_cmd: Vec::new(),
        coins: Some("1000000000000uterp".to_string()),
        ..ict_rs::chain::FaucetConfig::default()
    });
    let mut chain = ict_rs::testing::setup_chain("artifact_stack", cfg)
        .await
        .expect("local terp chain");

    // cw-orch's sync Daemon calls block_on. That panics on a tokio worker thread.
    let mut builder = terp_orch::ict::daemon_builder(&chain, Some(LAB_MNEMONIC))
        .expect("daemon builder");
    builder.load_network(false);
    let deployed = std::thread::spawn(move || {
        let daemon = builder.build().expect("cw-orch daemon");
        let admin = daemon.sender_addr();
        let stack = TerpArtifactStack::deploy_on(daemon, admin.into()).expect("live deploy");
        assert_bound(&stack)
    })
    .join();

    let cleanup = chain.cleanup().await;
    let rows = deployed.unwrap_or_else(|payload| std::panic::resume_unwind(payload));
    cleanup.expect("chain cleanup");
    assert_eq!(rows.len(), ARTIFACTS);
}
