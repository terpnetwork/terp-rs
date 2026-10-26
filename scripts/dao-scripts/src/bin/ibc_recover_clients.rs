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

use anyhow::{anyhow, bail, Result};
use base64::Engine;
use clap::Parser;
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
    #[arg(long, default_value = "Recover expired 07-tendermint clients on morocco-1 to fresh substitute clients created by ibc_create_clients.")]
    summary: String,
    /// Initial deposit in uterp.
    #[arg(long, default_value_t = 5_000_000_000)]
    deposit: u128,
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
            let (a, b) = p.split_once(':').ok_or_else(|| anyhow!("pair {p} is not subject:substitute"))?;
            Ok((a.to_string(), b.to_string()))
        })
        .collect()
}

fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider().install_default().ok();
    dao_scripts::kms_gate::load_dotenv_public();
    env_logger::init();
    let cli = Cli::parse();
    let pairs = parse_pairs(&cli.pairs)?;
    let morocco = kms_gate::morocco_info();
    let authority = kms_gate::module_address("gov", "terp")?;
    let proposer = if cli.granter.is_empty() { "<cw-orch-kms granter>".to_string() } else { cli.granter.clone() };
    let any = ibc_clients::recover_proposal_any(&pairs, &authority, &proposer, (cli.deposit, "uterp"), &cli.title, &cli.summary)?;
    // Live client status, read-only. A stale or unreachable node is reported, not fatal.
    let status = match kms_gate::query_only(&morocco) {
        Ok(d) => {
            let ibc = cw_orch::daemon::queriers::Ibc::new_async(d.channel());
            let mut out = Vec::new();
            for (s, t) in &pairs {
                for id in [s, t] {
                    let r = d.rt_handle.block_on(ibc._client_state(id));
                    out.push(serde_json::json!({"client_id": id, "found": r.is_ok(), "error": r.err().map(|e| e.to_string())}));
                }
            }
            serde_json::json!(out)
        }
        Err(e) => serde_json::json!({"error": e.to_string()}),
    };
    let plan = serde_json::json!({
        "dry_run": !cli.broadcast,
        "chain_id": morocco.chain_id,
        "gov_authority": authority,
        "proposer": proposer,
        "pairs": pairs,
        "client_status": status,
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
