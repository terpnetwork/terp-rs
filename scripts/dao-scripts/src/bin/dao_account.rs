//! DAO proposal runner over one cw-orch-kms session per tx.
//!
//! Modes:
//! - `--mode full|propose-vote|execute` (no `--action`): single-member,
//!   threshold-1 subDAO (file keys `dao`, `proposal_module`, `signer`,
//!   `propose`), unchanged behaviour.
//! - `--action propose-vote` (multi-member DAO, e.g. Foundation 3x1000 cw4):
//!   member key A proposes through the pre-propose module AND votes yes on the
//!   predicted next id in ONE tx.
//! - `--action vote-execute`: member key B votes yes and executes in ONE tx;
//!   refused unless B's yes passes the proposal (early pass) and it is open.
//!
//! Safety: `--granter` is the only signer; if the file pins `signer` it must
//! match, and the granter must have voting power. Scope: one ScopedCall per
//! contract (pre-propose: `propose`; proposal module: `vote`/`execute`),
//! calls = messages + 1. kms_broadcast self-revokes and asserts nothing is
//! left. Default is a dry run: plan JSON plus an unsigned LCD gas simulation
//! as the granter (no session, no signature).
use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use cosmos_sdk_proto::cosmos::base::v1beta1::Coin as ProtoCoin;
use cosmos_sdk_proto::cosmos::tx::v1beta1::{
    AuthInfo, Fee, ModeInfo, SignerInfo, TxBody, TxRaw, mode_info,
};
use cosmos_sdk_proto::prost::Message;
use cw_orch::environment::WasmQuerier;
use cw_orch::prelude::*;
use cw_orch_kms::ScopedCall;
use dao_scripts::kms_gate::{self, BroadcastRequest, Scope};
use serde_json::{Value, json};

#[derive(Parser)]
struct Cli {
    /// Proposal file. Single-member: keys dao/proposal_module/signer/propose.
    /// Multi-member: dao/proposal_module/pre_propose_module plus
    /// `execute_on_pre_propose` (or `propose`). Optional for vote-execute.
    #[arg(long, alias = "proposal-file")]
    file: Option<String>,
    #[arg(long)]
    granter: String,
    /// full = propose+vote+execute; propose-vote = no execute (veto timelock); execute = --proposal-id only
    #[arg(long, default_value = "full")]
    mode: String,
    /// Multi-member flow: propose-vote | vote-execute (overrides --mode).
    #[arg(long)]
    action: Option<String>,
    #[arg(long)]
    dao: Option<String>,
    #[arg(long)]
    proposal_module: Option<String>,
    #[arg(long)]
    pre_propose: Option<String>,
    #[arg(long)]
    proposal_id: Option<u64>,
    /// propose-vote: refuse unless the predicted next id equals this.
    #[arg(long)]
    expect_proposal_id: Option<u64>,
    #[arg(long)]
    broadcast: bool,
    #[arg(long, default_value = "")]
    chain_id: String,
    #[arg(long, default_value_t = 0)]
    account_index: u32,
    #[arg(long, default_value_t = 1800)]
    ttl_secs: u64,
    #[arg(long, default_value = "http://192.168.10.101:1317")]
    lcd: String,
    #[arg(long, default_value = "uthiol")]
    fee_denom: String,
    #[arg(long, default_value = "out/mainnet-deploy/dao-account-plan.json")]
    out: String,
}

pub fn calls_for(n_msgs: usize) -> u64 {
    n_msgs as u64 + 1
}

/// dao-proposal-single early-pass check against the proposal's snapshotted
/// total power (mirrors dao-voting `does_vote_count_pass` with total power).
pub fn passes(threshold: &Value, yes: u128, no: u128, abstain: u128, total: u128) -> Result<bool> {
    let opts = total.saturating_sub(abstain);
    let pct = |p: &Value, base: u128| -> Result<bool> {
        if p.get("majority").is_some() {
            return Ok(yes * 2 > base);
        }
        let d: f64 = p["percent"]
            .as_str()
            .ok_or_else(|| anyhow!("unsupported percent {p}"))?
            .parse()?;
        Ok(yes > 0 && (yes as f64) >= d * base as f64)
    };
    if let Some(t) = threshold.get("threshold_quorum") {
        let q = &t["quorum"];
        let cast = yes + no + abstain;
        let quorum_ok = if q.get("majority").is_some() {
            cast * 2 > total
        } else {
            let d: f64 = q["percent"].as_str().unwrap_or("1").parse()?;
            (cast as f64) >= d * total as f64
        };
        return Ok(quorum_ok && pct(&t["threshold"], opts)?);
    }
    if let Some(t) = threshold.get("absolute_percentage") {
        return pct(&t["percentage"], opts);
    }
    if let Some(t) = threshold.get("absolute_count") {
        let n: u128 = t["threshold"].as_str().unwrap_or("0").parse()?;
        return Ok(yes >= n);
    }
    bail!("unknown threshold {threshold}")
}

