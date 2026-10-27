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
    /// osmosis, juno, akash, secret, atomone, penumbra. Repeat or comma-separate. Default: all.
    #[arg(long, value_delimiter = ',')]
    counterparty: Vec<String>,
    #[arg(long, env = "PENUMBRA_RPC", default_value = "https://rpc.penumbra.fi")]
    penumbra_rpc: String,
    /// penumbra-1 unbonding: 120960 blocks; live osmosis clients use 604800s.
    #[arg(long, default_value_t = 604_800)]
    penumbra_unbonding_secs: u64,
    /// Default max clock drift for fresh clients (1..=3600s). Substitutes copy the subject's.
    #[arg(long, default_value_t = ibc_clients::DEFAULT_MAX_CLOCK_DRIFT_SECS)]
    max_clock_drift_secs: u64,
    /// Per-client spec, repeatable: NAME[:key=val;key=val]. Keys: drift=SECS,
    /// trusting=SECS, substitute_for=07-tendermint-N (copy all match-required
    /// fields from that live morocco-1 client). Same NAME may repeat.
    #[arg(long = "client")]
    clients: Vec<String>,
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
        // cw-orch-networks has no secret-4 entry and AKASH_MAINNET says "akash-1";
        // reuse the akash network info with the live chain ids.
        "akash" | "secret" => {
            let (chain_id, denom, grpc, prefix) = match name {
                "akash" => ("akashnet-2", "uakt", "https://akash.lavenderfive.com:443", "akash"),
                _ => ("secret-4", "uscrt", "https://secretnetwork.lavenderfive.com:443", "secret"),
            };
            let mut i: ChainInfoOwned = cw_orch::daemon::networks::AKASH_MAINNET.into();
            i.chain_id = chain_id.into();
            i.gas_denom = denom.into();
            i.grpc_urls = vec![grpc.into()];
            i.network_info.chain_name = name.into();
            i.network_info.pub_address_prefix = prefix.into();
            i
        }
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
        "store": format!("{:?}", a.store),
    })
}

#[derive(Debug, Default)]
struct Spec {
    name: String,
    drift: Option<u64>,
    trusting: Option<u64>,
    substitute_for: Option<String>,
}

fn parse_spec(raw: &str) -> Result<Spec> {
    let (name, rest) = raw.split_once(':').unwrap_or((raw, ""));
    let mut s = Spec { name: name.trim().to_string(), ..Default::default() };
    for kv in rest.split(';').filter(|x| !x.trim().is_empty()) {
        let (k, v) = kv.split_once('=').ok_or_else(|| anyhow!("bad client option {kv}"))?;
        match k.trim() {
            "drift" => s.drift = Some(v.trim().parse()?),
            "trusting" => s.trusting = Some(v.trim().parse()?),
            "substitute_for" => {
                if !ibc_clients::is_tm_client_id(v.trim()) {
                    bail!("substitute_for {v} is not a 07-tendermint id");
                }
                s.substitute_for = Some(v.trim().into())
            }
            other => bail!("unknown client option {other}"),
        }
    }
    Ok(s)
}

fn live_client_state(info: &ChainInfoOwned, id: &str) -> Result<ibc_clients::TmClientState> {
    use cosmos_sdk_proto::prost::Message;
    let d = kms_gate::query_only(info)?;
    let ibc = cw_orch::daemon::queriers::Ibc::new_async(d.channel());
    let r = d.rt_handle.block_on(ibc._client_state(id)).map_err(|e| anyhow!("{id}: {e}"))?;
    let any = r.client_state.ok_or_else(|| anyhow!("{id}: no client state"))?;
    if any.type_url != ibc_clients::TM_CLIENT_STATE {
        bail!("{id}: not a tendermint client ({})", any.type_url);
    }
    Ok(ibc_clients::TmClientState::decode(any.value.as_slice())?)
}

fn build(cli: &Cli, morocco: &ChainInfoOwned, spec: &Spec, signer: &str) -> Result<(cosmrs::Any, serde_json::Value)> {
    let a = anchor_for(cli, &spec.name)?;
    let (default_t, mut note) = ibc_clients::default_trusting_secs(a.unbonding_secs);
    let trusting = spec.trusting.unwrap_or(default_t);
    if spec.trusting.is_some() {
        note = Some("trusting override".into());
    }
    let (cs, purpose, matched) = match &spec.substitute_for {
        Some(subj_id) => {
            if spec.drift.is_some() {
                bail!("{}: drift is copied from {subj_id}; do not pass drift with substitute_for", spec.name);
            }
            let subj = live_client_state(morocco, subj_id)?;
            let cs = ibc_clients::substitute_state(&subj, &a, trusting)?;
            let m = ibc_clients::is_matching_client_state(&subj, &cs);
            (cs, format!("recovery substitute for {subj_id}"), Some(m))
        }
        None => {
            let drift = spec.drift.unwrap_or(cli.max_clock_drift_secs);
            (ibc_clients::client_state_with(&a, trusting, drift)?, "new route client".to_string(), None)
        }
    };
    let any = ibc_clients::create_client_any_from(&cs, &a, signer);
    let row = serde_json::json!({
        "counterparty": spec.name,
        "purpose": purpose,
        "on": morocco.chain_id,
        "anchor": summary(&a),
        "trusting_secs": cs.trusting_period.as_ref().map(|d| d.seconds),
        "unbonding_secs": cs.unbonding_period.as_ref().map(|d| d.seconds),
        "max_clock_drift_secs": cs.max_clock_drift.as_ref().map(|d| d.seconds),
        "trust_level": cs.trust_level.as_ref().map(|f| format!("{}/{}", f.numerator, f.denominator)),
        "note": note,
        "matches_subject": matched,
        "msg_bytes": any.value.len(),
        "msg": {"type_url": any.type_url, "value_b64": base64::engine::general_purpose::STANDARD.encode(&any.value)},
    });
    Ok((any, row))
}

fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider().install_default().ok();
    dao_scripts::kms_gate::load_dotenv_public();
    env_logger::init();
    let cli = Cli::parse();
    ibc_clients::check_drift(cli.max_clock_drift_secs)?;
    let mut specs: Vec<Spec> = cli.clients.iter().map(|c| parse_spec(c)).collect::<Result<_>>()?;
    specs.extend(cli.counterparty.iter().map(|n| Spec { name: n.clone(), ..Default::default() }));
    if specs.is_empty() {
        specs = ["osmosis", "juno", "akash", "secret", "atomone", "penumbra"]
            .map(|n| Spec { name: n.into(), ..Default::default() })
            .into_iter()
            .collect();
    }
    let names: Vec<String> = specs.iter().map(|s| s.name.clone()).collect();
    let morocco = kms_gate::morocco_info();
    let signer = if cli.granter.is_empty() { "<cw-orch-kms granter>".to_string() } else { cli.granter.clone() };
    let mut rows = Vec::new();
    let mut msgs = Vec::new();
    for spec in &specs {
        match build(&cli, &morocco, spec, &signer) {
            Ok((any, row)) => {
                rows.push(row);
                msgs.push(any);
            }
            Err(e) => rows.push(serde_json::json!({"counterparty": spec.name, "error": e.to_string()})),
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
