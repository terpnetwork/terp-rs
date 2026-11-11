//! Unified privacy-products e2e on ict-rs (Docker): Terp + anvil + zakura.
//!
//! ```sh
//! # groot-wan; wasm artifacts must exist in crates/terp-rs/artifacts (cw_lc_registry.wasm)
//! ICT_TEST_MNEMONIC=… RUSTC_WRAPPER=kache \
//!   cargo test -p terp-orch --features privacy-live --test privacy_products_e2e -- --ignored --nocapture
//! ```
//! Ports publish on 127.0.0.1 only (`ICT_PUBLISH_HOST` is forced). Zakura is
//! spawned when a Linux `zakurad` resolves (`ZAKURAD_BIN`), else the run uses
//! synthetic node state and says so. The mnemonic is read from the env only.
#![cfg(feature = "privacy-live")]

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use cw_orch::daemon::{CosmosOptions, TxSender};
use cw_orch::prelude::*;
use ed25519_dalek::SigningKey;
use ict_rs::chain::cosmos::CosmosChain;
use ict_rs::chain::{Chain, TestContext};
use ict_rs::runtime::{DockerConfig, IctRuntime};
use ict_rs::testing::{TestEnv, TestEthChain};
use terp_orch::suites::privacy::{
    synthetic_regtest_state, OperatorId, Outcome, PrivacySuite, SuiteCtx,
};
use tokio::runtime::Runtime;

/// Blocking JSON-RPC to a 127.0.0.1 zakurad (curl; no extra deps).
fn zakura_rpc(rpc: &str, method: &str, params: &str) -> Result<serde_json::Value> {
    let body = format!(r#"{{"jsonrpc":"2.0","id":1,"method":"{method}","params":{params}}}"#);
    let out = std::process::Command::new("curl")
        .args([
            "-sf",
            "--max-time",
            "5",
            "-X",
            "POST",
            rpc,
            "-H",
            "Content-Type: application/json",
            "-d",
            &body,
        ])
        .output()?;
    anyhow::ensure!(out.status.success(), "zakura rpc {method} failed");
    Ok(serde_json::from_slice::<serde_json::Value>(&out.stdout)?["result"].clone())
}

fn zakura_state_source(rpc: String) -> terp_orch::suites::privacy::NodeStateSource {
    let mut t = 0u64;
    Box::new(move |_i| {
        t += 1;
        let r = zakura_rpc(&rpc, "getblockchaininfo", "[]")?;
        let best = r["bestblockhash"]
            .as_str()
            .ok_or_else(|| anyhow!("no bestblockhash"))?
            .to_string();
        let hdr = zakura_rpc(&rpc, "getblockheader", &format!("[\"{best}\"]"))?;
        Ok(cw_lc_registry::msg::NodeState {
            tip_height: r["blocks"].as_u64().unwrap_or(0),
            prev_hash: hdr["previousblockhash"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| "00".repeat(32)),
            tip_hash: best,
            chain_work: r["chainwork"].as_str().unwrap_or("00").to_string(),
            finalized_height: None,
            peer_count: 0,
            observed_at_unix: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() * 1000 + t,
        })
    })
}

#[test]
#[ignore = "Docker: spawns Terp + anvil (+ zakura) on 127.0.0.1"]
fn privacy_products_full_run() -> Result<()> {
    // SAFETY: single-threaded at this point; dev services must not bind 0.0.0.0.
    unsafe { std::env::set_var("ICT_PUBLISH_HOST", "127.0.0.1") };
    let mnemonic = std::env::var("ICT_TEST_MNEMONIC")
        .context("ICT_TEST_MNEMONIC must be set (never a source literal)")?;
    let rt = Runtime::new()?;
    let n = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    let suite_chains = PrivacySuite::<Daemon>::standard().required_chains();
    println!("required chains: {suite_chains:?}");

    let (mut terp, mut anvil) = rt.block_on(async {
        let runtime = IctRuntime::Docker(DockerConfig::default())
            .into_backend()
            .await?;
        let mut terp = CosmosChain::new(TestEnv::terp_config(), 1, 0, runtime);
        let ctx = TestContext {
            test_name: format!("privacy-products-{n}"),
            network_id: String::new(),
        };
        terp.initialize(&ctx).await?;
        terp.start(&[]).await?;
        let anvil = TestEthChain::setup(&format!("privacy-anvil-{n}")).await?;
        Ok::<_, anyhow::Error>((terp, anvil))
    })?;

    let result = (|| -> Result<()> {
        let mut builder = ict_rs_cw_orch::daemon_builder_from_chain(&terp, Some(&mnemonic))?;
        builder.handle(rt.handle());
        let daemon = builder.build()?;
        let mut operators = vec![];
        for i in 1..=3u32 {
            let d = daemon.clone().new_sender(
                CosmosOptions::default()
                    .mnemonic(mnemonic.clone())
                    .hd_index(i),
            );
            let addr = d.sender_addr();
            rt.block_on(
                daemon
                    .sender()
                    .bank_send(&addr, &[Coin::new(5_000_000u128, "uterp")]),
            )?;
            operators.push(OperatorId {
                sender: d.sender().clone(),
                addr,
                key: SigningKey::from_bytes(&[0x40 + i as u8; 32]),
            });
        }
        let zakura_rpc = std::env::var("ZAKURA_RPC_URL").ok();
        let node_state = match &zakura_rpc {
            Some(rpc) => zakura_state_source(rpc.clone()),
            None => {
                println!("zakura: ZAKURA_RPC_URL unset → synthetic node state");
                synthetic_regtest_state()
            }
        };
        let mut ctx = SuiteCtx {
            terp: daemon,
            operators,
            node_state,
            zakura_rpc,
            anvil_rpc: Some(anvil.chain.host_rpc_address()),
            notes: vec![],
        };
        let mut suite = PrivacySuite::<Daemon>::standard();
        let mut failed = 0;
        for r in suite.run(&mut ctx) {
            match r {
                Ok(r) => println!(
                    "[{:?}] {} → {:?} {:?}",
                    r.workstream, r.id, r.outcome, r.notes
                ),
                Err(e) => {
                    failed += 1;
                    println!("FAILED: {e:#}");
                }
            }
        }
        anyhow::ensure!(failed == 0, "{failed} product(s) failed");
        let _ = Outcome::Passed;
        Ok(())
    })();

    rt.block_on(async {
        let _ = anvil.cleanup().await;
        let _ = terp.stop().await;
    });
    result
}
