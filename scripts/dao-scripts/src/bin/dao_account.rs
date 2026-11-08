//! subDAO proposal runner: a single-member, threshold-1 DAO-DAO subDAO
//! proposes, votes and (when no veto timelock applies) executes its own
//! proposal in ONE tx through a cw-orch-kms session. Input is a prepared file
//! from `params/morocco-1.dao-c/` (keys `dao`, `proposal_module`, `signer`, `propose`).
//!
//! Safety: `--granter` is pinned and must equal the file's `signer` and be a
//! cw4 member of the DAO; the approval page refuses any other signer before
//! the grant is broadcast. Scope: the proposal module only, keys
//! propose/vote/execute, calls = messages + 1. kms_broadcast self-revokes and
//! asserts nothing is left. Default is a dry-run that only writes the plan.
use anyhow::{Result, anyhow, bail};
use clap::Parser;
use cw_orch::environment::WasmQuerier;
use cw_orch::prelude::*;
use cw_orch_kms::ScopedCall;
use dao_scripts::kms_gate::{self, BroadcastRequest, Scope};
use serde_json::{Value, json};

#[derive(Parser)]
struct Cli {
    #[arg(long)]
    file: String,
    #[arg(long)]
    granter: String,
    /// full = propose+vote+execute; propose-vote = no execute (veto timelock); execute = --proposal-id only
    #[arg(long, default_value = "full")]
    mode: String,
    #[arg(long)]
    proposal_id: Option<u64>,
    #[arg(long)]
    broadcast: bool,
    #[arg(long, default_value = "")]
    chain_id: String,
    #[arg(long, default_value_t = 0)]
    account_index: u32,
    #[arg(long, default_value_t = 1800)]
    ttl_secs: u64,
    #[arg(long, default_value = "out/mainnet-deploy/dao-account-plan.json")]
    out: String,
}

pub fn calls_for(n_msgs: usize) -> u64 {
    n_msgs as u64 + 1
}

fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();
    kms_gate::load_dotenv_public();
    env_logger::init();
    let cli = Cli::parse();
    let f: Value = serde_json::from_str(&std::fs::read_to_string(&cli.file)?)?;
    let s = |k: &str| {
        f[k].as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow!("{}: missing {k}", cli.file))
    };
    let (dao, pm, signer) = (s("dao")?, s("proposal_module")?, s("signer")?);
    if cli.granter != signer {
        bail!(
            "refused: --granter {} is not the pinned signer {signer}",
            cli.granter
        );
    }
    let info = kms_gate::morocco_info();
    let d = kms_gate::query_only(&info)?;
    let q = |a: &str, m: Value| -> Result<Value> {
        d.wasm_querier()
            .smart_query(&Addr::unchecked(a), &m)
            .map_err(|e| anyhow!("query {a}: {e}"))
    };
    // On-chain invariants.
    let cfg = q(&pm, json!({"config": {}}))?;
    if cfg["dao"] != json!(dao) {
        bail!("proposal module {pm} belongs to {}, not {dao}", cfg["dao"]);
    }
    let mods = q(&dao, json!({"proposal_modules": {}}))?;
    if !mods.as_array().is_some_and(|v| {
        v.iter()
            .any(|m| m["address"] == json!(pm) && m["status"] == json!("enabled"))
    }) {
        bail!("{pm} is not an enabled module of {dao}");
    }
    if q(&pm, json!({"proposal_creation_policy": {}}))?
        .get("anyone")
        .is_none()
    {
        bail!("{pm}: creation policy is not anyone (pre-propose not supported here)");
    }
    let vm: String = serde_json::from_value(q(&dao, json!({"voting_module": {}}))?)?;
    let vp = q(&vm, json!({"voting_power_at_height": {"address": signer}}))?;
    let tp = q(&vm, json!({"total_power_at_height": {}}))?;
    if vp["power"] == json!("0") {
        bail!("{signer} has no voting power in {dao}");
    }
    if cfg["threshold"]["absolute_count"]["threshold"].as_str() != Some("1")
        || vp["power"] != tp["power"]
    {
        bail!(
            "not a single-member threshold-1 DAO (threshold {}, power {}/{})",
            cfg["threshold"],
            vp["power"],
            tp["power"]
        );
    }
    let veto = !cfg["veto"].is_null();
    let count: u64 = serde_json::from_value(q(&pm, json!({"proposal_count": {}}))?)?;
    let (mut msgs, mut steps) = (vec![], vec![]);
    let pid = match cli.mode.as_str() {
        "full" | "propose-vote" => {
            if cli.mode == "full" && veto {
                bail!(
                    "{pm} has a veto timelock ({}); use --mode propose-vote now and --mode execute after it",
                    cfg["veto"]
                );
            }
            let id = count + 1;
            msgs.push(f["propose"].clone());
            steps.push("propose");
            msgs.push(json!({"vote": {"proposal_id": id, "vote": "yes"}}));
            steps.push("vote");
            if cli.mode == "full" {
                msgs.push(json!({"execute": {"proposal_id": id}}));
                steps.push("execute");
            }
            id
        }
        "execute" => {
            let id = cli
                .proposal_id
                .ok_or_else(|| anyhow!("--mode execute needs --proposal-id"))?;
            let p = q(&pm, json!({"proposal": {"proposal_id": id}}))?;
            if p["proposal"]["status"] != json!("passed") {
                bail!(
                    "proposal {id} status {} (not passed)",
                    p["proposal"]["status"]
                );
            }
            msgs.push(json!({"execute": {"proposal_id": id}}));
            steps.push("execute");
            id
        }
        m => bail!("unknown --mode {m}"),
    };
    let calls = calls_for(msgs.len());
    let keys: Vec<String> = steps.iter().map(|s| s.to_string()).collect();
    let title = f["propose"]["propose"]["title"]
        .as_str()
        .unwrap_or("dao proposal")
        .to_string();
    kms_gate::write_plan(
        &cli.out,
        &json!({
            "dry_run": !cli.broadcast, "file": cli.file, "dao": dao, "proposal_module": pm, "signer": signer,
            "proposal_id": pid, "mode": cli.mode, "veto": cfg["veto"], "steps": steps, "msgs": msgs,
            "predicted_proxy": f["predicted_proxy"],
            "session_scope": {"allow_execute": {"address": pm, "keys": keys, "calls": calls}, "ttl_secs": cli.ttl_secs},
        }),
    )?;
    println!(
        "{title}: {} on {pm} as {signer}, proposal {pid}, calls {calls}",
        steps.join("+")
    );
    if !cli.broadcast {
        return Ok(());
    }
    // Release the read-only daemon (state-file lock) before the session daemon opens.
    drop(d);
    let anys = msgs
        .iter()
        .map(|m| kms_gate::execute_any(&signer, &pm, m))
        .collect::<Result<Vec<_>>>()?;
    let tx = kms_gate::kms_broadcast(
        BroadcastRequest {
            info,
            expected_chain_id: cli.chain_id.clone(),
            granter: signer.clone(),
            purpose: "dao-account".into(),
            account_index: cli.account_index,
            ttl_secs: cli.ttl_secs,
            scope: Scope::Execute(ScopedCall {
                address: pm.clone(),
                keys,
                calls,
                funds: vec![],
            }),
            summary: format!("{title} ({})", steps.join("+")),
        },
        anys,
        "dao_account",
    )?;
    println!("tx {tx}");
    let d = kms_gate::query_only(&kms_gate::morocco_info())?;
    let p = d
        .wasm_querier()
        .smart_query::<_, Value>(
            &Addr::unchecked(&pm),
            &json!({"proposal": {"proposal_id": pid}}),
        )
        .map_err(|e| anyhow!("{e}"))?;
    println!("proposal {pid} status {}", p["proposal"]["status"]);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn headroom_is_one() {
        assert_eq!(super::calls_for(3), 4);
        assert_eq!(super::calls_for(1), 2);
    }
}
