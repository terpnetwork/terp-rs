//! Local **production-shaped** HashMerchant workflow: ict-rs `local-zk` chain
//! + real `hash-market-server` process + cw-orch Daemon.
//!
//! Not in-process oneshot. Gate: `ICT_HASHMERCHANT=1`.
//!
//! ```sh
//! cargo build --manifest-path tools/hash-market/Cargo.toml \
//!   --bin hash-market-server --features server,blossom
//! ICT_HASHMERCHANT=1 cargo test -p terp-scripts --test hashmerchant_live \
//!   --offline -- --ignored --test-threads=1 --nocapture
//! ```

use anyhow::{Context, Result};
use axum::http::StatusCode;
use cw_orch::daemon::DaemonBuilder;
use cw_orch::environment::{ChainInfoOwned, ChainKind, NetworkInfoOwned};
use cw_orch::prelude::*;
use ict_rs::prelude::*;
use ict_rs::testing::env::TestEnv;
use terp_scripts::suite::sidecar::TerpSidecar;
use terp_scripts::suite::{HashMerchantClient, HashMerchantSuite, SidecarRegistry};

const ICT_GATE: &str = "ICT_HASHMERCHANT";
const BIND: &str = "127.0.0.1:19190";

/// Same wallet cw-orch and ict recover. Never MAIN_MNEMONIC.
const LAB_MNEMONIC: &str = "chapter wrist alcohol shine angry noise mercy simple rebel recycle \
    vehicle wrap morning giraffe lazy outdoor noise blood ginger sort reunion boss crowd dutch";

fn require_gate() {
    assert_eq!(
        std::env::var(ICT_GATE).ok().as_deref(),
        Some("1"),
        "{ICT_GATE}=1 required (docker local-zk + hash-market-server)"
    );
}

fn line(ok: bool, name: &str, detail: &str) {
    println!("{} {name} {detail}", if ok { "PASS" } else { "FAIL" });
}

fn chain_info(chain: &CosmosChain) -> ChainInfoOwned {
    let cfg = chain.config();
    let gas_price: f64 = cfg
        .gas_prices
        .trim_end_matches(|c: char| c.is_alphabetic())
        .parse()
        .unwrap_or(0.025);
    ChainInfoOwned {
        chain_id: cfg.chain_id.clone(),
        gas_denom: cfg.denom.clone(),
        gas_price,
        grpc_urls: vec![chain.host_grpc_address()],
        lcd_url: None,
        fcd_url: None,
        network_info: NetworkInfoOwned {
            chain_name: cfg.name.clone(),
            pub_address_prefix: cfg.bech32_prefix.clone(),
            coin_type: cfg.coin_type,
        },
        kind: ChainKind::Local,
    }
}

fn daemon_bank_query(info: ChainInfoOwned, addr: String) -> Result<String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    // cw-orch Daemon::build panics if called inside a tokio runtime.
    let daemon = DaemonBuilder::new(info)
        .is_test(true)
        .mnemonic(LAB_MNEMONIC)
        .build()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let bank: cw_orch::daemon::queriers::Bank = daemon.querier();
    let coins = bank
        .balance(&Addr::unchecked(addr), None)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(format!("{coins:?}"))
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "ICT_HASHMERCHANT=1 — docker local-zk + hash-market-server binary"]
async fn hashmerchant_live_ict_orch_server() -> Result<()> {
    require_gate();

    let runtime = IctRuntime::Docker(DockerConfig::default())
        .into_backend()
        .await
        .context("docker backend")?;
    let mut cfg = TestEnv::terp_config();
    cfg.chain_id = "ict-hm-live".into();
    let mut chain = CosmosChain::new(cfg, 1, 0, runtime);
    chain
        .initialize(&TestContext {
            test_name: "hashmerchant-live".into(),
            network_id: String::new(),
        })
        .await
        .context("initialize")?;
    chain.start(&[]).await.context("start chain")?;

    let result = run_workflow(&chain).await;
    let _ = chain.stop().await;
    result
}

