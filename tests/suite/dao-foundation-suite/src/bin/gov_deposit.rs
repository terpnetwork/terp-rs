//! Prepare two governance deposits. This binary does not sign or broadcast.
//!
//! Proposal 60 is an unsigned `MsgDeposit` of 10,000 TERP from the foundation
//! hot wallet. Proposal 61 is a foundation-DAO proposal whose only message
//! deposits the DAO treasury's own TERP. The default DAO amount is the
//! shortfall up to the chain `min_deposit`.
//!
//! The run also records whether that DAO proposal can execute as soon as
//! its quorum is met.

use anyhow::{Context, Result, bail};
use clap::Parser;
use cosmwasm_std::Uint256;
use dao_foundation_suite::ibc_ops::foundation_proposal_wrapper;
use dao_foundation_suite::ibc_ops::native_delegation::{
    MICRO, PARENT_CORE, PRE_PROPOSE_A, PROPOSAL_A, STAKE_DENOM, early_majority,
    executes_when_quorum_met, gov_deposit_any_msg, quorum_met, smallest_yes_meeting_quorum,
};
use dao_foundation_suite::ibc_ops::quorum::{
    FOUNDATION_DELEGATOR, QuorumPlan, fee_for_gas, next_block_gas, unsigned_tx,
};
use std::path::PathBuf;
use std::process::Command;

const DIRECT_UTERP: u128 = 10_000 * MICRO;

#[derive(Parser, Debug)]
#[command(name = "gov_deposit")]
struct Args {
    #[arg(long, env = "TERP_LCD", default_value = "http://192.168.10.101:1317")]
    lcd: String,
    #[arg(long, default_value = "/tmp/gov-deposit-prep")]
    out_dir: PathBuf,
    #[arg(long, env = "CHAIN_ID", default_value = "morocco-1")]
    chain_id: String,
    #[arg(long, default_value = "0.05uthiol")]
    gas_prices: String,
    #[arg(long, default_value = FOUNDATION_DELEGATOR)]
    depositor: String,
    #[arg(long, default_value = PARENT_CORE)]
    dao: String,
    #[arg(long, default_value = PROPOSAL_A)]
    proposal_module: String,
    #[arg(long, default_value = PRE_PROPOSE_A)]
    pre_propose: String,
    #[arg(long, default_value_t = 60)]
    prop60: u64,
    #[arg(long, default_value_t = 61)]
    prop61: u64,
    /// Whole micro-units on the direct proposal 60 deposit. 10,000 TERP.
    #[arg(long, default_value_t = DIRECT_UTERP)]
    direct_uterp: u128,
    /// Micro-units the DAO deposits on proposal 61. Unset means the shortfall
    /// up to `min_deposit`.
    #[arg(long)]
    dao_uterp: Option<u128>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let lcd = args.lcd.trim_end_matches('/').to_string();
    std::fs::create_dir_all(&args.out_dir)
        .with_context(|| format!("create {}", args.out_dir.display()))?;

    let deposit_params = lcd_get(&format!("{lcd}/cosmos/gov/v1/params/deposit"))?;
    let min_deposit = coin_amount(&deposit_params, "/params/min_deposit", STAKE_DENOM)?;
    let prop60 = lcd_get(&format!("{lcd}/cosmos/gov/v1/proposals/{}", args.prop60))?;
    let prop61 = lcd_get(&format!("{lcd}/cosmos/gov/v1/proposals/{}", args.prop61))?;
    let already60 = coin_amount(&prop60, "/proposal/total_deposit", STAKE_DENOM)?;
    let already61 = coin_amount(&prop61, "/proposal/total_deposit", STAKE_DENOM)?;
    let dao_amount = match args.dao_uterp {
        Some(amount) => amount,
        None => min_deposit.saturating_sub(already61),
    };

