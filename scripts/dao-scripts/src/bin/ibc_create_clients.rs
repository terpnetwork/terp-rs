//! Create 07-tendermint clients on morocco-1 for its counterparties.
//!
//! cw-orch runtime: headers come from cw-orch's read-only gRPC daemon (cosmos
//! chains) or CometBFT RPC (Penumbra); `MsgCreateClient` is broadcast through
//! the cw-orch Daemon with cw-orch-kms as sender (MsgExec for the granter).
//! Default is dry-run: build, print, write `out/ibc_create_clients.json`.
//!
//! ```sh
//! cargo run -p dao-scripts --bin ibc_create_clients -- --counterparty penumbra
//! TERP_SCRIPT_BROADCAST=1 cargo run -p dao-scripts --bin ibc_create_clients -- \
//!   --counterparty penumbra --broadcast --chain-id morocco-1
//! ```
//!
//! Not done here: the counterparty-side client of morocco-1 and the
//! connection/channel handshake. Those are txs on the counterparty chain
//! (Penumbra: IbcRelay actions signed by a Penumbra wallet) and are printed
//! as blockers.

use anyhow::{anyhow, bail, Result};
use base64::Engine;
use clap::Parser;
use cw_orch::environment::ChainInfoOwned;
use dao_scripts::ibc_clients::{self, Anchor, MSG_CREATE_CLIENT};
use dao_scripts::kms_gate::{self, BroadcastRequest, Scope};

#[derive(Parser, Debug)]
#[command(name = "ibc_create_clients")]
struct Cli {
    /// osmosis, juno, atomone, penumbra. Repeat or comma-separate. Default: all.
    #[arg(long, value_delimiter = ',')]
    counterparty: Vec<String>,
    #[arg(long, env = "PENUMBRA_RPC", default_value = "https://rpc.penumbra.fi")]
    penumbra_rpc: String,
    /// penumbra-1 unbonding: 120960 blocks; live osmosis clients use 604800s.
    #[arg(long, default_value_t = 604_800)]
    penumbra_unbonding_secs: u64,
    #[arg(long, default_value_t = 7235)]
    max_clock_drift_secs: u64,
    /// Also read morocco-1 and print the counterparty-side client (not broadcast).
    #[arg(long)]
    counterparty_side: bool,
    #[arg(long)]
    broadcast: bool,
    /// Required with --broadcast; must equal the configured morocco chain.
    #[arg(long, default_value = "")]
    chain_id: String,
    #[arg(long, default_value = "")]
    granter: String,
    #[arg(long, default_value_t = 0)]
    account_index: u32,
    #[arg(long, default_value_t = 1800)]
    ttl_secs: u64,
    #[arg(long, default_value = "out/ibc_create_clients.json")]
    out: String,
}

fn cosmos_info(name: &str) -> Result<ChainInfoOwned> {
    use cw_orch::daemon::networks::{ATOMEONE_MAINNET, JUNO_1, OSMOSIS_1};
    let mut info: ChainInfoOwned = match name {
        "osmosis" => OSMOSIS_1.into(),
        "juno" => JUNO_1.into(),
        "atomone" => ATOMEONE_MAINNET.into(),
        other => bail!("unknown counterparty {other}"),
    };
    if let Ok(grpc) = std::env::var(format!("{}_GRPC", name.to_uppercase())) {
        info.grpc_urls = vec![grpc];
    }
    Ok(info)
}

fn anchor_for(cli: &Cli, name: &str) -> Result<Anchor> {
    match name {
        "penumbra" => ibc_clients::penumbra_anchor(&cli.penumbra_rpc, "penumbra-1", cli.penumbra_unbonding_secs),
        other => ibc_clients::cosmos_anchor(&cosmos_info(other)?),
    }
}

fn summary(a: &Anchor) -> serde_json::Value {
    serde_json::json!({
        "chain_id": a.chain_id,
        "height": a.height,
        "time_unix": a.time_unix,
        "unbonding_secs": a.unbonding_secs,
        "trusting_secs": ibc_clients::trusting_secs(a.unbonding_secs),
        "store": format!("{:?}", a.store),
    })
}

fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider().install_default().ok();
    dao_scripts::kms_gate::load_dotenv_public();
    env_logger::init();
    let cli = Cli::parse();
    let names: Vec<String> = if cli.counterparty.is_empty() {
        ["osmosis", "juno", "atomone", "penumbra"].map(String::from).to_vec()
    } else {
        cli.counterparty.clone()
    };
    let morocco = kms_gate::morocco_info();
    let signer = if cli.granter.is_empty() { "<cw-orch-kms granter>".to_string() } else { cli.granter.clone() };
    let mut rows = Vec::new();
    let mut msgs = Vec::new();
    for name in &names {
        match anchor_for(&cli, name) {
            Ok(a) => {
                let any = ibc_clients::create_client_any(&a, &signer, cli.max_clock_drift_secs)?;
                rows.push(serde_json::json!({
                    "counterparty": name,
                    "on": morocco.chain_id,
                    "anchor": summary(&a),
                    "msg": {"type_url": any.type_url, "value_b64": base64::engine::general_purpose::STANDARD.encode(&any.value)},
                }));
                msgs.push(any);
            }
            Err(e) => rows.push(serde_json::json!({"counterparty": name, "error": e.to_string()})),
        }
    }
    let counterparty_side = if cli.counterparty_side {
        match ibc_clients::cosmos_anchor(&morocco) {
            Ok(a) => serde_json::json!({"morocco_anchor": summary(&a),
                "note": "Submit on each counterparty with that chain's signer. Penumbra: IbcRelay action from a Penumbra wallet/relayer, fees in UM. Not signable by cw-orch-kms."}),
            Err(e) => serde_json::json!({"error": e.to_string()}),
        }
    } else {
        serde_json::Value::Null
    };
    let plan = serde_json::json!({
        "dry_run": !cli.broadcast,
        "chain_id": morocco.chain_id,
        "grpc": morocco.grpc_urls,
        "session_scope": {"allow_sdk": [MSG_CREATE_CLIENT], "ttl_secs": cli.ttl_secs, "challenge": "cw-orch-granter:v2"},
        "clients": rows,
        "counterparty_side": counterparty_side,
        "handshake": "blocked: cw-orch-interchain opens connections/channels through a relayer (Starship/hermes), not a cw-orch-kms sender. Needs a relayer session plan (enable_relay_sessions) per chain.",
        "simulate": "skipped in dry-run: simulation needs the granter account; the broadcast path simulates inside commit_tx_any.",
    });
    if let Some(dir) = std::path::Path::new(&cli.out).parent() {
        std::fs::create_dir_all(dir).ok();
    }
    std::fs::write(&cli.out, serde_json::to_string_pretty(&plan)?)?;
    println!("{}", serde_json::to_string_pretty(&plan)?);
    if !cli.broadcast {
        return Ok(());
    }
    if msgs.len() != names.len() {
        bail!("broadcast refused: not every client was built");
    }
    if cli.granter.is_empty() {
        bail!("broadcast refused: pass --granter (the v2 challenge page confirms it) so MsgCreateClient.signer is the granter");
    }
    let tx = kms_gate::kms_broadcast(
        BroadcastRequest {
            info: morocco,
            expected_chain_id: cli.chain_id.clone(),
            granter: cli.granter.clone(),
            purpose: "ibc-create-clients".into(),
            account_index: cli.account_index,
            ttl_secs: cli.ttl_secs,
            scope: Scope::Sdk(vec![MSG_CREATE_CLIENT.into()]),
            summary: format!("create {} 07-tendermint clients: {}", msgs.len(), names.join(",")),
        },
        msgs,
        "ibc_create_clients",
    )
    .map_err(|e| anyhow!(e))?;
    println!("tx {tx}");
    Ok(())
}
