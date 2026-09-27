// //! e2e — Full Terp Network suite integration test
// //!
// //! Spins up two chains + Hermes relayer via cw-orch Interchain pattern,
// //! deploys the full contract stack (SVG, DAO, shitstraps, headstash),
// //! starts all sidecars (minio-ipfs, indexer, merkle-server, hashmerchant),
// //! uploads website dist to MinIO, and keeps running until Ctrl+C.
// //!
// //! The cw-orch state file (~/.cw-orchestrator/state.json) auto-records
// //! deployed contract addresses, which are consumed by the frontends
// //! (terp-docs, dao-dao-ui, terp.network) via config-loader.js.
// //!
// //! # Flow
// //!
// //! 1. Spawn 2 Terp chains (terp-test-1, terp-test-2) + Hermes relayer
// //! 2. Deploy DAO dao suite (with dao-calendar voting module)
// //! 3. Get IBC denoms → init shitstraps with callbacks
// //! 4. Init SVG mint pipeline (cw-infuser, merkle whitelist, minter)
// //! 5. Start minio-ipfs, indexer, merkle-server
// //! 6. Start hashmerchant sidecar with mock loyalty DB
// //! 7. Upload website dist to MinIO
// //! 8. Print deployed contract addresses from state
// //! 9. Wait for Ctrl+C — env tears down on signal
// //!
// //! # Prerequisites
// //!
// //! ```sh
// //! cd terp-core && make build-docker-local
// //! docker images terpnetwork/terp-core:local-zk
// //! cargo build --bin e2e -p terp-scripts --features docker,nostr
// //! ```
// //!

// use std::collections::HashMap;
// use std::sync::Arc;

// use anyhow::{Context, Result};
// use cw_orch::daemon::networks::terp::{TERP_LOCAL_CHAIN_ID, TERP_LOCAL_CHAIN_ID_B};
// use cw_orch::daemon::networks::TERP_LOCALNET;
// use cw_orch::daemon::DaemonBuilder;
// use cw_orch::environment::{ChainInfoOwned, ChainKind, TxHandler};
// use cw_orch::prelude::*;
// use hex;
// use ict_rs::prelude::*;
// use ict_rs::testing::TestEnv;
// use log::info;
// use terp_scripts::suite::TerpNetworkDeployData;
// use terp_scripts::suite::{HealthStatus, SidecarFleet};
// use sha2;
// use sha2::Digest;
// use tokio::signal;

// // ---------------------------------------------------------------------------
// // Constants
// // ---------------------------------------------------------------------------
// Daemon seed: std::env::var("ICT_TEST_MNEMONIC") or "TEST_MNEMONIC" only — never a source literal.
// const TEST_NAME: &str = "e2e-full";
// const IMAGE_REPO: &str = "terpnetwork/terp-core";
// const IMAGE_TAG: &str = "v5.2.0-zk-localterp";

// // ---------------------------------------------------------------------------
// // Chain configs
// // ---------------------------------------------------------------------------

// fn main() -> Result<()> {
//     rustls::crypto::aws_lc_rs::default_provider()
//         .install_default()
//         .unwrap();
//     env_logger::init();
//     dotenv::dotenv().ok();

//     // Run the entire test in a dedicated blocking-friendly runtime
//     let rt = tokio::runtime::Builder::new_current_thread()
//         .enable_all()
//         .build()
//         .context("Failed to create tokio runtime")?;

//     rt.block_on(async { run().await }).map_err(|e| {
//         eprintln!("E2E failed: {e}");
//         // Emergency kill on failure
//         let _ = std::process::Command::new("docker")
//             .args(["ps", "-a", "-q", "--filter", "name=ict-e2e-full"])
//             .output()
//             .and_then(|out| {
//                 let ids = String::from_utf8_lossy(&out.stdout).trim().to_string();
//                 if !ids.is_empty() {
//                     let _ = std::process::Command::new("docker")
//                         .args(["rm", "-f"])
//                         .args(ids.split_whitespace())
//                         .output();
//                 }
//                 Ok(())
//             });
//         e
//     })
// }