    let proposal_cfg = smart(&lcd, &args.proposal_module, r#"{"config":{}}"#)?;
    let pre_cfg = smart(&lcd, &args.pre_propose, r#"{"config":{}}"#)?;
    let voting_module = smart(&lcd, &args.dao, r#"{"voting_module":{}}"#)?
        .pointer("/data")
        .and_then(|v| v.as_str())
        .context("voting module address")?
        .to_string();
    let power_body = smart(&lcd, &voting_module, r#"{"total_power_at_height":{}}"#)?;
    let total_power = power_body
        .pointer("/data/power")
        .and_then(|v| v.as_str())
        .context("total power")?
        .parse::<u128>()
        .context("parse total power")?;
    let group = smart(&lcd, &voting_module, r#"{"group_contract":{}}"#)?
        .pointer("/data")
        .and_then(|v| v.as_str())
        .context("cw4 group")?
        .to_string();
    let members_body = smart(
        &lcd,
        &group,
        r#"{"list_members":{"start_after":null,"limit":50}}"#,
    )?;
    let members = members_body
        .pointer("/data/members")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let quorum_percent = proposal_cfg
        .pointer("/data/threshold/threshold_quorum/quorum/percent")
        .and_then(|v| v.as_str())
        .context("proposal quorum percent")?
        .to_string();
    let allow_revoting = proposal_cfg
        .pointer("/data/allow_revoting")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let min_voting_period = proposal_cfg.pointer("/data/min_voting_period").cloned();
    let veto = proposal_cfg.pointer("/data/veto").cloned();
    let veto_inactive = veto.as_ref().map(|v| v.is_null()).unwrap_or(true);
    let only_members_execute = proposal_cfg
        .pointer("/data/only_members_execute")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let max_voting_period = proposal_cfg.pointer("/data/max_voting_period").cloned();

    let weights: Vec<u128> = members
        .iter()
        .filter_map(|m| m.get("weight").and_then(|w| w.as_u64()).map(u128::from))
        .collect();
    let quorum_yes = smallest_yes_meeting_quorum(total_power, &quorum_percent)
        .map_err(|err| anyhow::anyhow!(err))?;
    let on_quorum = executes_when_quorum_met(total_power, &quorum_percent)
        .map_err(|err| anyhow::anyhow!(err))?;
    let early_rule_applies = !allow_revoting
        && (min_voting_period.is_none() || min_voting_period.as_ref().is_some_and(|v| v.is_null()));
    let single_member_passes_now = weights.iter().any(|w| {
        quorum_met(*w, total_power, &quorum_percent).unwrap_or(false)
            && early_majority(*w, total_power)
    });
    let two_member_passes_now = weights.iter().enumerate().any(|(i, a)| {
        weights.iter().skip(i + 1).any(|b| {
            let yes = a + b;
            quorum_met(yes, total_power, &quorum_percent).unwrap_or(false)
                && early_majority(yes, total_power)
        })
    });

    let hot_spendable = spendable_uterp(&lcd, &args.depositor)?;
    let dao_spendable = spendable_uterp(&lcd, &args.dao)?;
    let account = lcd_get(&format!(
        "{lcd}/cosmos/auth/v1beta1/accounts/{}",
        args.depositor
    ))?;
    let account_type = account
        .pointer("/account/@type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let vest_end = account
        .pointer("/account/base_vesting_account/end_time")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let direct_broadcastable = hot_spendable >= args.direct_uterp;
    let dao_funded = dao_spendable >= dao_amount;

    let gate = serde_json::json!({
        "immediate_on_quorum": on_quorum && early_rule_applies && veto_inactive,
        "quorum_percent": quorum_percent,
        "total_power": total_power.to_string(),
        "smallest_yes_for_quorum": quorum_yes.to_string(),
        "early_majority_rule": "while the proposal is open, yes * 2 must be greater than total power. Quorum is necessary and not sufficient.",
        "early_rule_applies": early_rule_applies,
        "allow_revoting": allow_revoting,
        "min_voting_period": min_voting_period,
        "max_voting_period": max_voting_period,
        "veto": veto,
        "only_members_execute": only_members_execute,
        "single_member_passes_before_expiry": single_member_passes_now && early_rule_applies,
        "two_members_pass_before_expiry": two_member_passes_now && early_rule_applies,
        "after_expiry": "once the voting period ends, majority is of the votes cast, excluding abstain. One yes that already met quorum then passes.",
        "members": members,
        "pre_propose": pre_cfg.pointer("/data").cloned().unwrap_or(serde_json::Value::Null),
        "note": "The treasury deposit on proposal 61 can be executed when the DAO proposal is passed. Meeting quorum does not pass it.",
    });
    let gate_path = args.out_dir.join("00-dao-execution-gate.json");
    std::fs::write(&gate_path, serde_json::to_vec_pretty(&gate)?)?;

    let gas = next_block_gas(
        true,
        &QuorumPlan {
            redelegates: vec![],
            delegate: 0,
            dest_after: 0,
            total_after: 0,
        },
    );
    let (fee_amount, fee_denom) = fee_for_gas(gas, &args.gas_prices)
        .map_err(|err| anyhow::anyhow!("gas price {}: {err}", args.gas_prices))?;
    let direct_msg = serde_json::json!({
        "@type": "/cosmos.gov.v1.MsgDeposit",
        "proposal_id": args.prop60.to_string(),
        "depositor": args.depositor,
        "amount": [{"denom": STAKE_DENOM, "amount": args.direct_uterp.to_string()}],
    });
    let unsigned = unsigned_tx(&[direct_msg.clone()], gas, fee_amount, &fee_denom);
    let unsigned_path = args.out_dir.join("01-prop60-unsigned.json");
    std::fs::write(&unsigned_path, serde_json::to_vec_pretty(&unsigned)?)?;
    let command = vec![
        "terpd".to_string(),
        "tx".into(),
        "gov".into(),
        "deposit".into(),
        args.prop60.to_string(),
        format!("{}{STAKE_DENOM}", args.direct_uterp),
        "--from".into(),
        args.depositor.clone(),
        "--chain-id".into(),
        args.chain_id.clone(),
        "--gas".into(),
        gas.to_string(),
        "--gas-prices".into(),
        args.gas_prices.clone(),
        "--generate-only".into(),
    ];
    let direct = serde_json::json!({
        "proposal_id": args.prop60,
        "title": prop60.pointer("/proposal/title").and_then(|v| v.as_str()).unwrap_or(""),
        "status": prop60.pointer("/proposal/status").and_then(|v| v.as_str()).unwrap_or(""),
        "depositor": args.depositor,
        "amount": {"denom": STAKE_DENOM, "amount": args.direct_uterp.to_string()},
        "already_deposited": already60.to_string(),
        "min_deposit": min_deposit.to_string(),
        "spendable_uterp": hot_spendable.to_string(),
        "account_type": account_type,
        "vesting_end_time": vest_end,
        "broadcastable": direct_broadcastable,
        "gas_limit": gas.to_string(),
        "fee": {"denom": fee_denom, "amount": fee_amount.to_string()},
        "unsigned_tx": unsigned_path.display().to_string(),
        "command": command,
        "note": "Unsigned direct deposit. --generate-only, no --yes, no node URL. broadcastable is false while the hot wallet has fewer spendable uterps than this amount.",
    });
    let direct_path = args.out_dir.join("01-prop60.json");
    std::fs::write(&direct_path, serde_json::to_vec_pretty(&direct)?)?;

    let dao_msg = gov_deposit_any_msg(
        args.dao.clone(),
        args.prop61,
        Uint256::from(dao_amount),
        STAKE_DENOM,
    );
    let title = format!("Deposit treasury TERP on chain proposal {}", args.prop61);
    let prop61_title = prop61
        .pointer("/proposal/title")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let description = format!(
        "This proposal deposits {dao_amount} {STAKE_DENOM} from this DAO treasury onto cosmos governance proposal {id} ({prop61_title}). It is a deposit, not a vote. The chain minimum deposit is {min_deposit} {STAKE_DENOM} and {already61} {STAKE_DENOM} is already deposited. The depositor on the message is this DAO.",
        id = args.prop61,
    );
    let propose = foundation_proposal_wrapper(&title, &description, vec![dao_msg]);
    let propose_path = args.out_dir.join("02-prop61-propose.json");
    std::fs::write(&propose_path, serde_json::to_vec_pretty(&propose)?)?;
    let dao_file = serde_json::json!({
        "proposal_id": args.prop61,
        "title": prop61_title,
        "status": prop61.pointer("/proposal/status").and_then(|v| v.as_str()).unwrap_or(""),
        "depositor": args.dao,
        "amount": {"denom": STAKE_DENOM, "amount": dao_amount.to_string()},
        "amount_is_shortfall": args.dao_uterp.is_none(),
        "already_deposited": already61.to_string(),
        "min_deposit": min_deposit.to_string(),
        "dao_spendable_uterp": dao_spendable.to_string(),
        "dao_can_pay": dao_funded,
        "propose_path": propose_path.display().to_string(),
        "pre_propose": args.pre_propose,
        "proposal_module": args.proposal_module,
        "note": "DAO proposal payload only. Submit it through the pre-propose contract after the execution gate in 00-dao-execution-gate.json is accepted. This file is not a broadcast.",
    });
    let dao_path = args.out_dir.join("02-prop61.json");
    std::fs::write(&dao_path, serde_json::to_vec_pretty(&dao_file)?)?;

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "gate": gate_path.display().to_string(),
            "immediate_on_quorum": gate["immediate_on_quorum"],
            "prop60": direct_path.display().to_string(),
            "prop60_broadcastable": direct_broadcastable,
            "prop61": dao_path.display().to_string(),
            "prop61_dao_can_pay": dao_funded,
            "prop61_amount": dao_amount.to_string(),
        }))?
    );
    Ok(())
}

fn spendable_uterp(lcd: &str, address: &str) -> Result<u128> {
    let body = lcd_get(&format!(
        "{lcd}/cosmos/bank/v1beta1/spendable_balances/{address}"
    ))?;
    Ok(body
        .pointer("/balances")
        .and_then(|v| v.as_array())
        .and_then(|rows| {
            rows.iter().find_map(|row| {
                let denom = row.get("denom").and_then(|v| v.as_str())?;
                if denom != STAKE_DENOM {
                    return None;
                }
                row.get("amount").and_then(|v| v.as_str())
            })
        })
        .unwrap_or("0")
        .parse()
        .context("spendable uterp")?)
}

fn coin_amount(body: &serde_json::Value, pointer: &str, denom: &str) -> Result<u128> {
    let rows = body
        .pointer(pointer)
        .and_then(|v| v.as_array())
        .with_context(|| format!("missing {pointer}"))?;
    let text = rows
        .iter()
        .find_map(|row| {
            let got = row.get("denom").and_then(|v| v.as_str())?;
            if got != denom {
                return None;
            }
            row.get("amount").and_then(|v| v.as_str())
        })
        .with_context(|| format!("no {denom} in {pointer}"))?;
    text.parse().with_context(|| format!("amount {text}"))
}

fn smart(lcd: &str, contract: &str, query: &str) -> Result<serde_json::Value> {
    let encoded = base64_std(query.as_bytes());
    lcd_get(&format!(
        "{lcd}/cosmwasm/wasm/v1/contract/{contract}/smart/{encoded}"
    ))
}

fn lcd_get(url: &str) -> Result<serde_json::Value> {
    let output = Command::new("curl")
        .args([
            "-fsS",
            "--max-time",
            "30",
            "-H",
            "User-Agent: terp-gov-deposit",
            url,
        ])
        .output()
        .context("curl LCD")?;
    if !output.status.success() {
        bail!(
            "LCD {url} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    serde_json::from_slice(&output.stdout).with_context(|| format!("LCD JSON from {url}"))
}

fn base64_std(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n =
            (u32::from(bytes[i]) << 16) | (u32::from(bytes[i + 1]) << 8) | u32::from(bytes[i + 2]);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
        out.push(TABLE[(n & 63) as usize] as char);
        i += 3;
    }
    let rest = bytes.len() - i;
    if rest == 1 {
        let n = u32::from(bytes[i]) << 16;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rest == 2 {
        let n = (u32::from(bytes[i]) << 16) | (u32::from(bytes[i + 1]) << 8);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
        out.push('=');
    }
    out
}
