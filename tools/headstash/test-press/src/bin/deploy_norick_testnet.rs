//! Deploy No-Rick circuit + contract to Terp public testnet (or local testnet origin).
//!
//! Usage (from monorepo, with artifacts present):
//! ```text
//! export MNEMONIC='…'   # funded on 120u-1
//! # optional: CHAIN=local  → TERP_TESTNET_LOCAL (127.0.0.1 gRPC)
//! # optional: BUILD_KEYS=1 → run Halo2 keygen into artifacts/
//! cargo run -p zk-test-press --bin deploy_norick_testnet --features interface,zk
//! ```
//!
//! Prints JSON with circuit/code ids and contract address for desk wiring
//! (`?contract=…` / localStorage zk-wasmvm-test).

use std::env;

use cw_orch::anyhow::{self, Context};
use cw_orch::daemon::networks::{TERP_TESTNET, TERP_TESTNET_LOCAL};
use cw_orch::prelude::*;
use zk_test_press::suites::no_rick::interface::NoRickSuite;

fn main() -> anyhow::Result<()> {
    env_logger::init();

    let mnemonic = env::var("MNEMONIC").context(
        "MNEMONIC env required (funded testnet wallet, never commit secrets)",
    )?;

    let chain_info = match env::var("CHAIN").as_deref() {
        Ok("local") | Ok("LOCAL") => TERP_TESTNET_LOCAL.clone(),
        _ => TERP_TESTNET.clone(),
    };

    eprintln!(
        "deploy_norick_testnet: chain_id={} grpc={:?}",
        chain_info.chain_id, chain_info.grpc_urls
    );

    let daemon = Daemon::builder(chain_info)
        .mnemonic(&mnemonic)
        .build()?;

    let suite = NoRickSuite::new(daemon.clone());

    if env::var("BUILD_KEYS").ok().as_deref() == Some("1") {
        eprintln!("BUILD_KEYS=1 → generating No-Rick keys…");
        suite.circuit.build_keys()?;
    }

    eprintln!("uploading circuit…");
    suite.circuit.upload_circuit()?;

    eprintln!("uploading + instantiating contract…");
    let contract = cw_norick::NoRickContractSuite::deploy_on(daemon, Default::default())?;

    let code_id = contract
        .code_id()
        .map_err(|e| anyhow::anyhow!("code_id: {e}"))?;
    let addr = contract.addr_str()?;

    let out = serde_json::json!({
        "demo": "norick",
        "contract_code_id": code_id,
        "contract_address": addr,
        "notes": [
            format!("Wire desk: https://permissionless.money/zk?demos=1&contract={addr}"),
            "Or localStorage.setItem('zk-wasmvm-test', contract_address)",
            "Circuit: upload_circuit via NoRickCircuitSuite (artifacts/norick_vk.bin)",
        ],
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