async fn run_workflow(chain: &CosmosChain) -> Result<()> {
    chain
        .build_wallet("lab", LAB_MNEMONIC)
        .await
        .context("recover lab wallet")?;
    let lab = chain.key_address("lab").await.context("lab addr")?;
    chain
        .send_funds(
            "validator",
            &WalletAmount {
                address: lab.clone(),
                denom: "uterp".into(),
                amount: 50_000_000_000,
            },
        )
        .await
        .context("fund lab")?;
    wait_for_blocks(chain, 2).await.context("blocks")?;
    line(true, "chain", &format!("funded {lab}"));

    let mut hm = HashMerchantSuite::mint_host_onboard("ict-hm-live", BIND, &lab, &lab);
    let registry = SidecarRegistry::new();
    hm.start(&registry).await.context("start hash-market-server")?;
    let client = hm.http_client().context("http client")?;
    line(true, "server", &hm.server_http_url().unwrap_or_default());

    let health = client.health().await.context("GET /health")?;
    anyhow::ensure!(health["status"] == "ok", "health {health}");
    anyhow::ensure!(health["onboard"]["prefix"] == "/onboard", "onboard nest {health}");
    line(true, "health", "merchant + onboard prefix");

    let ost = client.onboard_status().await.context("GET /onboard/status")?;
    anyhow::ensure!(ost["ready"].as_bool() == Some(true), "onboard ready {ost}");
    anyhow::ensure!(ost["chain_id"] == "ict-hm-live", "{ost}");
    line(true, "onboard/status", &ost["dao"].to_string());

    let (st, grant) = client.request_register_grant(&lab).await?;
    anyhow::ensure!(st == StatusCode::OK, "register-grant {st} {grant}");
    let gs = grant.to_string();
    anyhow::ensure!(
        gs.contains("AllowedMsgAllowance") || gs.contains("MsgGrantAllowance"),
        "grant body {grant}"
    );
    line(true, "register-grant", "AllowedMsgAllowance json");

    let (mst, mint) = client.request_mint_grant(&lab).await?;
    anyhow::ensure!(mst == StatusCode::OK, "mint-grant {mst} {mint}");
    anyhow::ensure!(mint["purpose"] == "mint", "{mint}");
    line(true, "mint-grant", "wasm execute allowance json");

    let (st, _) = client.request_register_grant("cosmos1nope").await?;
    anyhow::ensure!(st == StatusCode::BAD_REQUEST, "insanity grant {st}");
    let st = client.probe("GET", "/status").await?;
    anyhow::ensure!(st == StatusCode::NOT_FOUND, "root /status must 404, got {st}");
    line(true, "insanity", "bad addr + no root /status");

    let payload = b"live-hashmerchant-blob";
    let up = client.upload_blob(payload).await.context("POST /blobs")?;
    let sha = up["sha256"].as_str().context("sha256")?;
    let (bst, got) = client.get_blob(sha).await?;
    anyhow::ensure!(bst.is_success() && got == payload, "blob roundtrip");
    line(true, "blobs", sha);

    let tree = serde_json::json!({
        "merkle_root": "cd".repeat(32),
        "accounts": {
            "terp1live00000000000000000000000000000": {
                "tier": 1, "allocation": 1, "proof_hashes": []
            }
        }
    });
    let tst = client.put_tree("live-a", tree).await?;
    anyhow::ensure!(tst.is_success() || tst == StatusCode::CREATED, "tree {tst}");
    line(true, "trees", "live-a");
    let (mp, proof) = client
        .member_proof("live-a", "terp1live00000000000000000000000000000")
        .await?;
    anyhow::ensure!(mp.is_success() && proof["eligible"] == true, "member {mp} {proof}");
    line(true, "member-proof", "eligible");

    let mut tasks = Vec::new();
    for i in 0..16 {
        let c = client.clone();
        tasks.push(tokio::spawn(async move {
            match i % 3 {
                0 => c.health().await.map(|_| ()),
                1 => {
                    let a = format!("terp1live{i:024}");
                    c.request_register_grant(&a).await.map(|(s, _)| {
                        assert_eq!(s, StatusCode::OK);
                    })
                }
                _ => c
                    .upload_blob(format!("live-{i}").as_bytes())
                    .await
                    .map(|_| ()),
            }
        }));
    }
    for t in tasks {
        t.await.context("join")?.context("concurrent live")?;
    }
    line(true, "concurrent", "16 HTTP tasks on listening server");

    chain.create_key("grantee").await.context("grantee key")?;
    let grantee = chain
        .primary_node()?
        .get_key_address("grantee")
        .await
        .context("grantee addr")?;
    chain
        .send_funds(
            "validator",
            &WalletAmount {
                address: grantee.clone(),
                denom: "uterp".into(),
                amount: 1_000_000,
            },
        )
        .await
        .ok();
    let grant_tx = chain
        .chain_exec_tx(&[
            "tx",
            "feegrant",
            "grant",
            &lab,
            &grantee,
            "--spend-limit",
            "25000uterp",
            "--from",
            "lab",
        ])
        .await
        .context("feegrant tx")?;
    anyhow::ensure!(grant_tx.exit_code == 0, "feegrant {}", grant_tx.stderr_str());
    let txj: serde_json::Value =
        serde_json::from_str(grant_tx.stdout_str().trim()).unwrap_or(serde_json::Value::Null);
    let hash = txj["txhash"].as_str().unwrap_or("");
    anyhow::ensure!(!hash.is_empty(), "no feegrant txhash");
    wait_for_blocks(chain, 2).await.ok();
    let q = chain
        .chain_exec(&[
            "query",
            "feegrant",
            "grant",
            &lab,
            &grantee,
            "--output",
            "json",
        ])
        .await
        .context("feegrant query")?;
    let qs = q.stdout_str();
    anyhow::ensure!(
        qs.contains(&grantee) || qs.contains("allowance") || qs.contains("spend_limit"),
        "feegrant query empty: {} {}",
        qs,
        q.stderr_str()
    );
    line(true, "feegrant-on-chain", hash);

    let info = chain_info(chain);
    let grpc = info.grpc_urls.clone();
    let lab_q = lab.clone();
    let coins = tokio::task::spawn_blocking(move || daemon_bank_query(info, lab_q))
        .await
        .context("daemon thread")??;
    line(true, "cw-orch", &format!("gRPC {grpc:?} (bank secondary) {coins}"));

    hm.stop().await.ok();
    Ok(())
}
