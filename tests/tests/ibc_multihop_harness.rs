//! 4-chain line multi-hop tokenfactory authenticity harness.
//!
//! Spawns Terp chains A—B—C—D via ict-rs, creates tokenfactory denoms,
//! executes hop-by-hop ICS-20 transfers for scenarios 1–6, and asserts:
//!
//!   predict(trace_path) → ibc/HASH  ==  denom_trace  ==  bank balance denom
//!
//! Live run (Docker + Terp image required):
//! ```sh
//! cargo test -p terp-scripts --test ibc_multihop_harness -- --ignored --nocapture
//! ```
//!
//! Layout notes: `tests/data/ibc/harness/README.md`.

#![cfg(feature = "docker")]

use std::collections::HashMap;

use ict_rs::chain::cosmos::CosmosChain;
use ict_rs::chain::{Chain, ChainConfig, ChainType, SigningAlgorithm};
use ict_rs::interchain::{Interchain, InterchainBuildOptions, InterchainLink, wait_for_blocks};
use ict_rs::modules::tokenfactory::TokenfactoryMsgExt;
use ict_rs::relayer::{RelayerType, build_relayer};
use ict_rs::runtime::{DockerConfig, DockerImage, IctRuntime};
use ict_rs::tx::{TransferOptions, WalletAmount};
use serde_json::json;
use terp_scripts::ibc::{
    ChannelHop, IBCChannelGraph, PredictedWorld, check_invariants, compute_ibc_denom_hash,
    hop_count_from_trace_path,
};

// ---------------------------------------------------------------------------
// Topology constants
// ---------------------------------------------------------------------------

const CHAIN_A: &str = "terp-a";
const CHAIN_B: &str = "terp-b";
const CHAIN_C: &str = "terp-c";
const CHAIN_D: &str = "terp-d";

const USER_KEY: &str = "user";
const VALIDATOR_KEY: &str = "validator";
const NATIVE_DENOM: &str = "uterp";
const FUND_AMOUNT: u128 = 10_000_000_000;
const MINT_AMOUNT: u128 = 1_000_000_000;
const TRANSFER_AMOUNT: u128 = 1_000;
const RELAY_WAIT_BLOCKS: u64 = 12;

// ---------------------------------------------------------------------------
// Pure prediction helpers (same geometry as ibc_core::compute_ibc_denom_for_route)
// ---------------------------------------------------------------------------

/// Build ICS-20 trace path from the destination looking back toward origin.
///
/// `receive_channels_looking_back` is ordered **final dest receive channel first**,
/// then previous hop receive channels, ending at the first hop's receive channel.
///
/// Example (A→B→C): `[ch_on_C, ch_on_B]` + base →
/// `transfer/{ch_on_C}/transfer/{ch_on_B}/{base}`.
fn predict_trace_path(receive_channels_looking_back: &[&str], base_denom: &str) -> String {
    let mut path = String::new();
    for ch in receive_channels_looking_back {
        path.push_str("transfer/");
        path.push_str(ch);
        path.push('/');
    }
    path.push_str(base_denom);
    path
}

fn predict_ibc_denom(trace_path: &str) -> String {
    compute_ibc_denom_hash(trace_path)
}

/// Ordered hop list origin→dest: each entry is the **receive** channel on the hop destination.
fn predict_after_hops(
    receive_channels_origin_to_dest: &[&str],
    base_denom: &str,
) -> (String, String) {
    let looking_back: Vec<&str> = receive_channels_origin_to_dest
        .iter()
        .rev()
        .copied()
        .collect();
    let path = predict_trace_path(&looking_back, base_denom);
    let denom = predict_ibc_denom(&path);
    // Cross-check hop_count honesty + graph helper agreement
    assert_eq!(
        hop_count_from_trace_path(&path),
        receive_channels_origin_to_dest.len()
    );
    (path, denom)
}

