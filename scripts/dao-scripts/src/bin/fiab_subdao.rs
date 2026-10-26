//! FIAB-SUBDAO deploy: a Foundation proposal (via pre-propose module A) whose
//! one message instantiates FIAB-SUBDAO with the Foundation as wasm admin and
//! DAO admin, seeded with 1,000,000 TERP + 1,000,000 THIOL.
//!
//! cw-orch runtime. Default dry-run prints the proposal and checks admin
//! invariants. `--broadcast` sends one `MsgExecuteContract{propose}` to
//! PRE_PROPOSE_A through the cw-orch Daemon; the cw-orch-kms session allows
//! exactly that contract, key `propose`, one call. The granter must be a
//! Foundation member. Voting and execution stay with the Foundation DAO.

use anyhow::{bail, Result};
use clap::Parser;
use cosmwasm_std::{CosmosMsg, WasmMsg};
use cw_orch_kms::ScopedCall;
use dao_foundation_suite::ibc_ops::{
    fiab_seed, foundation_proposal_wrapper, morocco_fiab_instantiate, CODE_CORE, CODE_CW4_GROUP,
    CODE_PROPOSAL_SINGLE, CODE_VOTING_CW4, FIAB_MEMBER, FIAB_PROPOSAL_TITLE, FOUNDATION_DESCRIPTION,
    PARENT_CORE, PRE_PROPOSE_A,
};
use dao_scripts::kms_gate::{self, BroadcastRequest, Scope};

#[derive(Parser, Debug)]
#[command(name = "fiab_subdao")]
struct Cli {
    #[arg(long, default_value = PARENT_CORE)]
    parent: String,
    #[arg(long, default_value = PRE_PROPOSE_A)]
    pre_propose: String,
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
    #[arg(long, default_value = "out/fiab_subdao.json")]
    out: String,
}

/// Admin invariants for the FIAB instantiate. Public for the Mock tests.
pub fn check_fiab(msg: &CosmosMsg, parent: &str) -> Result<serde_json::Value> {
    let CosmosMsg::Wasm(WasmMsg::Instantiate { admin, code_id, msg: init, funds, .. }) = msg else {
        bail!("FIAB message is not a wasm instantiate");
    };
    if admin.as_deref() != Some(parent) {
        bail!("wasm admin {admin:?} is not the Foundation {parent}");
    }
    if *code_id != CODE_CORE {
        bail!("code {code_id} is not dao-dao-core {CODE_CORE}");
    }
    let init: serde_json::Value = serde_json::from_slice(init)?;
    if init["admin"].as_str() != Some(parent) {
        bail!("DAO admin {} is not the Foundation", init["admin"]);
    }
    if init["voting_module_instantiate_info"]["code_id"] != CODE_VOTING_CW4 {
        bail!("voting module code mismatch");
    }
    if init["proposal_modules_instantiate_info"][0]["code_id"] != CODE_PROPOSAL_SINGLE {
        bail!("proposal module code mismatch");
    }
    if *funds != fiab_seed() {
        bail!("funds {funds:?} differ from the 1M TERP + 1M THIOL seed");
    }
    Ok(serde_json::json!({
        "wasm_admin": parent, "dao_admin": parent, "core_code": CODE_CORE,
        "voting_cw4_code": CODE_VOTING_CW4, "cw4_group_code": CODE_CW4_GROUP,
        "proposal_single_code": CODE_PROPOSAL_SINGLE, "member": FIAB_MEMBER,
        "funds": funds,
    }))
}

fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider().install_default().ok();
    dao_scripts::kms_gate::load_dotenv_public();
    env_logger::init();
    let cli = Cli::parse();
    let msg = morocco_fiab_instantiate(&cli.parent);
    let checks = check_fiab(&msg, &cli.parent)?;
    let propose = foundation_proposal_wrapper(FIAB_PROPOSAL_TITLE, FOUNDATION_DESCRIPTION, vec![msg]);
    let morocco = kms_gate::morocco_info();
    kms_gate::write_plan(&cli.out, &serde_json::json!({
        "dry_run": !cli.broadcast,
        "chain_id": morocco.chain_id,
        "contract": cli.pre_propose,
        "execute": propose,
        "invariants": checks,
        "session_scope": {"allow_execute": {"address": cli.pre_propose, "keys": ["propose"], "calls": 1}, "ttl_secs": cli.ttl_secs, "challenge": "cw-orch-granter:v2"},
        "after": "Foundation members vote on proposal module A; execution sends the instantiate with the funds from the Foundation treasury.",
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
            purpose: "fiab-subdao".into(),
            account_index: cli.account_index,
            ttl_secs: cli.ttl_secs,
            scope: Scope::Execute(ScopedCall { address: cli.pre_propose.clone(), keys: vec!["propose".into()], calls: 1, funds: vec![] }),
            summary: FIAB_PROPOSAL_TITLE.into(),
        },
        vec![any],
        "fiab_subdao",
    )?;
    println!("tx {tx}");
    Ok(())
}
