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
    #[arg(long, default_value = "out/06-bounty-subdao.json")]
    out: String,
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
    let CosmosMsg::Wasm(WasmMsg::Instantiate { admin, funds, msg: init, .. }) = msg else {
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
    let msg = bounty_msg(&cli.parent, cli.member.trim(), cli.bounty_code_id, &init)?;
    let checks = check_bounty(&msg, &cli.parent)?;
    let propose = foundation_proposal_wrapper(BOUNTY_PROPOSAL_TITLE, &bounty_proposal_description(cli.bounty_code_id), vec![msg]);
    let morocco = kms_gate::morocco_info();
    kms_gate::write_plan(&cli.out, &serde_json::json!({
        "dry_run": !cli.broadcast,
        "chain_id": morocco.chain_id,
        "contract": cli.pre_propose,
        "execute": propose,
        "invariants": checks,
        "session_scope": {"allow_execute": {"address": cli.pre_propose, "keys": ["propose"], "calls": 1}, "ttl_secs": cli.ttl_secs, "challenge": "cw-orch-granter:v2"},
    }))?;
    if !cli.broadcast {
        return Ok(());
    }
    if cli.granter.is_empty() {
        bail!("broadcast refused: pass --granter (a Foundation member)");
    }
    let any = kms_gate::execute_any(&cli.granter, &cli.pre_propose, &propose)?;
    let tx = kms_gate::kms_broadcast(
        BroadcastRequest {
            info: morocco,
            expected_chain_id: cli.chain_id.clone(),
            granter: cli.granter.clone(),
            purpose: "bounty-subdao".into(),
            account_index: cli.account_index,
            ttl_secs: cli.ttl_secs,
            scope: Scope::Execute(ScopedCall { address: cli.pre_propose.clone(), keys: vec!["propose".into()], calls: 1, funds: vec![] }),
            summary: BOUNTY_PROPOSAL_TITLE.into(),
        },
        vec![any],
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
