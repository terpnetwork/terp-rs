//! Foundation DAO: native delegations + FIAB-SUBDAO in one proposal, then vote yes.
//!
//! Default is dry-run (prints msgs). `--broadcast` is refused for morocco-1.
//!
//!   cargo run -p dao-scripts --bin foundation_delegate
//!   TERP_LCD=http://192.168.10.101:1317 cargo run -p dao-scripts --bin foundation_delegate

use anyhow::{Context, Result, bail};
use clap::Parser;
use dao_testing::suite::modules::native_delegation::{
    NEW_VALIDATOR_ACCOUNT, NEW_VALIDATOR_OPERATOR, NativeDelegationPlan, vote_yes_msg,
};

const KILL: &[&str] = &["morocco-1"];

#[derive(Parser, Debug)]
#[command(
    name = "foundation_delegate",
    about = "DAO native stake + FIAB-SUBDAO proposal"
)]
struct Args {
    /// LCD for bonded validators (default: LAN hub).
    #[arg(long, env = "TERP_LCD", default_value = "http://192.168.10.101:1317")]
    lcd: String,
    #[arg(long, default_value_t = true)]
    dry_run: bool,
    #[arg(long, default_value_t = false)]
    broadcast: bool,
    #[arg(long, env = "CHAIN_ID", default_value = "morocco-1")]
    chain_id: String,
    /// Parent foundation dao-dao core. Required to shape the propose wrapper.
    #[arg(
        long,
        env = "PARENT_CORE",
        default_value = "PLACEHOLDER_TERP_FOUNDATION_DAO"
    )]
    parent_core: String,
    /// If set, skip LCD and use this comma-separated valoper list as "others".
    #[arg(long)]
    others: Option<String>,
}

fn fetch_bonded(lcd: &str) -> Result<Vec<String>> {
    let url = format!(
        "{}/cosmos/staking/v1beta1/validators?status=BOND_STATUS_BONDED&pagination.limit=200",
        lcd.trim_end_matches('/')
    );
    let out = std::process::Command::new("curl")
        .args([
            "-fsS",
            "--max-time",
            "20",
            "-H",
            "User-Agent: terp-foundation-delegate",
            &url,
        ])
        .output()
        .context("curl LCD")?;
    if !out.status.success() {
        bail!("LCD fetch failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let vals = v["validators"].as_array().cloned().unwrap_or_default();
    Ok(vals
        .iter()
        .filter_map(|x| x["operator_address"].as_str().map(str::to_string))
        .collect())
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.broadcast && KILL.iter().any(|c| c.eq_ignore_ascii_case(&args.chain_id)) {
        bail!("--broadcast refused for {}", args.chain_id);
    }

    let others = if let Some(raw) = args.others {
        raw.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        match fetch_bonded(&args.lcd) {
            Ok(v) => {
                eprintln!("bonded validators from {}: {}", args.lcd, v.len());
                v
            }
            Err(e) => {
                eprintln!("LCD unavailable ({e}); pass --others valoper1,valoper2");
                Vec::new()
            }
        }
    };

    let plan = NativeDelegationPlan::terp_foundation(others);
    let msgs = plan.cosmos_msgs();
    eprintln!(
        "new operator {} ← {} {} + {} {} to {}",
        plan.new_validator_operator,
        plan.new_validator_stake,
        plan.stake_denom,
        plan.new_validator_gas,
        plan.gas_denom,
        plan.new_validator_account
    );
    eprintln!(
        "other validators {} × {} {}",
        plan.other_validators.len(),
        plan.other_stake,
        plan.stake_denom
    );
    eprintln!(
        "FIAB-SUBDAO instantiate is appended by the caller (same propose.msgs). This binary prints native msgs + vote-yes shape."
    );

    let propose = serde_json::json!({
        "propose": {
            "msg": {
                "propose": {
                    "title": "Say fuck it and begin: FIAB-SUBDAO + validator delegations",
                    "description": "Instantiate FIAB-SUBDAO under Terp Foundation (1e6 uterp + 1e6 uthiol) and delegate 200000 TERP + 7.10 THIOL gas to the new validator, plus 100000 TERP to every other bonded validator.",
                    "msgs": msgs,
                }
            }
        }
    });
    println!("{}", serde_json::to_string_pretty(&propose)?);
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "vote_yes": vote_yes_msg(1),
            "parent_core": args.parent_core,
            "new_validator_account": NEW_VALIDATOR_ACCOUNT,
            "new_validator_operator": NEW_VALIDATOR_OPERATOR,
            "dry_run": args.dry_run,
        }))?
    );
    Ok(())
}
