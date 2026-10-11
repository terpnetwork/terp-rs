//! Primitive DAO init and use on a real terpd, through cw-orch `Daemon`.
//!
//! ict-rs starts the container. `daemon_builder_from_chain` connects the
//! daemon to the host-mapped gRPC port. The funded account is a fresh
//! mnemonic placed in genesis; it is not printed.
//!
//! Nested subDAOs use the reply voting modules in `dao-contracts/artifacts`.
//! `salt` is `None`, so each child is `WasmMsg::Instantiate` and the module
//! reads the instantiate event. This pins `registry.terp.network/terp-core:v6.4.0`.
//!
//! ```text
//! ICT_DAO_ACTIONS=1 cargo test -p dao-foundation-suite --test dao_actions_chain \
//!   --offline --locked -- --ignored --nocapture
//! ```
//!
//! `ICT_MOCK=1` is rejected. This does not store code on morocco-1.

use cosmwasm_std::Addr;
use cw_orch::prelude::*;
use ict_rs::chain::GenesisStyle;
use ict_rs::prelude::generate_mnemonic;
use ict_rs::testing::{TestChain, TestChainConfig, TestEnv};
use ict_rs::tx::WalletAmount;
use ict_rs::wallet::KeyWallet;
use ict_rs_cw_orch::daemon_builder_from_chain;
use serde_json::{json, Value};

#[tokio::test]
#[ignore = "docker: ICT_DAO_ACTIONS=1 on registry.terp.network/terp-core:v6.4.0"]
async fn dao_modules_initialize_on_terp() {
    if std::env::var("ICT_DAO_ACTIONS").ok().as_deref() != Some("1") {
        panic!("set ICT_DAO_ACTIONS=1 before running this ignored test");
    }
    if std::env::var("ICT_MOCK").ok().as_deref() == Some("1") {
        panic!("ICT_MOCK does not exercise terpd");
    }
    // The recut that restored wasm reply events. An ambient older tag must not win.
    unsafe {
        std::env::set_var("TERP_IMAGE_REPO", "registry.terp.network/terp-core");
        std::env::set_var("TERP_IMAGE_VERSION", "v6.4.0");
        std::env::remove_var("ARTIFACTS_DIR");
    }
    assert_reply_voting_artifacts();

    let mnemonic = generate_mnemonic();
    let member = KeyWallet::from_mnemonic("member", &mnemonic, "terp", 118)
        .expect("member wallet");
    let vestee = funded_addr("vestee");
    let counterparty = funded_addr("swap-two");

    let mut cfg = TestEnv::terp_config();
    cfg.chain_id = "dao-actions-1".into();
    cfg.gas_prices = "0uterp".into();
    cfg.gas_adjustment = 2.5;
    cfg.block_time = "500ms".into();
    cfg.genesis_style = GenesisStyle::Legacy;
    cfg.env
        .push(("MAX_WASM_SIZE".into(), (800 * 1024 * 2).to_string()));
    cfg.config_file_overrides.insert(
        "config/config.toml".into(),
        json!({
            "rpc": { "max_body_bytes": 104_857_600 },
            "mempool": { "max_tx_bytes": 104_857_600 }
        }),
    );
    cfg.modify_genesis = Some(Box::new(|_cfg, raw| {
        let mut genesis: Value = serde_json::from_slice(&raw)
            .map_err(|err| ict_rs::error::IctError::Config(format!("genesis json: {err}")))?;
        if let Some(block) = genesis.pointer_mut("/consensus_params/block") {
            block["max_gas"] = json!("-1");
            block["max_bytes"] = json!("104857600");
        }
        if let Some(params) = genesis.pointer_mut("/app_state/wasm/params") {
            params["code_upload_access"] = json!({"permission": "Everybody", "addresses": []});
            params["instantiate_default_permission"] = json!("Everybody");
        }
        if let Some(params) = genesis.pointer_mut("/app_state/tokenfactory/params") {
            params["denom_creation_fee"] = json!([]);
        }
        serde_json::to_vec(&genesis)
            .map_err(|err| ict_rs::error::IctError::Config(format!("genesis encode: {err}")))
    }));

    let mut chain = TestChain::setup(
        "dao-actions",
        TestChainConfig {
            chain_config: cfg,
            num_validators: 1,
            num_full_nodes: 0,
            genesis_wallets: vec![WalletAmount {
                address: member.bech32_address.clone(),
                denom: "uterp".into(),
                amount: 50_000_000_000,
            }],
        },
    )
    .await
    .expect("local terp chain");

    println!("member {}", member.bech32_address);
    // DaemonBuilder::build and later block waits use cw-orch's private
    // runtime. Calling them on the tokio test thread panics.
    let exercise = std::thread::scope(|scope| {
        let handle = scope.spawn(|| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut builder = daemon_builder_from_chain(&chain.chain, Some(&mnemonic))
                    .expect("daemon builder");
                builder.load_network(false);
                let daemon = builder.build().expect("daemon");
                assert_eq!(daemon.sender_addr().as_str(), member.bech32_address);
                dao_foundation_suite::actions::run_primitive_actions(
                    &daemon,
                    &Addr::unchecked(&vestee),
                    &Addr::unchecked(&counterparty),
                    true,
                );
            }))
        });
        handle.join().expect("daemon thread")
    });
    if exercise.is_err() {
        if let Some(node) = chain.chain.validators().first() {
            let log = node.read_chain_log(120).await;
            println!("chain.log tail:\n{log}");
        }
        chain.dump_logs().await;
    }
    let cleanup = chain.cleanup().await;
    match exercise {
        Ok(()) => cleanup.expect("chain cleanup"),
        Err(payload) => {
            let _ = cleanup;
            std::panic::resume_unwind(payload);
        }
    }
}

/// The default artifact dir is the reply build. Each voting module reads the
/// child address from the instantiate event. cw4, cw721-staked, and
/// cw721-roles use `parse_event_from_reply_submsg` (`key not found in event`).
/// cw20-staked and cw20-balance scan that same event inline
/// (`contract_address not found in reply`). Neither build predicts an
/// instantiate2 address.
fn assert_reply_voting_artifacts() {
    let artifacts = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../dao-contracts/artifacts");
    for name in [
        "dao_voting_cw4.wasm",
        "dao_voting_cw20_staked.wasm",
        "dao_voting_cw721_staked.wasm",
        "dao_voting_cw721_roles.wasm",
        "dao_voting_cw20_balance.wasm",
    ] {
        let path = artifacts.join(name);
        let bytes = std::fs::read(&path).unwrap_or_else(|err| panic!("read {path:?}: {err}"));
        let text = String::from_utf8_lossy(&bytes);
        let reads_instantiate_event = text.contains("key not found in event")
            || text.contains("contract_address not found in reply");
        assert!(
            reads_instantiate_event,
            "{name} does not parse the instantiate event"
        );
        assert!(
            !text.contains("predict_instantiate2"),
            "{name} still contains the instantiate2 predictor"
        );
        assert!(
            !text.contains("dao-instantiate2/1"),
            "{name} still contains the instantiate2 salt label"
        );
    }
}

fn funded_addr(name: &str) -> String {
    let mnemonic = generate_mnemonic();
    KeyWallet::from_mnemonic(name, &mnemonic, "terp", 118)
        .unwrap_or_else(|err| panic!("{name} address: {err}"))
        .bech32_address
}