fn simulate(lcd: &str, signer: &str, denom: &str, msgs: &[cosmrs::Any]) -> Result<u64> {
    let lcd = lcd.trim_end_matches('/');
    let curl = |args: &[&str]| -> Result<Value> {
        let out = std::process::Command::new("curl").args(args).output()?;
        serde_json::from_slice(&out.stdout).context("lcd response")
    };
    let acct = curl(&[
        "-s",
        "--max-time",
        "20",
        &format!("{lcd}/cosmos/auth/v1beta1/accounts/{signer}"),
    ])?;
    let seq: u64 = acct["account"]["sequence"]
        .as_str()
        .or_else(|| acct["account"]["base_account"]["sequence"].as_str())
        .ok_or_else(|| anyhow!("{signer}: no account/sequence on chain"))?
        .parse()?;
    let body = TxBody {
        messages: msgs.to_vec(),
        memo: "dao_account dry run".into(),
        ..Default::default()
    };
    let auth = AuthInfo {
        signer_infos: vec![SignerInfo {
            public_key: None,
            sequence: seq,
            mode_info: Some(ModeInfo {
                sum: Some(mode_info::Sum::Single(mode_info::Single { mode: 1 })),
            }),
        }],
        fee: Some(Fee {
            amount: vec![ProtoCoin {
                denom: denom.into(),
                amount: "0".into(),
            }],
            gas_limit: 0,
            payer: String::new(),
            granter: String::new(),
        }),
        ..Default::default()
    };
    let raw = TxRaw {
        body_bytes: body.encode_to_vec(),
        auth_info_bytes: auth.encode_to_vec(),
        signatures: vec![vec![]],
    };
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(raw.encode_to_vec());
    let req = json!({ "tx_bytes": b64 }).to_string();
    let v = curl(&[
        "-s",
        "--max-time",
        "30",
        "-X",
        "POST",
        "-H",
        "content-type: application/json",
        "--data",
        &req,
        &format!("{lcd}/cosmos/tx/v1beta1/simulate"),
    ])?;
    match v["gas_info"]["gas_used"].as_str() {
        Some(g) => Ok(g.parse()?),
        None => bail!(
            "simulate failed: {}",
            v["message"]
                .as_str()
                .map(str::to_string)
                .unwrap_or(v.to_string())
        ),
    }
}

fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();
    kms_gate::load_dotenv_public();
    env_logger::init();
    let cli = Cli::parse();
    match cli.action.as_deref() {
        None => single_member(&cli),
        Some(a @ ("propose-vote" | "vote-execute")) => member(&cli, a),
        Some(a) => bail!("unknown --action {a} (propose-vote | vote-execute)"),
    }
}