// fn build_chain_info(c: &dyn Chain, id: &str) -> ChainInfoOwned {
//     let mut info: ChainInfoOwned = TERP_LOCALNET.into();
//     info.chain_id = match id {
//         "240u-1" => TERP_LOCAL_CHAIN_ID.into(),
//         _ => "240u-2".to_string(),
//     };
//     info.gas_denom = "uterp".into();
//     info.gas_price = 0.25;
//     info.grpc_urls = vec![c.host_grpc_address().to_string()];
//     info.kind = ChainKind::Local;
//     info.network_info.chain_name = "terp network".into();
//     info.network_info.pub_address_prefix = "terp".into();
//     info
// }
// fn mk_config(chain_id: &str) -> ict_rs::chain::ChainConfig {
//     let mut cfg = TestEnv::terp_localterp_config();
//     cfg.chain_id = chain_id.to_string();
//     cfg
// }

// // ---------------------------------------------------------------------------
// // Entry point — manual runtime (avoids cw-orch RUNTIME lazy_static conflict)
// // ---------------------------------------------------------------------------

// pub async fn spawn_dual_chain(
//     a_id: &str,
//     b_id: &str,
//     nw: &str,
//     rt: Arc<dyn RuntimeBackend>,
// ) -> Result<Interchain> {
//     // spawn 2 cosmos chains, and a relayer
//     let ca = CosmosChain::new(mk_config(a_id), 1, 0, rt.clone());
//     let cb = CosmosChain::new(mk_config(b_id), 1, 0, rt.clone());
//     let rl = build_relayer(RelayerType::Hermes, rt.clone(), TEST_NAME, &nw).await?;

//     let mut ic = Interchain::new(rt)
//         .add_chain(Box::new(ca))
//         .add_chain(Box::new(cb))
//         .add_relayer("hermes", rl)
//         .add_link(InterchainLink {
//             chain1: a_id.into(),
//             chain2: b_id.into(),
//             relayer: "hermes".into(),
//             path: "ibc-path".into(),
//         });

//     ic.build(InterchainBuildOptions {
//         test_name: TEST_NAME.into(),
//         skip_path_creation: false,
//         genesis_wallets: HashMap::default(),
//     })
//     .await?;
//     Ok(ic)
// }

// async fn run() -> Result<()> {
//     clean_docker()?;
//     let network_id = format!("ict-{TEST_NAME}");
//     let mut infos: Vec<(&str, ChainInfoOwned)> = Vec::new();
//     println!("═══ Terp Network E2E Suite ═══");
//     println!("Chains: {TERP_LOCAL_CHAIN_ID}, {TERP_LOCAL_CHAIN_ID_B}");
//     println!("Image: {IMAGE_REPO}:{IMAGE_TAG}");
//     let rt: Arc<dyn RuntimeBackend> = IctRuntime::Docker(DockerConfig::default())
//         .into_backend()
//         .await
//         .context("Docker rt creation failed")?;
//     rt.create_network(&network_id).await?;
//     let mut ic = spawn_dual_chain(
//         TERP_LOCAL_CHAIN_ID,
//         TERP_LOCAL_CHAIN_ID_B,
//         &network_id,
//         rt.clone(),
//     )
//     .await?;

//     for id in [TERP_LOCAL_CHAIN_ID, TERP_LOCAL_CHAIN_ID_B] {
//         let c = ic.get_chain(id).expect("chain exists");
//         c.build_wallet("shitter", &std::env::var("ICT_TEST_MNEMONIC")?).await?;
//         let shit = c.key_address("shitter").await?;
//         let fund = 10_000_000_000u128;
//         c.send_funds(
//             "validator",
//             &WalletAmount {
//                 address: shit.clone(),
//                 denom: "uterp".to_string(),
//                 amount: fund,
//             },
//         )
//         .await?;
//         let grpc = c.host_grpc_address();
//         println!("  Chain {} -> gRPC: {}", id, grpc);
//         infos.push((id, build_chain_info(c, id)));
//     }

//     let (id_a, info_a) = infos.remove(0);
//     let (id_b, info_b) = infos.remove(0);
//     assert_eq!((TERP_LOCAL_CHAIN_ID, TERP_LOCAL_CHAIN_ID), (id_a, id_b));

//     // // Build daemons (blocking — cw-orch uses tokio::runtime::Handle internally)
//     // // https://orchestrator.abstract.money/interchain/integrations/daemon.html#for-scripting
//     // let (da, db, suite) = tokio::task::spawn_blocking(move || -> Result<_> {
//     //     let handle = tokio::runtime::Handle::current();

//     //     let da = DaemonBuilder::new(info_a)
//     //         .handle(&handle)
//     //         .mnemonic(&std::env::var("ICT_TEST_MNEMONIC")?)
//     //         .build()?;

