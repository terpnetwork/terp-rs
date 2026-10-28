//! BOUNTY-SUBDAO: a Foundation proposal (via pre-propose module A) whose one
//! message instantiates the bounty subDAO under the Foundation, seeded with
//! 1,000,000 TERP. Same design as the suite's `foundation_delegate --bounty`
//! (`morocco_bounty_instantiate`): the Foundation is wasm admin and DAO admin,
//! one cw4 member, proposal-single (threshold 1, 24h, members execute), and
//! module B is the bounties contract (`--bounty-code-id`, `--bounty-init`).
//!
//! This is the canonical bounty DAO path. The older standalone instantiate2
//! plan (bounty DAO as its own admin, ephemeral creator) is `bounty_dao_standalone`.
//!
//! Default dry-run writes the plan and checks admin invariants. `--broadcast`
//! sends one `MsgExecuteContract{propose}` to PRE_PROPOSE_A through cw-orch-kms
//! (scope: that contract, key `propose`, one call). Granter = Foundation member.

use anyhow::{bail, Context, Result};
use clap::Parser;
use cosmwasm_std::{CosmosMsg, WasmMsg};
use cw_orch_kms::ScopedCall;
use dao_foundation_suite::ibc_ops::{
    bounty_proposal_description, bounty_seed, foundation_proposal_wrapper, morocco_bounty_instantiate,
    BOUNTY_PROPOSAL_TITLE, PARENT_CORE, PRE_PROPOSE_A,
};
use dao_interface::state::{Admin, ModuleInstantiateInfo};
use dao_scripts::kms_gate::{self, BroadcastRequest, Scope};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "bounty_dao")]
struct Cli {
    #[arg(long, default_value = PARENT_CORE)]
    parent: String,
    #[arg(long, default_value = PRE_PROPOSE_A)]
    pre_propose: String,
    /// The bounty subDAO's single cw4 member (terp1...).
    #[arg(long)]
    member: String,
    /// Uploaded bounties module code id.
    #[arg(long)]
    bounty_code_id: u64,
    /// Bounties instantiate JSON file.
    #[arg(long)]
    bounty_init: PathBuf,
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
    /// instantiate2 salt of the bounty DAO core (Foundation is the creator).
    #[arg(long, default_value = BOUNTY_SALT)]
    salt: String,
    /// sha256 (hex) of the DAO core code (code 23 on morocco-1), used to predict the address.
    #[arg(long, default_value = CORE_CHECKSUM)]
    core_checksum: String,
    /// Also submit, in the same session, the Foundation proposal instantiating the
    /// Polytone notes (code --note-code-id) to Juno and Osmosis, Foundation as admin.
    #[arg(long)]
    with_notes: bool,
    #[arg(long, default_value_t = 78)]
    note_code_id: u64,
    #[arg(long, default_value = "out/06-bounty-subdao.json")]
    out: String,
}

const BOUNTY_SALT: &str = "bounty-subdao";
const CORE_CHECKSUM: &str = "9cac5bccfc7d68893bf99d737a02af27b43fe7992bcf1af2379417955790eaaf";
/// Placeholder in --bounty-init replaced by the predicted DAO core address.
const CORE_PLACEHOLDER: &str = "$DAO_CORE";

fn predict_core(parent: &str, checksum_hex: &str, salt: &str) -> Result<String> {
    let h = checksum_hex.trim();
    if h.len() != 64 { bail!("--core-checksum must be 32-byte hex"); }
    let sum = (0..32).map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16)).collect::<Result<Vec<u8>, _>>().context("--core-checksum hex")?;
    let (_, creator) = dao_foundation_suite::ibc_ops::steward::decode_bech32(parent).map_err(anyhow::Error::msg)?;
    cw_orch_kms::instantiate2_bech32("terp", &sum, &creator, salt.as_bytes()).map_err(|e| anyhow::anyhow!("{e}"))
}

fn resolve_init(init: &serde_json::Value, core: &str) -> Result<serde_json::Value> {
    let s = serde_json::to_string(init)?.replace(CORE_PLACEHOLDER, core);
    let v: serde_json::Value = serde_json::from_str(&s)?;
    for k in ["owner", "approver"] {
        if v[k].as_str() != Some(core) { bail!("bounty init {k} {} is not the predicted DAO core {core}", v[k]); }
    }
    Ok(v)
}