fn u(v: &Value) -> u128 {
    v.as_str().and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// Multi-member DAO: one key, one tx, scoped per contract.
fn member(cli: &Cli, action: &str) -> Result<()> {
    let f: Value = match &cli.file {
        Some(p) => serde_json::from_str(&std::fs::read_to_string(p)?)?,
        None => json!({}),
    };
    let pick = |flag: &Option<String>, k: &str| -> Option<String> {
        flag.clone().or_else(|| f[k].as_str().map(str::to_string))
    };
    let dao = pick(&cli.dao, "dao").ok_or_else(|| anyhow!("need --dao or file key dao"))?;
    let pm = pick(&cli.proposal_module, "proposal_module")
        .ok_or_else(|| anyhow!("need --proposal-module or file key proposal_module"))?;
    let signer = cli.granter.clone();
    if let Some(pinned) = f["signer"].as_str() {
        if pinned != signer {
            bail!("refused: --granter {signer} is not the file's pinned signer {pinned}");
        }
    }
    let info = kms_gate::morocco_info();
    let d = kms_gate::query_only(&info)?;
    let q = |a: &str, m: Value| -> Result<Value> {
        d.wasm_querier()
            .smart_query(&Addr::unchecked(a), &m)
            .map_err(|e| anyhow!("query {a}: {e}"))
    };
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
    let vm: String = serde_json::from_value(q(&dao, json!({"voting_module": {}}))?)?;
    let vp = u(&q(&vm, json!({"voting_power_at_height": {"address": signer}}))?["power"]);
    let tp = u(&q(&vm, json!({"total_power_at_height": {}}))?["power"]);
    if vp == 0 {
        bail!("refused: {signer} has no voting power in {dao}");
    }
    let mut notes: Vec<String> = vec![];
    let mut execs: Vec<(String, Value, &str)> = vec![];
    let pid;
    let mut proposal_view = Value::Null;
    if action == "propose-vote" {
        let pp = pick(&cli.pre_propose, "pre_propose_module");
        let policy = q(&pm, json!({"proposal_creation_policy": {}}))?;
        let (target, propose_msg) = match &pp {
            Some(pp) => {
                if policy["module"]["addr"] != json!(pp) {
                    bail!("{pm} creation policy is {policy}, not module {pp}");
                }
                let ppc = q(pp, json!({"config": {}}))?;
                if !ppc["deposit_info"].is_null() {
                    bail!("{pp} requires a deposit {}; funds not supported", ppc["deposit_info"]);
                }
                let sp = &ppc["submission_policy"];
                let allowed = sp.get("anyone").is_some()
                    || sp["specific"]["dao_members"] == json!(true)
                    || sp["specific"]["allowlist"]
                        .as_array()
                        .is_some_and(|a| a.iter().any(|x| x == &json!(signer)));
                let denied = sp["specific"]["denylist"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|x| x == &json!(signer)));
                if !allowed || denied {
                    bail!("{signer} may not submit to {pp}: {sp}");
                }
                let m = f
                    .get("execute_on_pre_propose")
                    .cloned()
                    .or_else(|| f.get("propose").map(|p| json!({"propose": {"msg": p}})))
                    .ok_or_else(|| anyhow!("file needs execute_on_pre_propose or propose"))?;
                if m["propose"]["msg"]["propose"].is_null() {
                    bail!("pre-propose msg must be {{propose:{{msg:{{propose:{{..}}}}}}}}");
                }
                (pp.clone(), m)
            }
            None => {
                if policy.get("anyone").is_none() {
                    bail!("{pm} needs a pre-propose module ({policy}); pass --pre-propose");
                }
                (pm.clone(), f["propose"].clone())
            }
        };
        let count: u64 = serde_json::from_value(q(&pm, json!({"proposal_count": {}}))?)?;
        let next: Option<u64> = q(&pm, json!({"next_proposal_id": {}}))
            .ok()
            .and_then(|v| serde_json::from_value(v).ok());
        let id = next.unwrap_or(count + 1);
        if next.is_some_and(|n| n != count + 1) {
            bail!("next_proposal_id {next:?} != proposal_count+1 {}", count + 1);
        }
        if let Some(e) = cli.expect_proposal_id {
            if e != id {
                bail!("refused: next proposal id is {id}, --expect-proposal-id {e}");
            }
        }
        notes.push(format!(
            "race: vote targets id {id} (proposal_count {count} + 1, next_proposal_id {next:?}) read now; \
             if another member's proposal lands first, the propose still succeeds but the yes vote lands \
             on THEIR proposal. The bin re-reads proposal_count right before broadcast and refuses on drift; \
             pass --expect-proposal-id {id} to pin it."
        ));
        execs.push((target, propose_msg, "propose"));
        execs.push((pm.clone(), json!({"vote": {"proposal_id": id, "vote": "yes"}}), "vote"));
        pid = id;
    } else {
        let id = cli
            .proposal_id
            .ok_or_else(|| anyhow!("--action vote-execute needs --proposal-id"))?;
        pid = id;
        match q(&pm, json!({"proposal": {"proposal_id": id}})) {
            Err(e) if !cli.broadcast => notes.push(format!(
                "proposal {id} not on chain yet ({e}); checks deferred, simulation fails until action 1 lands"
            )),
            Err(e) => return Err(e),
            Ok(p) => {
                let pr = &p["proposal"];
                proposal_view = pr.clone();
                if pr["status"] != json!("open") {
                    bail!("proposal {id} status {} (need open)", pr["status"]);
                }
                let mine = q(&pm, json!({"get_vote": {"proposal_id": id, "voter": signer}}))?;
                if !mine["vote"].is_null() {
                    bail!("{signer} already voted on {id}: {}", mine["vote"]);
                }
                let svp = u(&q(
                    &vm,
                    json!({"voting_power_at_height": {"address": signer, "height": pr["start_height"]}}),
                )?["power"]);
                if svp == 0 {
                    bail!("{signer} had no power at start height {}", pr["start_height"]);
                }
                let v = &pr["votes"];
                let total = u(&pr["total_power"]);
                let after = passes(
                    &pr["threshold"],
                    u(&v["yes"]) + svp,
                    u(&v["no"]),
                    u(&v["abstain"]),
                    total,
                )?;
                notes.push(format!(
                    "pass math: yes {}+{svp} / total {total}, threshold {} -> passes={after}",
                    u(&v["yes"]),
                    pr["threshold"]
                ));
                if !after {
                    bail!("refused: this yes does not pass proposal {id}; execute would fail");
                }
                if !pr["veto"].is_null() {
                    bail!("proposal {id} has a veto timelock; execute later");
                }
            }
        }
        notes.push(format!("only_members_execute={}", cfg["only_members_execute"]));
        execs.push((pm.clone(), json!({"vote": {"proposal_id": id, "vote": "yes"}}), "vote"));
        execs.push((pm.clone(), json!({"execute": {"proposal_id": id}}), "execute"));
    }
    let mut scopes: Vec<ScopedCall> = vec![];
    for (c, _, step) in &execs {
        match scopes.iter_mut().find(|s| &s.address == c) {
            Some(s) => {
                s.keys.push(step.to_string());
                s.calls += 1;
            }
            None => scopes.push(ScopedCall {
                address: c.clone(),
                keys: vec![step.to_string()],
                calls: calls_for(1),
                funds: vec![],
            }),
        }
    }
    let anys = execs
        .iter()
        .map(|(c, m, _)| kms_gate::execute_any(&signer, c, m))
        .collect::<Result<Vec<_>>>()?;
    let sim = simulate(&cli.lcd, &signer, &cli.fee_denom, &anys);
    let title = execs[0].1["propose"]["msg"]["propose"]["title"]
        .as_str()
        .or(proposal_view["title"].as_str())
        .unwrap_or("dao proposal")
        .to_string();
    let steps: Vec<&str> = execs.iter().map(|e| e.2).collect();
    let execs_json: Vec<Value> = execs
        .iter()
        .map(|(c, m, s)| json!({"contract": c, "step": s, "msg": m}))
        .collect();
    let scope_json: Vec<Value> = scopes
        .iter()
        .map(|s| json!({"allow_execute": {"address": s.address, "keys": s.keys, "calls": s.calls}}))
        .collect();
    kms_gate::write_plan(
        &cli.out,
        &json!({
            "dry_run": !cli.broadcast, "action": action, "dao": dao, "proposal_module": pm,
            "signer": signer, "signer_power": vp.to_string(), "total_power": tp.to_string(),
            "proposal_id": pid, "steps": steps, "execs": execs_json,
            "session_scope": scope_json, "ttl_secs": cli.ttl_secs,
            "simulated_gas_used": sim.as_ref().ok(),
            "simulate_error": sim.as_ref().err().map(|e| e.to_string()),
            "notes": notes,
        }),
    )?;
    println!("{title}: {} as {signer}, proposal {pid}", steps.join("+"));
    match &sim {
        Ok(g) => println!("simulated gas_used {g}"),
        Err(e) => println!("simulation: {e}"),
    }
    if !cli.broadcast {
        return Ok(());
    }
    if sim.is_err() {
        bail!("refusing to broadcast: simulation failed");
    }
    if action == "propose-vote" {
        let count: u64 = serde_json::from_value(q(&pm, json!({"proposal_count": {}}))?)?;
        if count + 1 != pid {
            bail!("race: proposal_count moved to {count}; rebuild");
        }
    }
    drop(d);
    let tx = kms_gate::kms_broadcast(
        BroadcastRequest {
            info,
            expected_chain_id: cli.chain_id.clone(),
            granter: signer.clone(),
            purpose: "dao-member".into(),
            account_index: cli.account_index,
            ttl_secs: cli.ttl_secs,
            scope: Scope::Executes(scopes),
            summary: format!("{title} ({})", steps.join("+")),
        },
        anys,
        "dao_account",
    )?;
    println!("tx {tx}");
    let d = kms_gate::query_only(&kms_gate::morocco_info())?;
    let p: Value = d
        .wasm_querier()
        .smart_query(&Addr::unchecked(&pm), &json!({"proposal": {"proposal_id": pid}}))
        .map_err(|e| anyhow!("{e}"))?;
    println!(
        "proposal {pid} status {} title {}",
        p["proposal"]["status"], p["proposal"]["title"]
    );
    Ok(())
}

/// Single-member threshold-1 subDAO (original behaviour).
fn single_member(cli: &Cli) -> Result<()> {
    let file = cli.file.clone().ok_or_else(|| anyhow!("--file required"))?;
    let f: Value = serde_json::from_str(&std::fs::read_to_string(&file)?)?;
    let s = |k: &str| {
        f[k].as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow!("{}: missing {k}", file))
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
            "dry_run": !cli.broadcast, "file": file, "dao": dao, "proposal_module": pm, "signer": signer,
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

    #[test]
    fn foundation_two_of_three_passes_early() {
        let t = serde_json::json!({"threshold_quorum": {"threshold": {"majority": {}}, "quorum": {"percent": "0.2"}}});
        assert!(!super::passes(&t, 1000, 0, 0, 3000).unwrap());
        assert!(super::passes(&t, 2000, 0, 0, 3000).unwrap());
        assert!(super::passes(&t, 2000, 1000, 0, 3000).unwrap());
    }
}
