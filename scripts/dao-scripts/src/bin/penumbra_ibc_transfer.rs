//! Terp -> Penumbra ICS-20 over transfer/channel-16 (<-> penumbra-1 channel-25).
//!
//! Two jobs, both dry-run by default:
//!   1. a small test transfer from our own key (`--granter`), signed through the
//!      cw-orch-kms Zigner session on a random loopback page (`--broadcast` +
//!      TERP_SCRIPT_BROADCAST=1); the session allows MsgTransfer only.
//!   2. `--fiab-proposal-out <file>` renders the FIAB-SUBDAO resend proposal
//!      (dao_account params format, from 8-fiab-penumbra-liquidity.json) for the
//!      same receiver. It is never broadcast from here; use dao_account.
//!
//! The receiver is always a parameter. Addresses from the pre-0.13.0 Zigner
//! placeholder derivation (penumbra1gmapdg…) are refused: no key owns them.

use anyhow::{Result, bail};
use clap::Parser;
use cw_orch::prelude::*;
use dao_scripts::kms_gate::{self, BroadcastRequest, Scope};
use ibc_proto::cosmos::base::v1beta1::Coin as ProtoCoin;
use ibc_proto::ibc::applications::transfer::v1::MsgTransfer;
use serde_json::{Value, json};

const MSG_TRANSFER: &str = "/ibc.applications.transfer.v1.MsgTransfer";
const DEFAULT_GRANTER: &str = "terp1fxccvvhhy43tvet2ah7jqwq4cwl9k3dxmww052";
const UNSPENDABLE_PREFIX: &str = "penumbra1gmapdg";
const BECH32_CHARSET: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";

#[derive(Parser, Debug)]
#[command(name = "penumbra_ibc_transfer")]
struct Cli {
    /// Penumbra receive address shown by the fixed Zigner (0.13.0+).
    #[arg(long)]
    receiver: String,
    #[arg(long, default_value = DEFAULT_GRANTER)]
    granter: String,
    /// micro-units of uterp (1 TERP = 1000000)
    #[arg(long, default_value_t = 1_000_000)]
    uterp: u128,
    /// micro-units of uthiol; 0 = skip
    #[arg(long, default_value_t = 0)]
    uthiol: u128,
    #[arg(long, default_value = "channel-16")]
    channel: String,
    /// relative timeout from now (minutes)
    #[arg(long, default_value_t = 60)]
    timeout_mins: u64,
    #[arg(long, default_value = "")]
    memo: String,
    #[arg(long)]
    broadcast: bool,
    #[arg(long, default_value = "")]
    chain_id: String,
    #[arg(long, default_value_t = 0)]
    account_index: u32,
    #[arg(long, default_value_t = 900)]
    ttl_secs: u64,
    #[arg(long, default_value = "out/penumbra-test-transfer.json")]
    out: String,
    /// Render the FIAB resend proposal (dao_account params) to this path instead.
    #[arg(long)]
    fiab_proposal_out: Option<String>,
    #[arg(
        long,
        default_value = "params/morocco-1.dao-c/8-fiab-penumbra-liquidity.json"
    )]
    fiab_template: String,
    /// Absolute timeout for the FIAB proposal (unix seconds); must be after execution.
    #[arg(long, default_value_t = 0)]
    fiab_timeout_unix: u64,
    /// Label the rendered proposal as a simulation-only artifact (dummy receiver).
    #[arg(long)]
    simulation_only: bool,
}

/// Syntax + placeholder check. Ownership is checked against the FVK separately.
pub fn check_receiver(addr: &str) -> Result<()> {
    if addr.starts_with(UNSPENDABLE_PREFIX) {
        bail!(
            "refused: {addr:.20}… is the pre-fix placeholder address; read the NEW address from Zigner 0.13.0+"
        );
    }
    let Some(data) = addr.strip_prefix("penumbra1") else {
        bail!("receiver is not a penumbra1… address")
    };
    if !(130..=160).contains(&addr.len()) {
        bail!(
            "receiver length {} is not a Penumbra payment address (expected ~143)",
            addr.len()
        );
    }
    if let Some(c) = data.chars().find(|c| !BECH32_CHARSET.contains(*c)) {
        bail!("receiver has non-bech32 character {c:?}");
    }
    Ok(())
}

fn now_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

