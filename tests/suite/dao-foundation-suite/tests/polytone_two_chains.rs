//! Polytone note on one terp network and voice on another.
//!
//! The note's first `Execute` with an empty message list creates the remote
//! proxy account on the voice chain. The relayer opens an unordered
//! `polytone-1` channel between `wasm.{note}` and `wasm.{voice}`.
//!
//! ```text
//! TERP_IMAGE_REPO=terp-core TERP_IMAGE_VERSION=v630-arm64 ICT_POLYTONE=1 \
//!   POLYTONE_ARTIFACTS=/path/to/wasm \
//!   cargo test -p dao-foundation-suite --test polytone_two_chains -- --ignored --nocapture
//! ```
//!
//! `POLYTONE_ARTIFACTS` must contain `polytone_note.wasm`, `polytone_voice.wasm`,
//! and `polytone_proxy.wasm`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ict_rs::chain::cosmos::CosmosChain;
use ict_rs::chain::{Chain, GenesisStyle, TestContext};
use ict_rs::cosmwasm::CosmWasmExt;
use ict_rs::ibc::{ChannelOptions, ChannelOrdering, ClientOptions};
use ict_rs::relayer::{build_relayer, Relayer, RelayerType};
use ict_rs::runtime::DockerConfig;
use ict_rs::runtime::RuntimeBackend;
use ict_rs::testing::TestEnv;
use ict_rs::tx::WalletAmount;
use serde_json::json;

const NETWORK: &str = "ict-polytone";
const WASM: [&str; 3] = [
    "polytone_note.wasm",
    "polytone_voice.wasm",
    "polytone_proxy.wasm",
];

#[tokio::test]
#[ignore = "docker: TERP_IMAGE_REPO=terp-core TERP_IMAGE_VERSION=v630-arm64 ICT_POLYTONE=1"]
async fn polytone_remote_account_on_two_terp_networks() {
    if std::env::var("ICT_POLYTONE").ok().as_deref() != Some("1") {
        panic!(
            "set ICT_POLYTONE=1 TERP_IMAGE_REPO=terp-core TERP_IMAGE_VERSION=v630-arm64 \
             POLYTONE_ARTIFACTS=... before running this ignored test"
        );
    }
    if std::env::var("ICT_MOCK").ok().as_deref() == Some("1") {
        panic!("ICT_MOCK does not open a polytone channel");
    }

    let artifacts = artifact_dir();
    let backend = ict_rs::runtime::docker::DockerBackend::new(DockerConfig::default())
        .await
        .expect("docker");
    let runtime: Arc<dyn RuntimeBackend> = Arc::new(backend);
    let ctx = TestContext {
        test_name: "polytone".into(),
        network_id: NETWORK.into(),
    };

    let mut chain_a = CosmosChain::new(terp_config("polytone-a"), 1, 0, runtime.clone());
    let mut chain_b = CosmosChain::new(terp_config("polytone-b"), 1, 0, runtime.clone());
    chain_a.initialize(&ctx).await.expect("init chain a");
    chain_b.initialize(&ctx).await.expect("init chain b");
    chain_a.start(&[]).await.expect("start chain a");
    chain_b.start(&[]).await.expect("start chain b");

    let result = connect(&chain_a, &chain_b, runtime, &artifacts).await;
    let stop_a = chain_a.stop().await;
    let stop_b = chain_b.stop().await;
    result.expect("polytone two-network exercise");
    stop_a.expect("stop chain a");
    stop_b.expect("stop chain b");
}