//     //     let db = DaemonBuilder::new(info_b)
//     //         .handle(&handle)
//     //         .mnemonic(&std::env::var("ICT_TEST_MNEMONIC")?)
//     //         .state(da.state())
//     //         .build()?;

//     //     da.wait_blocks(1)?;
//     //     //  deploy main app-layer suite
//     //     let dao_data = TerpNetworkDeployData::local_default(da.sender_addr(), &[])?;
//     //     let suite = terp_scripts::suite::DeploySuite::deploy_on(da.clone(), dao_data)?;
//     //     Ok((da, db, suite))
//     // })
//     // .await??;

//     let sender = da.sender_addr();
//     info!("Deployer: {}", sender);

//     println!("\n--- Patching website config.json ---");
//     e2e_patch_static_website_config()?;

//     // 8. Start SidecarFleet for infrastructure services
//     println!("\n--- Starting sidecars ---");
//     let mut fleet = SidecarFleet::new(TEST_NAME)
//         .with_minio_ipfs_defaults()
//         .with_merkle_server_defaults()
//         .with_hashmerchant_defaults(TERP_LOCAL_CHAIN_ID)
//         .with_nostr_relay("nostr-e2e")
//         .with_minimal_indexer("argus");
//     fleet.start_all().await?;
//     println!("  MinIO, merkle-server, hashmerchant, indexer, nostr-relay running");

//     // ── Verify all sidecar endpoints ──────────────────────────────────────────
//     println!("\n--- Verifying sidecar endpoints ---");
//     let health = fleet.health_all().await?;
//     for (id, status) in &health {
//         let ok = matches!(status, HealthStatus::Healthy);
//         let label = if ok { "HEALTHY" } else { "UNHEALTHY" };
//         println!("  {id}: {label}");
//         if !ok {
//             anyhow::bail!("{id} health check failed: {status:?}");
//         }
//     }

//     // ── Hashmerchant full workflow verification ──────────────────────────────
//     println!("\n--- Hashmerchant workflow verification ---");
//     let hm_url = fleet
//         .endpoint("hashmerchant", "http")
//         .context("hashmerchant endpoint not found")?;
//     let client = reqwest::Client::new();

//     // 1. Blob upload + retrieve
//     let content = b"hashmerchant e2e verification blob";
//     let upload = client
//         .post(format!("{hm_url}/blobs"))
//         .body(content.to_vec())
//         .send()
//         .await?;
//     assert_eq!(upload.status(), 200, "POST /blobs failed");
//     let desc: serde_json::Value = upload.json().await?;
//     let hash_hex = desc["sha256"]
//         .as_str()
//         .context("sha256 field missing from upload response")?;

//     // 2. Retrieve blob & verify hash
//     let get_resp = client
//         .get(format!("{hm_url}/blobs/{hash_hex}"))
//         .send()
//         .await?;
//     assert_eq!(get_resp.status(), 200, "GET /blobs/{{hash}} failed");
//     let body = get_resp.text().await?;
//     let expected = sha2::Sha256::digest(content);
//     assert_eq!(hex::encode(expected), hash_hex, "hash mismatch");

//     // 3. Upload merkle tree
//     let tree_id = "e2e-test-tree";
//     let tree_input = serde_json::json!({
//         "merkle_root": "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
//         "accounts": {
//             "terp1e2etest000000000000000000000000000000": {
//                 "tier": 1,
//                 "allocation": 1000,
//                 "proof_hashes": [
//                     "0000000000000000000000000000000000000000000000000000000000000001"
//                 ]
//             }
//         }
//     });
//     let tree_resp = client
//         .post(format!("{hm_url}/trees/{tree_id}"))
//         .json(&tree_input)
//         .send()
//         .await?;
//     assert_eq!(tree_resp.status(), 201, "POST /trees/{{id}} failed");
//     println!("  3. Merkle tree uploaded");

//     // 4. List trees
//     let list_resp = client.get(format!("{hm_url}/trees")).send().await?;
//     assert_eq!(list_resp.status(), 200, "GET /trees failed");
//     let trees: Vec<String> = list_resp.json().await?;
//     assert!(
//         trees.iter().any(|t| t == tree_id),
//         "tree {tree_id} not in list"
//     );
//     println!("  4. Tree listed in /trees");