fn render_fiab(cli: &Cli, out: &str) -> Result<()> {
    let mut f: Value = serde_json::from_str(&std::fs::read_to_string(&cli.fiab_template)?)?;
    let ts = if cli.fiab_timeout_unix == 0 {
        bail!("--fiab-timeout-unix is required (after the expected execution time)")
    } else {
        cli.fiab_timeout_unix
    };
    if ts * 1_000_000_000 <= now_ns() {
        bail!("--fiab-timeout-unix is in the past");
    }
    let ts_ns = (ts as u128 * 1_000_000_000).to_string();
    let old = "penumbra1gmapdgqxe0s20trdqz0fulurzgk0agyzawp3ymtm4msks4mu9kvhm3ghw3rah3l7g727xuj5vw9jjg596ec2ecprw3hpta05rfx4djrl0y6e4ujj6cvmcf7t93ldcsquzmqf22";
    let p = &mut f["propose"]["propose"];
    p["title"] =
        json!("FIAB-SUBDAO: resend TERP and THIOL to the corrected Penumbra liquidity wallet");
    p["description"] = json!(p["description"].as_str().unwrap_or_default().replace(old, &cli.receiver).replace(
        "Two ICS-20 transfers", "Resend (the earlier destination was a Zigner placeholder address no key owns, zigner#35). Two ICS-20 transfers"));
    for m in p["msgs"].as_array_mut().unwrap() {
        m["ibc"]["transfer"]["to_address"] = json!(cli.receiver);
        m["ibc"]["transfer"]["timeout"]["timestamp"] = json!(ts_ns);
    }
    f["status"] = json!(if cli.simulation_only {
        "SIMULATION ONLY - dummy receiver, DO NOT LAUNCH"
    } else {
        "DRAFT - launch only after the test transfer is confirmed by the user"
    });
    f["timeout_note"] = json!(format!(
        "absolute timestamp {ts} (unix s); must be after execution, refresh before launch"
    ));
    f["receiver"] = json!(cli.receiver);
    f["supersedes"] =
        json!("8-fiab-penumbra-liquidity.json (commit 896fe8e), placeholder receiver");
    kms_gate::write_plan(out, &f)
}

fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();
    kms_gate::load_dotenv_public();
    env_logger::init();
    let cli = Cli::parse();
    check_receiver(&cli.receiver)?;
    if let Some(out) = cli.fiab_proposal_out.clone() {
        return render_fiab(&cli, &out);
    }
    if cli.uterp > 5_000_000 || cli.uthiol > 5_000_000 {
        bail!("refused: this is the small test transfer (max 5 TERP / 5 THIOL)");
    }
    let info = kms_gate::morocco_info();
    let d = kms_gate::query_only(&info)?;
    let mut coins = vec![];
    if cli.uterp > 0 {
        coins.push(("uterp", cli.uterp));
    }
    if cli.uthiol > 0 {
        coins.push(("uthiol", cli.uthiol));
    }
    let timeout_ns = now_ns() + cli.timeout_mins * 60 * 1_000_000_000;
    let mut balances = serde_json::Map::new();
    for (denom, amt) in &coins {
        let bal = d
            .bank_querier()
            .balance(&Addr::unchecked(&cli.granter), Some(denom.to_string()))?;
        let have: u128 = bal
            .first()
            .map(|c| c.amount.to_string().parse::<u128>().unwrap_or(u128::MAX))
            .unwrap_or(0);
        balances.insert(denom.to_string(), json!(have.to_string()));
        if have < *amt {
            bail!("granter has {have}{denom}, needs {amt}");
        }
    }
    let msgs: Vec<MsgTransfer> = coins
        .iter()
        .map(|(denom, amt)| MsgTransfer {
            source_port: "transfer".into(),
            source_channel: cli.channel.clone(),
            token: Some(ProtoCoin {
                denom: denom.to_string(),
                amount: amt.to_string(),
            }),
            sender: cli.granter.clone(),
            receiver: cli.receiver.clone(),
            timeout_height: None,
            timeout_timestamp: timeout_ns,
            memo: cli.memo.clone(),
        })
        .collect();
    kms_gate::write_plan(
        &cli.out,
        &json!({
            "dry_run": !cli.broadcast,
            "chain_id": info.chain_id,
            "sender": cli.granter,
            "receiver": cli.receiver,
            "channel": cli.channel,
            "coins": coins.iter().map(|(d, a)| json!({"denom": d, "amount": a.to_string()})).collect::<Vec<_>>(),
            "sender_balances": balances,
            "timeout_timestamp_ns": timeout_ns.to_string(),
            "session_scope": {"sdk": [MSG_TRANSFER], "ttl_secs": cli.ttl_secs, "page": cw_orch_kms::page_addr().to_string()},
            "simulate": "skipped in dry-run: simulation needs the granter's pubkey; the broadcast path simulates inside commit_tx_any.",
        }),
    )?;
    if !cli.broadcast {
        return Ok(());
    }
    let anys = msgs
        .iter()
        .map(|m| kms_gate::any(MSG_TRANSFER, m))
        .collect();
    let tx = kms_gate::kms_broadcast(
        BroadcastRequest {
            info,
            expected_chain_id: cli.chain_id.clone(),
            granter: cli.granter.clone(),
            purpose: "penumbra-test-transfer".into(),
            account_index: cli.account_index,
            ttl_secs: cli.ttl_secs,
            scope: Scope::Sdk(vec![MSG_TRANSFER.into()]),
            summary: format!(
                "ICS-20 test transfer to {} over {}",
                cli.receiver, cli.channel
            ),
        },
        anys,
        "penumbra test transfer",
    )?;
    println!("tx {tx}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::check_receiver;
    #[test]
    fn refuses_placeholder() {
        assert!(check_receiver("penumbra1gmapdgqxe0s20trdqz0fulurzgk0agyzawp3ymtm4msks4mu9kvhm3ghw3rah3l7g727xuj5vw9jjg596ec2ecprw3hpta05rfx4djrl0y6e4ujj6cvmcf7t93ldcsquzmqf22").is_err());
        assert!(check_receiver("terp1xyz").is_err());
    }
}