async fn connect(
    chain_a: &CosmosChain,
    chain_b: &CosmosChain,
    runtime: Arc<dyn RuntimeBackend>,
    artifacts: &Path,
) -> Result<(), String> {
    let relayer = build_relayer(
        RelayerType::CosmosRly,
        runtime,
        "polytone",
        NETWORK,
    )
    .await
    .map_err(|err| err.to_string())?;

    fund_member(chain_a).await?;
    fund_member(chain_b).await?;
    let (note_id, voice_id, proxy_id) = store_polytone(chain_a, chain_b, artifacts).await?;
    let note = instantiate_note(chain_a, note_id).await?;
    let voice = instantiate_voice(chain_b, voice_id, proxy_id).await?;
    println!("note {note} on polytone-a; voice {voice} on polytone-b");

    configure_relayer(relayer.as_ref(), chain_a, chain_b).await?;
    relayer
        .create_channel(
            "polytone",
            &ChannelOptions {
                src_port: format!("wasm.{note}"),
                dst_port: format!("wasm.{voice}"),
                ordering: ChannelOrdering::Unordered,
                version: "polytone-1".into(),
            },
        )
        .await
        .map_err(|err| format!("polytone channel: {err}"))?;
    relayer
        .start(&["polytone"])
        .await
        .map_err(|err| err.to_string())?;

    let member = chain_a
        .key_address("member")
        .await
        .map_err(|err| err.to_string())?;
    chain_a
        .execute_contract(
            "member",
            &note,
            &json!({
                "execute": {
                    "msgs": [],
                    "callback": null,
                    "timeout_seconds": "300"
                }
            })
            .to_string(),
            None,
        )
        .await
        .map_err(|err| format!("create remote account: {err}"))?;
    tokio::time::sleep(std::time::Duration::from_secs(20)).await;

    let remote = chain_a
        .query_contract(
            &note,
            &json!({ "remote_address": { "local_address": member } }).to_string(),
        )
        .await
        .map_err(|err| format!("remote address query: {err}"))?;
    let text = remote.to_string();
    assert!(
        text.contains("terp1"),
        "voice chain did not create a proxy account: {text}"
    );
    relayer.stop().await.map_err(|err| err.to_string())?;
    Ok(())
}

async fn configure_relayer(
    relayer: &dyn Relayer,
    chain_a: &CosmosChain,
    chain_b: &CosmosChain,
) -> Result<(), String> {
    for chain in [chain_a, chain_b] {
        let chain_id = chain.chain_id();
        let key = format!("relayer-rly-{chain_id}");
        relayer
            .add_chain_configuration(chain.config(), &key, chain.rpc_address(), chain.grpc_address())
            .await
            .map_err(|err| format!("relayer config {chain_id}: {err}"))?;
        let wallet = relayer
            .add_key(chain_id, &key)
            .await
            .map_err(|err| format!("relayer key {chain_id}: {err}"))?;
        let address = wallet.formatted_address();
        chain
            .send_funds(
                "validator",
                &WalletAmount {
                    address,
                    denom: "uterp".into(),
                    amount: 100_000_000,
                },
            )
            .await
            .map_err(|err| format!("fund relayer on {chain_id}: {err}"))?;
    }
    relayer
        .generate_path(chain_a.chain_id(), chain_b.chain_id(), "polytone")
        .await
        .map_err(|err| format!("path: {err}"))?;
    relayer
        .create_clients("polytone", &ClientOptions::default())
        .await
        .map_err(|err| format!("clients: {err}"))?;
    relayer
        .create_connections("polytone")
        .await
        .map_err(|err| format!("connections: {err}"))?;
    Ok(())
}

async fn fund_member(chain: &CosmosChain) -> Result<(), String> {
    chain
        .create_key("member")
        .await
        .map_err(|err| err.to_string())?;
    let member = chain
        .key_address("member")
        .await
        .map_err(|err| err.to_string())?;
    chain
        .send_funds(
            "validator",
            &WalletAmount {
                address: member,
                denom: "uterp".into(),
                amount: 50_000_000_000,
            },
        )
        .await
        .map_err(|err| err.to_string())?;
    Ok(())
}

async fn store_polytone(
    chain_a: &CosmosChain,
    chain_b: &CosmosChain,
    artifacts: &Path,
) -> Result<(u64, u64, u64), String> {
    for name in WASM {
        let host = artifacts.join(name);
        let dest = format!("/tmp/{name}");
        chain_a
            .validators()
            .first()
            .ok_or("chain a has no validator")?
            .copy_file_from_host(&host, &dest)
            .await
            .map_err(|err| format!("copy {name} to a: {err}"))?;
        chain_b
            .validators()
            .first()
            .ok_or("chain b has no validator")?
            .copy_file_from_host(&host, &dest)
            .await
            .map_err(|err| format!("copy {name} to b: {err}"))?;
    }
    let note = store(chain_a, "/tmp/polytone_note.wasm").await?;
    let voice = store(chain_b, "/tmp/polytone_voice.wasm").await?;
    let proxy = store(chain_b, "/tmp/polytone_proxy.wasm").await?;
    Ok((note, voice, proxy))
}