/// Same geometry via lib `IBCChannelGraph::compute_ibc_denom_for_route` (must match helpers).
fn predict_via_graph(
    hops_origin_to_dest: &[(
        /*from*/ &str,
        /*from_ch*/ &str,
        /*to*/ &str,
        /*to_ch*/ &str,
    )],
    base: &str,
) -> (String, String) {
    let graph = IBCChannelGraph::new();
    let route: Vec<ChannelHop> = hops_origin_to_dest
        .iter()
        .map(|(f, fc, t, tc)| ChannelHop {
            from_chain: f.to_string(),
            from_channel: fc.to_string(),
            to_chain: t.to_string(),
            to_channel: tc.to_string(),
        })
        .collect();
    graph.compute_ibc_denom_for_route(base, &route)
}

// ---------------------------------------------------------------------------
// Chain config
// ---------------------------------------------------------------------------

fn terp_image() -> DockerImage {
    // Prefer TERP_* (e2e examples), fall back to ICT_* (TestEnv), then local-zk.
    let repo = std::env::var("TERP_IMAGE_REPO")
        .or_else(|_| std::env::var("ICT_IMAGE_REPO"))
        .unwrap_or_else(|_| "terpnetwork/terp-core".to_string());
    let version = std::env::var("TERP_IMAGE_VERSION")
        .or_else(|_| std::env::var("ICT_IMAGE_VERSION"))
        .unwrap_or_else(|_| "local-zk".to_string());
    DockerImage {
        repository: repo,
        version,
        uid_gid: None,
    }
}

fn terp_line_config(chain_id: &str) -> ChainConfig {
    ChainConfig {
        chain_type: ChainType::Cosmos,
        name: chain_id.to_string(),
        chain_id: chain_id.to_string(),
        images: vec![terp_image()],
        bin: "terpd".to_string(),
        bech32_prefix: "terp".to_string(),
        denom: NATIVE_DENOM.to_string(),
        coin_type: 118,
        signing_algorithm: SigningAlgorithm::Secp256k1,
        gas_prices: "0uterp".to_string(),
        gas_adjustment: 2.0,
        trusting_period: "112h".to_string(),
        block_time: "2s".to_string(),
        genesis: None,
        modify_genesis: None,
        pre_genesis: None,
        config_file_overrides: HashMap::new(),
        additional_start_args: Vec::new(),
        env: Vec::new(),
        sidecar_configs: Vec::new(),
        faucet: None,
        genesis_style: Default::default(),
    }
}

// ---------------------------------------------------------------------------
// Query helpers
// ---------------------------------------------------------------------------

async fn key_address(chain: &dyn Chain, key_name: &str) -> String {
    let output = chain
        .chain_exec(&["keys", "show", key_name, "-a", "--keyring-backend", "test"])
        .await
        .unwrap_or_else(|e| panic!("keys show {key_name}: {e}"));
    let addr = output.stdout_str().trim().to_string();
    assert!(!addr.is_empty(), "empty address for key '{key_name}'");
    addr
}

fn factory_denom(creator: &str, subdenom: &str) -> String {
    format!("factory/{creator}/{subdenom}")
}