//     // 5. Register headstash
//     let hs_id = "e2e-test-headstash";
//     let hs_input = serde_json::json!({
//         "blobs": [
//             {
//                 "url": format!("{hm_url}/blobs/{hash_hex}"),
//                 "sha256": hash_hex,
//                 "size": content.len(),
//                 "type": "text/plain"
//             }
//         ]
//     });
//     let hs_resp = client
//         .post(format!("{hm_url}/headstash/{hs_id}"))
//         .json(&hs_input)
//         .send()
//         .await?;
//     assert_eq!(hs_resp.status(), 201, "POST /headstash/{{id}} failed");
//     println!("  5. Headstash registered");

//     // 6. Retrieve headstash
//     let hs_get = client
//         .get(format!("{hm_url}/headstash/{hs_id}"))
//         .send()
//         .await?;
//     assert_eq!(hs_get.status(), 200, "GET /headstash/{{id}} failed");
//     println!("  6. Headstash retrieved");

//     // 7. Delete blob
//     let del_resp = client
//         .delete(format!("{hm_url}/blobs/{hash_hex}"))
//         .send()
//         .await?;
//     assert_eq!(del_resp.status(), 204, "DELETE /blobs/{{hash}} failed");
//     println!("  7. Blob deleted");

//     // 8. Verify blob is gone
//     let get_gone = client
//         .get(format!("{hm_url}/blobs/{hash_hex}"))
//         .send()
//         .await?;
//     assert_eq!(get_gone.status(), 404, "deleted blob should return 404");
//     println!("  8. Deleted blob returns 404 — confirmed");

//     // 9. Delete tree
//     let del_tree = client
//         .delete(format!("{hm_url}/trees/{tree_id}"))
//         .send()
//         .await?;
//     assert_eq!(del_tree.status(), 204, "DELETE /trees/{{id}} failed");
//     println!("  9. Tree deleted");

//     println!("\n═══ Hashmerchant workflow: all 9 steps passed ═══");

//     // 9. Upload website dist to MinIO
//     println!("\n--- Uploading website dist ---");
//     let dist = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
//         .parent()
//         .unwrap()
//         .parent()
//         .unwrap()
//         .join("websites/terp.network/dist");
//     if dist.exists() {
//         let status = std::process::Command::new("mc")
//             .arg("cp")
//             .arg("--recursive")
//             .arg(dist.to_str().unwrap())
//             .arg("usb2/static/terp.network/dist")
//             .status()?;
//         if status.success() {
//             println!("  dist/ → MinIO/static/terp.network/dist");
//         } else {
//             eprintln!("  Warning: mc upload exit code {:?}", status.code());
//         }
//     } else {
//         eprintln!("  Warning: dist/ not found at {:?}", dist);
//     }

//     // 10. Print cw-orchestrator state (contract addresses → frontends)
//     println!("\n--- Contract addresses (from ~/.cw-orchestrator/state.json) ---");
//     let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
//     let state_path = std::path::PathBuf::from(home).join(".cw-orchestrator/state.json");
//     if state_path.exists() {
//         let raw = std::fs::read_to_string(&state_path)?;
//         if let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) {
//             for chain_id in [TERP_LOCAL_CHAIN_ID, TERP_LOCAL_CHAIN_ID_B] {
//                 if let Some(chain) = json.get(chain_id) {
//                     println!("  [{chain_id}]");
//                     if let Some(defaults) = chain.get("default") {
//                         if let Some(obj) = defaults.as_object() {
//                             for (k, v) in obj {
//                                 println!("    {k}: {}", v.as_str().unwrap_or("?"));
//                             }
//                         }
//                     }
//                 }
//             }
//         }
//     }

//     println!("\n═══ Environment ready — press Ctrl+C to stop ═══\n");
//     println!("Chain A: http://127.0.0.1:26657");
//     println!("Chain B: http://127.0.0.1:26659 (or 9092 gRPC)");
//     println!("MinIO:  http://127.0.0.1:9000");
//     println!("Merkle: http://127.0.0.1:8765");
//     println!("");

//     // 11. Wait for Ctrl+C — NO teardown until signal
//     signal::ctrl_c().await?;
//     println!("\n--- Shutting down ---");

//     // Cleanup: stop sidecars first, then interchain
//     if let Err(e) = fleet.stop_all().await {
//         eprintln!("  Sidecar stop warning: {e}");
//     }
//     if let Err(e) = ic.close().await {
//         eprintln!("  Interchain close warning: {e}");
//     }

