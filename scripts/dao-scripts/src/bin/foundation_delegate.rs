//! Foundation DAO: native delegations (new validator + every bonded validator)
//! as one Foundation proposal via pre-propose module A, then vote yes.
//!
//! Default dry-run writes the proposal. Bonded validators come from the
//! cw-orch Staking querier on morocco-1 (read-only gRPC), or `--others`.
//! `--broadcast` sends one `MsgExecuteContract{propose}` to PRE_PROPOSE_A via
//! cw-orch-kms (scope: that contract, key `propose`, one call), behind the
//! kms_gate checks (`--chain-id morocco-1`, TERP_SCRIPT_BROADCAST=1, v2 capture).
//! FIAB-SUBDAO itself is `fiab_subdao`.

use anyhow::{Result, bail};
use clap::Parser;
use cw_orch::daemon::queriers::{Staking, StakingBondStatus};
use cw_orch_kms::ScopedCall;
use dao_foundation_suite::ibc_ops::{PARENT_CORE, PRE_PROPOSE_A, foundation_proposal_wrapper};
use dao_scripts::kms_gate::{self, BroadcastRequest, Scope};
use dao_testing::suite::native_delegation::{
    NEW_VALIDATOR_ACCOUNT, NEW_VALIDATOR_OPERATOR, NativeDelegationPlan, vote_yes_msg,
};

const TITLE: &str = "Delegate Foundation stake to the new validator and every bonded validator";

#[derive(Parser, Debug)]
#[command(
    name = "foundation_delegate",
    about = "Foundation native stake proposal"
)]
struct Cli {
    #[arg(long, env = "PARENT_CORE", default_value = PARENT_CORE)]
    parent_core: String,
    #[arg(long, default_value = PRE_PROPOSE_A)]
    pre_propose: String,
    /// Comma-separated valopers instead of querying bonded validators.
    #[arg(long)]
    others: Option<String>,
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
    #[arg(long, default_value = "out/foundation_delegate.json")]
    out: String,
}

fn bonded(info: &cw_orch::environment::ChainInfoOwned) -> Result<Vec<String>> {
    let d = kms_gate::query_only(info)?;
    let q = Staking::new_async(d.channel());
    Ok(d.rt_handle
        .block_on(q._validators(StakingBondStatus::Bonded))?
        .into_iter()
        .map(|v| v.address.to_string())
        .collect())
}

fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();
    kms_gate::load_dotenv_public();
    env_logger::init();
    let cli = Cli::parse();
    let morocco = kms_gate::morocco_info();
    let others: Vec<String> = match &cli.others {
        Some(raw) => raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        None => bonded(&morocco)?,
    };
    let plan = NativeDelegationPlan::terp_foundation(others);
    let description = format!(
        "Delegate {} {} (+{} {} gas to {}) to {}, and {} {} to each of {} other bonded validators.",
        plan.new_validator_stake,
        plan.stake_denom,
        plan.new_validator_gas,
        plan.gas_denom,
        plan.new_validator_account,
        plan.new_validator_operator,
        plan.other_stake,
        plan.stake_denom,
        plan.other_validators.len()
    );
    let propose = foundation_proposal_wrapper(TITLE, &description, plan.cosmos_msgs());
    kms_gate::write_plan(
        &cli.out,
        &serde_json::json!({
            "dry_run": !cli.broadcast,
            "chain_id": morocco.chain_id,
            "parent_core": cli.parent_core,
            "contract": cli.pre_propose,
            "execute": propose,
            "other_validators": plan.other_validators.len(),
            "new_validator_account": NEW_VALIDATOR_ACCOUNT,
            "new_validator_operator": NEW_VALIDATOR_OPERATOR,
            "after_vote_yes": vote_yes_msg(1),
            "session_scope": {"allow_execute": {"address": cli.pre_propose, "keys": ["propose"], "calls": 1}, "ttl_secs": cli.ttl_secs, "challenge": "cw-orch-granter:v2"},
        }),
    )?;
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
            purpose: "foundation-delegate".into(),
            account_index: cli.account_index,
            ttl_secs: cli.ttl_secs,
            scope: Scope::Execute(ScopedCall {
                address: cli.pre_propose.clone(),
                keys: vec!["propose".into()],
                calls: 1,
                funds: vec![],
            }),
            summary: TITLE.into(),
        },
        vec![any],
        "foundation_delegate",
    )?;
    println!("tx {tx}");
    Ok(())
}