/// Open transfer channels on a chain: (channel_id, counterparty_channel_id).
async fn list_transfer_channels(chain: &dyn Chain) -> Vec<(String, String)> {
    let output = chain
        .chain_exec(&["query", "ibc", "channel", "channels", "--output", "json"])
        .await
        .unwrap_or_else(|e| panic!("query channels on {}: {e}", chain.chain_id()));

    let v: serde_json::Value =
        serde_json::from_str(output.stdout_str().trim()).unwrap_or_else(|e| {
            panic!(
                "parse channels JSON on {}: {e}\nstdout={}",
                chain.chain_id(),
                output.stdout_str()
            )
        });

    let mut out = Vec::new();
    let channels = v
        .get("channels")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();

    for ch in channels {
        let port = ch.get("port_id").and_then(|p| p.as_str()).unwrap_or("");
        let state = ch.get("state").and_then(|s| s.as_str()).unwrap_or("");
        if port != "transfer" {
            continue;
        }
        // Accept STATE_OPEN or OPEN depending on CLI version.
        if !(state.contains("OPEN") || state == "3") {
            continue;
        }
        let channel_id = ch
            .get("channel_id")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        let cp = ch
            .get("counterparty")
            .and_then(|c| c.get("channel_id"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        if !channel_id.is_empty() && !cp.is_empty() {
            out.push((channel_id, cp));
        }
    }
    out
}

/// Resolve the bidirectional transfer channel pair between two chains.
fn resolve_link(
    chain1_id: &str,
    chain1_channels: &[(String, String)],
    chain2_id: &str,
    chain2_channels: &[(String, String)],
) -> LinkChannels {
    for (ch1, cp1) in chain1_channels {
        for (ch2, cp2) in chain2_channels {
            if ch1 == cp2 && ch2 == cp1 {
                return LinkChannels {
                    chain1: chain1_id.to_string(),
                    chain2: chain2_id.to_string(),
                    channel_on_1: ch1.clone(),
                    channel_on_2: ch2.clone(),
                };
            }
        }
    }
    panic!(
        "no matching transfer channel pair between {chain1_id} ({chain1_channels:?}) and {chain2_id} ({chain2_channels:?})"
    );
}

#[derive(Debug, Clone)]
struct LinkChannels {
    chain1: String,
    chain2: String,
    /// Channel id living on `chain1`.
    channel_on_1: String,
    /// Channel id living on `chain2`.
    channel_on_2: String,
}

impl LinkChannels {
    /// Outbound channel on `from` and receive channel on the counterparty.
    fn send_and_recv(&self, from: &str) -> (String, String) {
        if from == self.chain1 {
            (self.channel_on_1.clone(), self.channel_on_2.clone())
        } else if from == self.chain2 {
            (self.channel_on_2.clone(), self.channel_on_1.clone())
        } else {
            panic!(
                "chain {from} is not a side of link {}-{}",
                self.chain1, self.chain2
            );
        }
    }
}

/// Query denom trace path for an `ibc/HASH` denom (hash hex without prefix).
async fn query_denom_trace_path(chain: &dyn Chain, ibc_denom: &str) -> Option<String> {
    let hash = ibc_denom
        .strip_prefix("ibc/")
        .unwrap_or(ibc_denom)
        .to_string();

    // ibc-go: `query ibc-transfer denom-trace <hash>`
    let output = chain
        .chain_exec(&[
            "query",
            "ibc-transfer",
            "denom-trace",
            &hash,
            "--output",
            "json",
        ])
        .await
        .ok()?;

    if output.exit_code != 0 {
        // Fallback: some versions use `denom` query with full ibc/HASH.
        let output2 = chain
            .chain_exec(&[
                "query",
                "ibc-transfer",
                "denom",
                ibc_denom,
                "--output",
                "json",
            ])
            .await
            .ok()?;
        if output2.exit_code != 0 {
            return None;
        }
        return parse_trace_path(&output2.stdout_str());
    }

    parse_trace_path(&output.stdout_str())
}

fn parse_trace_path(stdout: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    // Common shapes:
    // { "denom_trace": { "path": "transfer/channel-0", "base_denom": "uterp" } }
    // { "trace": { "path": "...", "base_denom": "..." } }
    // { "path": "transfer/channel-0/uterp" }  (full path variants)
    if let Some(dt) = v.get("denom_trace").or_else(|| v.get("trace")) {
        let path = dt.get("path").and_then(|p| p.as_str()).unwrap_or("");
        let base = dt.get("base_denom").and_then(|b| b.as_str()).unwrap_or("");
        if !path.is_empty() && !base.is_empty() {
            // path field is usually "transfer/channel-0" without base.
            if path.contains(base) {
                return Some(path.to_string());
            }
            return Some(format!("{path}/{base}"));
        }
        if !path.is_empty() {
            return Some(path.to_string());
        }
    }
    v.get("path")
        .and_then(|p| p.as_str())
        .map(|s| s.to_string())
}

fn auth_mismatch(
    scenario: u32,
    hop: usize,
    expected_path: &str,
    expected_denom: &str,
    actual_trace: &str,
    actual_balance_denom: &str,
) -> ! {
    panic!(
        "AUTH_MISMATCH scenario={scenario} hop={hop} expected_path={expected_path} expected_denom={expected_denom} actual_trace={actual_trace} actual_balance_denom={actual_balance_denom}"
    );
}

// ---------------------------------------------------------------------------
// Scenario runner
// ---------------------------------------------------------------------------

struct ChainUsers {
    /// chain_id → (key name always USER_KEY, bech32 address)
    addrs: HashMap<String, String>,
}

/// Hop-by-hop transfer along `chain_path` (ordered chain ids origin…dest).
/// `links` maps sorted pair key `"a|b"` → LinkChannels.
async fn run_scenario(
    scenario: u32,
    chain_path: &[&str],
    base_denom: &str,
    ic: &Interchain,
    links: &HashMap<String, LinkChannels>,
    users: &ChainUsers,
) {
    assert!(
        chain_path.len() >= 2,
        "scenario {scenario}: need at least origin and dest"
    );

    let mut current_denom = base_denom.to_string();
    let mut receive_channels_origin_to_dest: Vec<String> = Vec::new();

    for hop_idx in 0..chain_path.len() - 1 {
        let from_id = chain_path[hop_idx];
        let to_id = chain_path[hop_idx + 1];
        let hop_num = hop_idx + 1;

        let link_key = link_key(from_id, to_id);
        let link = links
            .get(&link_key)
            .unwrap_or_else(|| panic!("scenario {scenario}: missing link {from_id}-{to_id}"));

        let (send_ch, recv_ch) = link.send_and_recv(from_id);
        let from_chain = ic
            .get_chain(from_id)
            .unwrap_or_else(|| panic!("missing chain {from_id}"));
        let to_chain = ic
            .get_chain(to_id)
            .unwrap_or_else(|| panic!("missing chain {to_id}"));
        let from_addr = users.addrs.get(from_id).expect("from user");
        let to_addr = users.addrs.get(to_id).expect("to user");

        println!(
            "  scenario={scenario} hop={hop_num}: {from_id} --{send_ch}--> {to_id} (recv {recv_ch}) denom={current_denom}"
        );

        let tx = from_chain
            .send_ibc_transfer(
                &send_ch,
                USER_KEY,
                &WalletAmount {
                    address: to_addr.clone(),
                    denom: current_denom.clone(),
                    amount: TRANSFER_AMOUNT,
                },
                &TransferOptions::default(),
            )
            .await
            .unwrap_or_else(|e| {
                panic!("scenario={scenario} hop={hop_num} send_ibc_transfer failed: {e}")
            });
        println!("    tx={} height={}", tx.tx_hash, tx.height);

        wait_for_blocks(from_chain, RELAY_WAIT_BLOCKS)
            .await
            .unwrap_or_else(|e| panic!("wait blocks from: {e}"));
        wait_for_blocks(to_chain, RELAY_WAIT_BLOCKS / 2)
            .await
            .unwrap_or_else(|e| panic!("wait blocks to: {e}"));

        receive_channels_origin_to_dest.push(recv_ch.clone());
        let recv_refs: Vec<&str> = receive_channels_origin_to_dest
            .iter()
            .map(|s| s.as_str())
            .collect();
        let (expected_path, expected_denom) = predict_after_hops(&recv_refs, base_denom);

        let bal = to_chain
            .get_balance(to_addr, &expected_denom)
            .await
            .unwrap_or(0);

        let actual_trace = query_denom_trace_path(to_chain, &expected_denom)
            .await
            .unwrap_or_else(|| "<missing denom_trace>".to_string());

        println!(
            "    expected_path={expected_path}\n    expected_denom={expected_denom}\n    balance={bal}\n    actual_trace={actual_trace}"
        );

        if bal < TRANSFER_AMOUNT {
            // Also report if any other ibc balance appeared under wrong hash.
            auth_mismatch(
                scenario,
                hop_num,
                &expected_path,
                &expected_denom,
                &actual_trace,
                &expected_denom, // balance query denom; amount was insufficient
            );
        }

        if actual_trace != expected_path && actual_trace != "<missing denom_trace>" {
            // Some CLI versions return path without base — accept if path is a prefix match.
            let alt = format!("{}/{}", actual_trace.trim_end_matches('/'), base_denom);
            if actual_trace != expected_path && alt != expected_path {
                auth_mismatch(
                    scenario,
                    hop_num,
                    &expected_path,
                    &expected_denom,
                    &actual_trace,
                    &expected_denom,
                );
            }
        }

        // If denom_trace missing but balance present, still fail closed on trace.
        if actual_trace == "<missing denom_trace>" {
            auth_mismatch(
                scenario,
                hop_num,
                &expected_path,
                &expected_denom,
                &actual_trace,
                &expected_denom,
            );
        }

        // Next hop forwards the voucher held on `to` chain.
        current_denom = expected_denom;
        let _ = from_addr; // funded sender; unused after transfer
    }

    println!("  scenario={scenario} OK path={:?}", chain_path);
}

fn link_key(a: &str, b: &str) -> String {
    if a < b {
        format!("{a}|{b}")
    } else {
        format!("{b}|{a}")
    }
}

// ---------------------------------------------------------------------------
// Pure unit tests (no Docker)
// ---------------------------------------------------------------------------

#[test]
fn predict_single_hop_matches_ibc_core_hash() {
    let path = predict_trace_path(&["channel-0"], "factory/terp1abc/ta0");
    assert_eq!(path, "transfer/channel-0/factory/terp1abc/ta0");
    let denom = predict_ibc_denom(&path);
    assert!(denom.starts_with("ibc/"));
    assert_eq!(denom, compute_ibc_denom_hash(&path));
    assert_eq!(denom, denom.to_uppercase().replacen("IBC/", "ibc/", 1));
}

#[test]
fn predict_helpers_match_channel_graph() {
    // A→B→C with distinct send/receive channels: only receive channels enter the path.
    let hops = [
        ("chain-a", "channel-5", "chain-b", "channel-0"),
        ("chain-b", "channel-7", "chain-c", "channel-1"),
    ];
    let (path, denom) = predict_after_hops(&["channel-0", "channel-1"], "factory/x/ta1");
    let (graph_denom, graph_path) = predict_via_graph(&hops, "factory/x/ta1");
    assert_eq!(graph_path, path);
    assert_eq!(graph_denom, denom);
}

#[test]
fn predict_multi_hop_looking_back_order() {
    // A→B→C: receive on B is channel-0, receive on C is channel-1
    let (path, denom) = predict_after_hops(&["channel-0", "channel-1"], "factory/x/ta1");
    assert_eq!(path, "transfer/channel-1/transfer/channel-0/factory/x/ta1");
    assert_eq!(denom, compute_ibc_denom_hash(&path));
    // hop_count geometry: number of transfer/ segments == hops
    assert_eq!(path.matches("transfer/").count(), 2);
}

#[test]
fn predict_triple_hop_geometry() {
    let (path, _) = predict_after_hops(&["channel-0", "channel-1", "channel-0"], "factory/x/ta2");
    assert_eq!(
        path,
        "transfer/channel-0/transfer/channel-1/transfer/channel-0/factory/x/ta2"
    );
    assert_eq!(path.matches("transfer/").count(), 3);
}

// ---------------------------------------------------------------------------
// Live harness
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires Docker + Terp image; live multi-hop authenticity proof"]
async fn test_line_four_chain_tokenfactory_multihop() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    println!("=== 4-chain line tokenfactory multi-hop authenticity harness ===");

    let runtime = IctRuntime::Docker(DockerConfig::default())
        .into_backend()
        .await
        .expect("Docker runtime — is the daemon running?");

    let test_name = "ibc-multihop-auth";
    let network_id = format!("ict-{test_name}");
    runtime
        .create_network(&network_id)
        .await
        .expect("create docker network");

    let chain_a = CosmosChain::new(terp_line_config(CHAIN_A), 1, 0, runtime.clone());
    let chain_b = CosmosChain::new(terp_line_config(CHAIN_B), 1, 0, runtime.clone());
    let chain_c = CosmosChain::new(terp_line_config(CHAIN_C), 1, 0, runtime.clone());
    let chain_d = CosmosChain::new(terp_line_config(CHAIN_D), 1, 0, runtime.clone());

    let relayer = build_relayer(RelayerType::Hermes, runtime.clone(), test_name, &network_id)
        .await
        .expect("build Hermes relayer");

    let mut ic = Interchain::new(runtime)
        .add_chain(Box::new(chain_a))
        .add_chain(Box::new(chain_b))
        .add_chain(Box::new(chain_c))
        .add_chain(Box::new(chain_d))
        .add_relayer("hermes", relayer)
        .add_link(InterchainLink {
            chain1: CHAIN_A.to_string(),
            chain2: CHAIN_B.to_string(),
            relayer: "hermes".to_string(),
            path: "path-ab".to_string(),
        })
        .add_link(InterchainLink {
            chain1: CHAIN_B.to_string(),
            chain2: CHAIN_C.to_string(),
            relayer: "hermes".to_string(),
            path: "path-bc".to_string(),
        })
        .add_link(InterchainLink {
            chain1: CHAIN_C.to_string(),
            chain2: CHAIN_D.to_string(),
            relayer: "hermes".to_string(),
            path: "path-cd".to_string(),
        });

    println!("Building interchain (4 chains, line links)...");
    ic.build(InterchainBuildOptions {
        test_name: test_name.to_string(),
        ..Default::default()
    })
    .await
    .expect("Interchain::build");

    let result = run_harness_body(&ic).await;

    println!("--- Shutdown ---");
    if let Err(e) = ic.close().await {
        eprintln!("warning: interchain close: {e}");
    }

    result.expect("harness body");
}

async fn run_harness_body(ic: &Interchain) -> Result<(), String> {
    // --- Fund users ---
    let chain_ids = [CHAIN_A, CHAIN_B, CHAIN_C, CHAIN_D];
    let mut addrs = HashMap::new();

    for id in chain_ids {
        let chain = ic.get_chain(id).ok_or_else(|| format!("missing {id}"))?;
        chain
            .create_key(USER_KEY)
            .await
            .map_err(|e| format!("create_key on {id}: {e}"))?;
        let addr = key_address(chain, USER_KEY).await;
        println!("  {id} user: {addr}");
        chain
            .send_funds(
                VALIDATOR_KEY,
                &WalletAmount {
                    address: addr.clone(),
                    denom: NATIVE_DENOM.to_string(),
                    amount: FUND_AMOUNT,
                },
            )
            .await
            .map_err(|e| format!("fund user on {id}: {e}"))?;
        addrs.insert(id.to_string(), addr);
    }

    for id in chain_ids {
        let chain = ic.get_chain(id).unwrap();
        wait_for_blocks(chain, 3)
            .await
            .map_err(|e| format!("wait fund: {e}"))?;
    }

    let users = ChainUsers { addrs };

    // --- Tokenfactory matrix ---
    let matrix: [(&str, [&str; 4]); 4] = [
        (CHAIN_A, ["ta0", "ta1", "ta2", "ta3"]),
        (CHAIN_B, ["tb0", "tb1", "tb2", "tb3"]),
        (CHAIN_C, ["tc0", "tc1", "tc2", "tc3"]),
        (CHAIN_D, ["td0", "td1", "td2", "td3"]),
    ];

    let mut factory_denoms: HashMap<String, String> = HashMap::new();

    for (chain_id, subdenoms) in &matrix {
        let chain = ic.get_chain(chain_id).unwrap();
        let creator = users.addrs.get(*chain_id).unwrap();
        for sub in subdenoms {
            chain
                .tokenfactory_create_denom(USER_KEY, sub)
                .await
                .map_err(|e| format!("create-denom {chain_id}/{sub}: {e}"))?;
            let denom = factory_denom(creator, sub);
            let mint_coin = format!("{MINT_AMOUNT}{denom}");
            chain
                .tokenfactory_mint(USER_KEY, &mint_coin, "")
                .await
                .map_err(|e| format!("mint {denom}: {e}"))?;
            factory_denoms.insert(format!("{chain_id}:{sub}"), denom.clone());
            println!("  created+minted {denom}");
        }
        wait_for_blocks(chain, 2)
            .await
            .map_err(|e| format!("wait tf: {e}"))?;
    }

    // --- Discover channel sides ---
    let mut per_chain_channels: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for id in chain_ids {
        let chain = ic.get_chain(id).unwrap();
        let chs = list_transfer_channels(chain).await;
        println!("  channels on {id}: {chs:?}");
        per_chain_channels.insert(id.to_string(), chs);
    }

    let ab = resolve_link(
        CHAIN_A,
        per_chain_channels.get(CHAIN_A).unwrap(),
        CHAIN_B,
        per_chain_channels.get(CHAIN_B).unwrap(),
    );
    let bc = resolve_link(
        CHAIN_B,
        per_chain_channels.get(CHAIN_B).unwrap(),
        CHAIN_C,
        per_chain_channels.get(CHAIN_C).unwrap(),
    );
    let cd = resolve_link(
        CHAIN_C,
        per_chain_channels.get(CHAIN_C).unwrap(),
        CHAIN_D,
        per_chain_channels.get(CHAIN_D).unwrap(),
    );

    println!("  link A-B: {ab:?}");
    println!("  link B-C: {bc:?}");
    println!("  link C-D: {cd:?}");

    let mut links = HashMap::new();
    links.insert(link_key(CHAIN_A, CHAIN_B), ab.clone());
    links.insert(link_key(CHAIN_B, CHAIN_C), bc.clone());
    links.insert(link_key(CHAIN_C, CHAIN_D), cd.clone());

    // --- Lib PredictedWorld: same authenticity path as generate/validate ---
    let ibc_data = harness_ibc_data(&ab, &bc, &cd);
    let mut chain_assets: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    for (chain_id, subdenoms) in &matrix {
        let creator = users.addrs.get(*chain_id).unwrap();
        let assets: Vec<_> = subdenoms
            .iter()
            .map(|sub| {
                let denom = factory_denom(creator, sub);
                json!({
                    "symbol": format!("{chain_id}_{sub}"),
                    "base": denom,
                })
            })
            .collect();
        chain_assets.insert(chain_id.to_string(), assets);
    }
    let world = PredictedWorld::from_inputs(ibc_data, &chain_assets, 3);
    let inv = check_invariants(&world);
    for item in &inv.items {
        println!(
            "  [PredictedWorld {:?}] {} — {}",
            item.severity, item.code, item.message
        );
    }
    assert!(
        !inv.has_errors(),
        "harness PredictedWorld must pass check_invariants before transfers"
    );
    println!(
        "  ✓ PredictedWorld clean: {} routes",
        world.routes.metadata.total_routes
    );

    // --- Scenarios 1–6 ---
    let ta0 = factory_denoms
        .get(&format!("{CHAIN_A}:ta0"))
        .unwrap()
        .clone();
    let ta1 = factory_denoms
        .get(&format!("{CHAIN_A}:ta1"))
        .unwrap()
        .clone();
    let ta2 = factory_denoms
        .get(&format!("{CHAIN_A}:ta2"))
        .unwrap()
        .clone();
    let td0 = factory_denoms
        .get(&format!("{CHAIN_D}:td0"))
        .unwrap()
        .clone();
    let tb0 = factory_denoms
        .get(&format!("{CHAIN_B}:tb0"))
        .unwrap()
        .clone();
    let tc0 = factory_denoms
        .get(&format!("{CHAIN_C}:tc0"))
        .unwrap()
        .clone();

    println!("\n--- Scenario 1: A TF → B (1 hop) ---");
    run_scenario(1, &[CHAIN_A, CHAIN_B], &ta0, ic, &links, &users).await;

    println!("\n--- Scenario 2: A TF → C (2 hops) ---");
    run_scenario(2, &[CHAIN_A, CHAIN_B, CHAIN_C], &ta1, ic, &links, &users).await;

    println!("\n--- Scenario 3: A TF → D (3 hops) ---");
    run_scenario(
        3,
        &[CHAIN_A, CHAIN_B, CHAIN_C, CHAIN_D],
        &ta2,
        ic,
        &links,
        &users,
    )
    .await;

    println!("\n--- Scenario 4: D TF → A ---");
    run_scenario(
        4,
        &[CHAIN_D, CHAIN_C, CHAIN_B, CHAIN_A],
        &td0,
        ic,
        &links,
        &users,
    )
    .await;

    println!("\n--- Scenario 5: B TF → D ---");
    run_scenario(5, &[CHAIN_B, CHAIN_C, CHAIN_D], &tb0, ic, &links, &users).await;

    println!("\n--- Scenario 6: C TF → A ---");
    run_scenario(6, &[CHAIN_C, CHAIN_B, CHAIN_A], &tc0, ic, &links, &users).await;

    // Re-check preferred-route invariants after successful scenarios (topology unchanged)
    let inv2 = check_invariants(&world);
    assert!(!inv2.has_errors(), "post-scenario invariants still clean");

    println!("\n=== All scenarios 1–6 authenticity checks passed ===");
    Ok(())
}

/// Build registry-shaped ibc_data for the A–B–C–D line from resolved link sides.
fn harness_ibc_data(ab: &LinkChannels, bc: &LinkChannels, cd: &LinkChannels) -> serde_json::Value {
    fn pair_entry(a: &str, b: &str, ch_a: &str, ch_b: &str) -> (String, serde_json::Value) {
        let (c1, c2, ch1, ch2) = if a < b {
            (a, b, ch_a, ch_b)
        } else {
            (b, a, ch_b, ch_a)
        };
        let key = format!("{c1}-{c2}");
        let entry = json!({
            "$schema": "../ibc_data.schema.json",
            "chain_1": {
                "chain_name": c1,
                "chain_id": c1,
                "client_id": "07-tendermint-0",
                "connection_id": "connection-0"
            },
            "chain_2": {
                "chain_name": c2,
                "chain_id": c2,
                "client_id": "07-tendermint-0",
                "connection_id": "connection-0"
            },
            "channels": [{
                "chain_1": { "channel_id": ch1, "port_id": "transfer" },
                "chain_2": { "channel_id": ch2, "port_id": "transfer" },
                "ordering": "unordered",
                "version": "ics20-1",
                "tags": { "preferred": true, "status": "ACTIVE" }
            }]
        });
        (key, entry)
    }

    let mut map = serde_json::Map::new();
    for (k, v) in [
        pair_entry(&ab.chain1, &ab.chain2, &ab.channel_on_1, &ab.channel_on_2),
        pair_entry(&bc.chain1, &bc.chain2, &bc.channel_on_1, &bc.channel_on_2),
        pair_entry(&cd.chain1, &cd.chain2, &cd.channel_on_1, &cd.channel_on_2),
    ] {
        map.insert(k, v);
    }
    serde_json::Value::Object(map)
}