//     // Explicitly remove Docker containers and network so next run doesn't conflict
//     println!("  Removing Docker artifacts...");
//     let names = [
//         format!("ict-{TEST_NAME}-{TERP_LOCAL_CHAIN_ID}-val-0"),
//         format!("ict-{TEST_NAME}-{TERP_LOCAL_CHAIN_ID_B}-val-0"),
//         format!("ict-{TEST_NAME}-hermes-bg"),
//     ];
//     for name in &names {
//         let _ = std::process::Command::new("docker")
//             .args(["rm", "--force", name])
//             .output();
//     }
//     let net = format!("ict-{TEST_NAME}");
//     let _ = std::process::Command::new("docker")
//         .args(["network", "rm", "--force", &net])
//         .output();
//     println!("  Containers and network removed");
//     Ok(())
// }

// fn clean_docker() -> Result<()> {
//     let network_id = format!("ict-{TEST_NAME}");
//     println!("  Network: {network_id}");
//     let _ = std::process::Command::new("docker")
//         .args([
//             "ps",
//             "-a",
//             "-q",
//             "--filter",
//             &format!("name=ict-{TEST_NAME}"),
//         ])
//         .output()
//         .and_then(|output| {
//             let ids = String::from_utf8_lossy(&output.stdout);
//             if !ids.trim().is_empty() {
//                 let _ = std::process::Command::new("docker")
//                     .args(["rm", "-f"])
//                     .args(ids.split_whitespace().collect::<Vec<_>>())
//                     .output();
//             }
//             Ok(())
//         });
//     let _ = std::process::Command::new("docker")
//         .args([
//             "volume",
//             "ls",
//             "-q",
//             "--filter",
//             &format!("name=ict-{TEST_NAME}"),
//         ])
//         .output()
//         .and_then(|output| {
//             let vols = String::from_utf8_lossy(&output.stdout);
//             if !vols.trim().is_empty() {
//                 let _ = std::process::Command::new("docker")
//                     .args(["volume", "rm", "-f"])
//                     .args(vols.split_whitespace().collect::<Vec<_>>())
//                     .output();
//             }
//             Ok(())
//         });
//     let _ = std::process::Command::new("docker")
//         .args(["network", "rm", "--force", &network_id])
//         .output();
//     Ok(())
// }

// pub fn e2e_patch_static_website_config() -> Result<()> {
//     // TODO: replace with testdata path for shared env.
//     let website_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
//         .parent()
//         .unwrap()
//         .parent()
//         .unwrap()
//         .join("websites/terp.network");
//     let config_path = website_root.join("public/config.json");
//     if config_path.exists() {
//         let raw = std::fs::read_to_string(&config_path)?;
//         if let Ok(mut config) = serde_json::from_str::<serde_json::Value>(&raw) {
//             let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
//             let state_path = std::path::PathBuf::from(home).join(".cw-orchestrator/state.json");
//             if state_path.exists() {
//                 let state_raw = std::fs::read_to_string(&state_path)?;
//                 if let Ok(state) = serde_json::from_str::<serde_json::Value>(&state_raw) {
//                     if let Some(chain_entry) = state.get(TERP_LOCAL_CHAIN_ID) {
//                         if let Some(defaults) = chain_entry.get("default") {
//                             let addrs: Vec<(String, String)> = defaults
//                                 .as_object()
//                                 .unwrap()
//                                 .iter()
//                                 .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("?").to_string()))
//                                 .collect();
//                             let chains = config["chains"].as_object_mut().unwrap();
//                             let entry = chains
//                                 .entry(TERP_LOCAL_CHAIN_ID.to_string())
//                                 .or_insert(serde_json::json!({}));
//                             if !entry.as_object().unwrap().contains_key("contracts") {
//                                 entry["contracts"] = serde_json::json!({});
//                             }
//                             let contracts = entry["contracts"].as_object_mut().unwrap();
//                             for (k, v) in &addrs {
//                                 contracts.insert(k.clone(), serde_json::json!(v));
//                                 println!("  {k}: {v}");
//                             }
//                             let updated = serde_json::to_string_pretty(&config)?;
//                             std::fs::write(&config_path, updated)?;
//                             println!("  config.json patched with {} contracts", addrs.len());
//                         }
//                     }
//                 }
//             }
//         }
//     }
//     Ok(())
// }

fn main() {}