async fn instantiate_note(chain: &CosmosChain, code_id: u64) -> Result<String, String> {
    chain
        .instantiate_contract(
            "member",
            &code_id.to_string(),
            &json!({ "pair": null, "block_max_gas": "100000000" }).to_string(),
            "polytone-note",
            None,
        )
        .await
        .map_err(|err| format!("instantiate note: {err}"))
}

async fn instantiate_voice(
    chain: &CosmosChain,
    voice_id: u64,
    proxy_id: u64,
) -> Result<String, String> {
    chain
        .instantiate_contract(
            "member",
            &voice_id.to_string(),
            &json!({
                "proxy_code_id": proxy_id.to_string(),
                "block_max_gas": "100000000"
            })
            .to_string(),
            "polytone-voice",
            None,
        )
        .await
        .map_err(|err| format!("instantiate voice: {err}"))
}

async fn store(chain: &CosmosChain, path: &str) -> Result<u64, String> {
    let before = code_ids(chain).await?;
    let opts = chain.default_tx_opts().from("member").gas_adjustment(2.5);
    let out = chain
        .chain_exec_tx_with(&["tx", "wasm", "store", path], opts)
        .await
        .map_err(|err| format!("store {path}: {err}"))?;
    if out.exit_code != 0 {
        return Err(format!("store {path}: {}", out.stderr_str()));
    }
    let after = code_ids(chain).await?;
    let id = after
        .into_iter()
        .find(|id| !before.contains(id))
        .ok_or_else(|| format!("no new code id after {path}"))?;
    id.parse().map_err(|err| format!("code id {id}: {err}"))
}

async fn code_ids(chain: &CosmosChain) -> Result<Vec<String>, String> {
    let out = chain
        .chain_exec(&["query", "wasm", "list-code", "--output", "json"])
        .await
        .map_err(|err| err.to_string())?;
    if out.exit_code != 0 {
        return Err(out.stderr_str());
    }
    let value: serde_json::Value = serde_json::from_str(out.stdout_str().trim())
        .map_err(|err| format!("list-code: {err}"))?;
    Ok(value
        .get("code_infos")
        .and_then(|rows| rows.as_array())
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    row.get("code_id").and_then(|id| {
                        id.as_str()
                            .map(str::to_string)
                            .or_else(|| id.as_u64().map(|n| n.to_string()))
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

fn terp_config(chain_id: &str) -> ict_rs::chain::ChainConfig {
    let mut cfg = TestEnv::terp_config();
    cfg.chain_id = chain_id.into();
    cfg.gas_prices = "0uterp".into();
    cfg.gas_adjustment = 2.5;
    cfg.block_time = "500ms".into();
    cfg.genesis_style = GenesisStyle::Legacy;
    cfg.config_file_overrides.insert(
        "config/config.toml".into(),
        json!({
            "rpc": { "max_body_bytes": 104_857_600 },
            "mempool": { "max_tx_bytes": 104_857_600 }
        }),
    );
    cfg.modify_genesis = Some(Box::new(|_cfg, raw| {
        let mut genesis: serde_json::Value = serde_json::from_slice(&raw)
            .map_err(|err| ict_rs::error::IctError::Config(format!("genesis json: {err}")))?;
        if let Some(block) = genesis.pointer_mut("/consensus_params/block") {
            block["max_gas"] = json!("-1");
            block["max_bytes"] = json!("104857600");
        }
        if let Some(params) = genesis.pointer_mut("/app_state/wasm/params") {
            params["code_upload_access"] = json!({"permission": "Everybody", "addresses": []});
            params["instantiate_default_permission"] = json!("Everybody");
        }
        serde_json::to_vec(&genesis)
            .map_err(|err| ict_rs::error::IctError::Config(format!("genesis encode: {err}")))
    }));
    cfg
}

fn artifact_dir() -> PathBuf {
    let dir = std::env::var("POLYTONE_ARTIFACTS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../polytone/artifacts")
        });
    for name in WASM {
        let path = dir.join(name);
        if !path.is_file() {
            panic!("missing {} (set POLYTONE_ARTIFACTS)", path.display());
        }
    }
    dir
}