/// Turn the suite's plain Instantiate into Instantiate2 (same code/admin/label/msg/funds).
fn to_instantiate2(msg: CosmosMsg, salt: &str) -> Result<CosmosMsg> {
    let CosmosMsg::Wasm(WasmMsg::Instantiate { admin, code_id, msg, funds, label }) = msg else { bail!("not an instantiate") };
    Ok(CosmosMsg::Wasm(WasmMsg::Instantiate2 { admin, code_id, label, msg, funds, salt: cosmwasm_std::Binary::from(salt.as_bytes()) }))
}

/// (remote, terp connection, remote connection, DAO-DAO voice on the remote chain)
const NOTE_PAIRS: &[(&str, &str, &str, &str)] = &[
    ("juno", "connection-14", "connection-557", "juno1mkq8ggvmr7kzu85c9muud30nmdcv98050uxyqrqmmftlmag044gs3e0d0u"),
    ("osmosis", "connection-15", "connection-11101", "osmo1af93h8xcszszes2a0kjms5zpm5ns3fys4aez2f40fgz428hc8aws28klzs"),
];
const NOTES_TITLE: &str = "Instantiate Polytone notes to Juno and Osmosis";

fn notes_msgs(parent: &str, code_id: u64) -> Result<(Vec<CosmosMsg>, serde_json::Value)> {
    use dao_scripts::polytone as pt;
    let mut msgs = vec![];
    let mut plan = serde_json::Map::new();
    for (remote, conn, rconn, voice) in NOTE_PAIRS {
        let r = pt::remote(remote)?;
        let port = format!("wasm.{voice}");
        let init = pt::note_init(Some((conn, &port)));
        let predicted = pt::predict_for(parent, pt::V3_NOTE_CHECKSUM, &r.note_salt())?;
        msgs.push(CosmosMsg::Wasm(WasmMsg::Instantiate2 {
            admin: Some(parent.to_string()), code_id, label: r.note_label(),
            msg: cosmwasm_std::to_json_binary(&init).map_err(anyhow::Error::msg)?, funds: vec![],
            salt: cosmwasm_std::Binary::from(r.note_salt().as_bytes()),
        }));
        plan.insert(r.chain_id.into(), serde_json::json!({"note_predicted": predicted, "terp_connection": conn, "remote_connection": rconn,
            "voice": voice, "label": r.note_label(), "salt": r.note_salt(), "init": init, "channel_version": pt::CHANNEL_VERSION}));
    }
    Ok((msgs, serde_json::Value::Object(plan)))
}

fn bounty_msg(parent: &str, member: &str, code_id: u64, init: &serde_json::Value) -> Result<CosmosMsg> {
    if !member.starts_with("terp1") || member.contains(char::is_whitespace) {
        bail!("--member must be a terp1 account");
    }
    if code_id == 0 || !init.is_object() {
        bail!("--bounty-code-id must be > 0 and --bounty-init a JSON object");
    }
    Ok(morocco_bounty_instantiate(
        parent,
        member,
        ModuleInstantiateInfo {
            code_id,
            msg: cosmwasm_std::to_json_binary(init).map_err(anyhow::Error::msg)?,
            admin: Some(Admin::CoreModule {}),
            funds: None,
            label: "bounty-bounties".into(),
            salt: None,
        },
    ))
}

fn check_bounty(msg: &CosmosMsg, parent: &str) -> Result<serde_json::Value> {
    let (CosmosMsg::Wasm(WasmMsg::Instantiate { admin, funds, msg: init, .. })
    | CosmosMsg::Wasm(WasmMsg::Instantiate2 { admin, funds, msg: init, .. })) = msg else {
        bail!("bounty message is not a wasm instantiate");
    };
    if admin.as_deref() != Some(parent) {
        bail!("wasm admin {admin:?} is not the Foundation {parent}");
    }
    let init: serde_json::Value = serde_json::from_slice(init.as_slice())?;
    if init["admin"].as_str() != Some(parent) {
        bail!("DAO admin {} is not the Foundation {parent}", init["admin"]);
    }
    if *funds != bounty_seed() {
        bail!("seed {funds:?} != {:?}", bounty_seed());
    }
    Ok(serde_json::json!({"wasm_admin": parent, "dao_admin": parent, "seed": funds}))
}

fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider().install_default().ok();
    kms_gate::load_dotenv_public();
    env_logger::init();
    let cli = Cli::parse();
    let init: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&cli.bounty_init).with_context(|| format!("read {}", cli.bounty_init.display()))?,
    )?;
    let core = predict_core(&cli.parent, &cli.core_checksum, &cli.salt)?;
    let init = resolve_init(&init, &core)?;
    let msg = to_instantiate2(bounty_msg(&cli.parent, cli.member.trim(), cli.bounty_code_id, &init)?, &cli.salt)?;
    let mut checks = check_bounty(&msg, &cli.parent)?;
    checks["predicted_dao_core"] = core.clone().into();
    checks["salt"] = cli.salt.clone().into();
    checks["bounty_init_resolved"] = init.clone();
    println!("predicted bounty DAO core {core} (instantiate2, creator {}, salt {})", cli.parent, cli.salt);
    let propose = foundation_proposal_wrapper(BOUNTY_PROPOSAL_TITLE, &bounty_proposal_description(cli.bounty_code_id), vec![msg]);
    let notes = if cli.with_notes {
        let (msgs, plan) = notes_msgs(&cli.parent, cli.note_code_id)?;
        for (k, v) in plan.as_object().unwrap() { println!("note to {k}: {} pinned {} -> wasm.{}", v["note_predicted"], v["terp_connection"], v["voice"].as_str().unwrap_or("")); }
        let desc = format!("Two Polytone notes (code {}) owned by the Foundation (wasm admin), instantiate2 from the Foundation core, each pinned to one Terp connection and an existing DAO-DAO voice:\n- juno-1: connection-14 (Juno connection-557), voice {}\n- osmosis-1: connection-15 (Osmosis connection-11101), voice {}\nChannels (polytone-1, unordered) are opened by the relayer after execution.", cli.note_code_id, NOTE_PAIRS[0].3, NOTE_PAIRS[1].3);
        Some((foundation_proposal_wrapper(NOTES_TITLE, &desc, msgs), plan))
    } else { None };
    // +1 headroom: a fully spent wasm grant is auto-deleted and the generic revoke then fails.
    let n_calls = 2 + notes.is_some() as u64;
    let morocco = kms_gate::morocco_info();
    kms_gate::write_plan(&cli.out, &serde_json::json!({
        "dry_run": !cli.broadcast,
        "chain_id": morocco.chain_id,
        "contract": cli.pre_propose,
        "execute": propose,
        "invariants": checks,
        "notes_proposal": notes.as_ref().map(|(p, plan)| serde_json::json!({"execute": p, "plan": plan})),
        "session_scope": {"allow_execute": {"address": cli.pre_propose, "keys": ["propose"], "calls": n_calls}, "ttl_secs": cli.ttl_secs, "challenge": "cw-orch-granter:v2"},
    }))?;
    if !cli.broadcast {
        return Ok(());
    }
    if cli.granter.is_empty() {
        bail!("broadcast refused: pass --granter (a Foundation member)");
    }
    let mut anys = vec![kms_gate::execute_any(&cli.granter, &cli.pre_propose, &propose)?];
    if let Some((p, _)) = &notes { anys.push(kms_gate::execute_any(&cli.granter, &cli.pre_propose, p)?); }
    let tx = kms_gate::kms_broadcast(
        BroadcastRequest {
            info: morocco,
            expected_chain_id: cli.chain_id.clone(),
            granter: cli.granter.clone(),
            purpose: "bounty-subdao".into(),
            account_index: cli.account_index,
            ttl_secs: cli.ttl_secs,
            scope: Scope::Execute(ScopedCall { address: cli.pre_propose.clone(), keys: vec!["propose".into()], calls: n_calls, funds: vec![] }),
            summary: BOUNTY_PROPOSAL_TITLE.into(),
        },
        anys,
        "bounty_dao",
    )?;
    println!("tx {tx}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const MEMBER: &str = "terp1d09ytkkt5uwdchx3nsehalktgw2h5ptsapmu4gde2lw2q7lm7xas3zyh7j";

    #[test]
    fn foundation_is_admin_of_bounty_subdao() {
        let m = bounty_msg(PARENT_CORE, MEMBER, 9, &serde_json::json!({})).unwrap();
        let c = check_bounty(&m, PARENT_CORE).unwrap();
        assert_eq!(c["wasm_admin"], PARENT_CORE);
        assert!(check_bounty(&m, MEMBER).is_err());
    }

    #[test]
    fn rejects_bad_inputs() {
        assert!(bounty_msg(PARENT_CORE, "osmo1x", 9, &serde_json::json!({})).is_err());
        assert!(bounty_msg(PARENT_CORE, MEMBER, 0, &serde_json::json!({})).is_err());
        assert!(bounty_msg(PARENT_CORE, MEMBER, 9, &serde_json::json!([])).is_err());
    }
}
