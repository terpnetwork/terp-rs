//! Gov proposal recovering expired morocco-1 IBC clients.
//!
//! cw-orch runtime: reads client states through cw-orch's read-only gRPC
//! daemon, builds `MsgSubmitProposal{MsgRecoverClient..}` and, with
//! `--broadcast`, submits it through the cw-orch Daemon with cw-orch-kms as
//! sender (session allows only `/cosmos.gov.v1.MsgSubmitProposal`). The
//! proposer is the granter. Default is dry-run.
//!
//! ```sh
//! cargo run -p dao-scripts --bin ibc_recover_clients -- \
//!   --pair 07-tendermint-12:07-tendermint-40 --granter terp1...
//! ```

use anyhow::{Result, anyhow, bail};
use base64::Engine;
use clap::Parser;
use cosmos_sdk_proto::prost::Message;
use dao_scripts::ibc_clients::{self, MSG_SUBMIT_PROPOSAL};
use dao_scripts::kms_gate::{self, BroadcastRequest, Scope};

#[derive(Parser, Debug)]
#[command(name = "ibc_recover_clients")]
struct Cli {
    /// subject:substitute client ids. Repeat for several.
    #[arg(long = "pair", required = true)]
    pairs: Vec<String>,
    #[arg(long, default_value = "Recover expired IBC clients")]
    title: String,
    #[arg(
        long,
        default_value = "Recover expired 07-tendermint clients on morocco-1 to fresh substitute clients created by ibc_create_clients."
    )]
    summary: String,
    /// Initial deposit in uterp.
    #[arg(long, default_value_t = 10_000_000_000)]
    deposit: u128,
    /// Each substitute must stay Active at least this long (voting period 4d + 1d margin).
    #[arg(long, default_value_t = 432_000)]
    min_remaining_secs: i64,
    #[arg(long)]
    broadcast: bool,
    #[arg(long, default_value = "")]
    chain_id: String,
    #[arg(long, default_value = "")]
    granter: String,
    #[arg(long, default_value_t = 0)]
    account_index: u32,
    #[arg(long, default_value_t = 1800)]
    ttl_secs: u64,
    #[arg(long, default_value = "out/ibc_recover_clients.json")]
    out: String,
}

fn parse_pairs(raw: &[String]) -> Result<Vec<(String, String)>> {
    raw.iter()
        .map(|p| {
            let (a, b) = p
                .split_once(':')
                .ok_or_else(|| anyhow!("pair {p} is not subject:substitute"))?;
            Ok((a.to_string(), b.to_string()))
        })
        .collect()
}

fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();
    dao_scripts::kms_gate::load_dotenv_public();
    env_logger::init();
    let cli = Cli::parse();
    let pairs = parse_pairs(&cli.pairs)?;
    let morocco = kms_gate::morocco_info();
    let authority = kms_gate::module_address("gov", "terp")?;
    let proposer = if cli.granter.is_empty() {
        "<cw-orch-kms granter>".to_string()
    } else {
        cli.granter.clone()
    };
    let any = ibc_clients::recover_proposal_any(
        &pairs,
        &authority,
        &proposer,
        (cli.deposit, "uterp"),
        &cli.title,
        &cli.summary,
    )?;
    // Live checks, read-only: subject/substitute must satisfy ibc-go v11
    // IsMatchingClientState and each substitute must be Active with enough
    // trusting time left to outlast the vote. Broadcast refuses on any failure.
    let mut all_ok = true;
    let status = match kms_gate::query_only(&morocco) {
        Ok(d) => {
            let ibc = cw_orch::daemon::queriers::Ibc::new_async(d.channel());
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs() as i64;
            let cs_of = |id: &str| -> Result<ibc_clients::TmClientState> {
                let r = d
                    .rt_handle
                    .block_on(ibc._client_state(id))
                    .map_err(|e| anyhow!("{id}: {e}"))?;
                let a = r
                    .client_state
                    .ok_or_else(|| anyhow!("{id}: no client state"))?;
                Ok(ibc_clients::TmClientState::decode(a.value.as_slice())?)
            };
            let status_of = |id: &str| {
                d.rt_handle
                    .block_on(ibc._client_status(id))
                    .map(|r| r.status)
                    .unwrap_or_else(|e| format!("error: {e}"))
            };
            let mut out = Vec::new();
            for (s, t) in &pairs {
                let subj = cs_of(s);
                let sub = cs_of(t);
                let sub_status = status_of(t);
                let remaining = sub.as_ref().ok().and_then(|c| {
                    let lh = c.latest_height?;
                    let cons = d.rt_handle.block_on(ibc._consensus_states(t)).ok()?;
                    let e = cons.consensus_states.into_iter().find(|e| {
                        e.height.as_ref().map(|h| h.revision_height) == Some(lh.revision_height)
                    })?;
                    let cs =
                        ibc_clients::TmConsensusState::decode(e.consensus_state?.value.as_slice())
                            .ok()?;
                    let ts = cs.timestamp?.seconds;
                    Some(ts + c.trusting_period.as_ref()?.seconds - now)
                });
                let matching = match (&subj, &sub) {
                    (Ok(a), Ok(b)) => Some(ibc_clients::is_matching_client_state(a, b)),
                    _ => None,
                };
                let ok = sub_status == "Active"
                    && remaining.is_some_and(|r| r >= cli.min_remaining_secs)
                    && matching == Some(true);
                all_ok &= ok;
                out.push(serde_json::json!({
                    "subject": s, "subject_status": status_of(s),
                    "subject_error": subj.as_ref().err().map(|e| e.to_string()),
                    "substitute": t, "substitute_status": sub_status,
                    "substitute_error": sub.as_ref().err().map(|e| e.to_string()),
                    "substitute_trusting_remaining_secs": remaining,
                    "is_matching_client_state": matching,
                    "ok": ok,
                }));
            }
            serde_json::json!(out)
        }
        Err(e) => {
            all_ok = false;
            serde_json::json!({"error": e.to_string()})
        }
    };
    let plan = serde_json::json!({
        "dry_run": !cli.broadcast,
        "chain_id": morocco.chain_id,
        "gov_authority": authority,
        "proposer": proposer,
        "pairs": pairs,
        "client_status": status,
        "checks_ok": all_ok,
        "deposit_uterp": cli.deposit.to_string(),
        "session_scope": {"allow_sdk": [MSG_SUBMIT_PROPOSAL], "ttl_secs": cli.ttl_secs, "challenge": "cw-orch-granter:v2"},
        "msg": {"type_url": any.type_url, "value_b64": base64::engine::general_purpose::STANDARD.encode(&any.value)},
    });
    if let Some(dir) = std::path::Path::new(&cli.out).parent() {
        std::fs::create_dir_all(dir).ok();
    }
    std::fs::write(&cli.out, serde_json::to_string_pretty(&plan)?)?;
    println!("{}", serde_json::to_string_pretty(&plan)?);
    if !cli.broadcast {
        return Ok(());
    }
    if !all_ok {
        bail!(
            "broadcast refused: a substitute is not Active with >= {}s left, or does not match its subject",
            cli.min_remaining_secs
        );
    }
    if cli.granter.is_empty() {
        bail!("broadcast refused: pass --granter so the proposer is the granter");
    }
    let tx = kms_gate::kms_broadcast(
        BroadcastRequest {
            info: morocco,
            expected_chain_id: cli.chain_id.clone(),
            granter: cli.granter.clone(),
            purpose: "ibc-recover-clients".into(),
            account_index: cli.account_index,
            ttl_secs: cli.ttl_secs,
            scope: Scope::Sdk(vec![MSG_SUBMIT_PROPOSAL.into()]),
            summary: format!("gov proposal: recover {} client(s)", pairs.len()),
        },
        vec![any],
        "ibc_recover_clients",
    )?;
    println!("tx {tx}");
    Ok(())
}
