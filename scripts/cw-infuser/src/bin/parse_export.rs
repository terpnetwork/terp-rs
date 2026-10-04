/// Parses a Cosmos SDK exported state JSON and/or a validators API JSON to
/// produce a tiered whitelist CSV.
///
/// Tiers:
///   1 — addresses with >= min_sequence txs on-chain            → allocation 3
///   2 — historical/inactive validators (non-bonded or jailed)  → allocation 6
///   3 — active bonded validators (BOND_STATUS_BONDED, !jailed) → allocation 18
///
/// Each address receives the highest applicable tier.
///
/// Usage:
/// cargo run -p cw-infuser-scripts --bin parse_export -- --export exported-state.json \
///     --validators data/validators.json --prefix terp -o whitelist.csv
use anyhow::{Context, Result};
use bech32::{Bech32, Hrp};
use clap::Parser;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Parse Cosmos SDK export + validators API JSON to produce a tiered whitelist CSV"
)]
struct Args {
    /// Path to the exported state JSON (live network export).
    /// Used for: active accounts (tier 1) based on non-zero sequence.
    #[arg(long)]
    export: Option<String>,

    /// Path to the validators JSON from the staking API
    /// (e.g. /cosmos/staking/v1beta1/validators?pagination.limit=300).
    /// Used for: active validators (tier 3) and historical validators (tier 2).
    #[arg(long)]
    validators: Option<String>,

    /// Bech32 address prefix (e.g. "terp", "cosmos", "osmo")
    #[arg(long, default_value = "terp")]
    prefix: String,

    /// Minimum sequence (tx count) to qualify for tier 1.
    #[arg(long, default_value = "1")]
    min_sequence: u64,

    /// Output CSV file (defaults to stdout)
    #[arg(long, short)]
    output: Option<String>,
}

// ---------------------------------------------------------------------------
// Validator API response types
// ---------------------------------------------------------------------------

#[derive(Deserialize, Debug)]
struct ValidatorsResponse {
    validators: Vec<Validator>,
}

#[derive(Deserialize, Debug)]
struct Validator {
    operator_address: String,
    jailed: bool,
    status: String,
    description: ValidatorDescription,
}

#[derive(Deserialize, Debug)]
struct ValidatorDescription {
    moniker: String,
}

impl Validator {
    fn is_active(&self) -> bool {
        self.status == "BOND_STATUS_BONDED" && !self.jailed
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract (address, sequence) from a Cosmos SDK auth account JSON object.
/// Handles BaseAccount, vesting accounts, and skips ModuleAccounts.
fn extract_account_info(account: &Value) -> Option<(String, u64)> {
    let account_type = account.get("@type")?.as_str()?;

    if account_type.contains("ModuleAccount") {
        return None;
    }

    let (address, sequence_str) = if let Some(bva) = account.get("base_vesting_account") {
        let ba = bva.get("base_account")?;
        (ba.get("address")?.as_str()?, ba.get("sequence")?.as_str()?)
    } else if account_type.contains("BaseAccount") {
        (
            account.get("address")?.as_str()?,
            account.get("sequence")?.as_str()?,
        )
    } else if let Some(ba) = account.get("base_account") {
        (ba.get("address")?.as_str()?, ba.get("sequence")?.as_str()?)
    } else {
        (
            account.get("address")?.as_str()?,
            account.get("sequence")?.as_str()?,
        )
    };

    let sequence: u64 = sequence_str.parse().ok()?;
    Some((address.to_string(), sequence))
}

/// Convert a valoper address to an account address by swapping the bech32 prefix.
/// e.g. terpvaloper1abc... → terp1abc...
fn valoper_to_account(valoper: &str, account_prefix: &str) -> Result<String> {
    let (_, data) = bech32::decode(valoper)
        .with_context(|| format!("failed to decode bech32 address: {}", valoper))?;
    let hrp = Hrp::parse(account_prefix)?;
    let encoded = bech32::encode::<Bech32>(hrp, &data)?;
    Ok(encoded)
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let args = Args::parse();

    if args.export.is_none() && args.validators.is_none() {
        anyhow::bail!("at least one of --export or --validators must be provided");
    }

    // address → tier (highest tier wins)
    let mut tiers: HashMap<String, u8> = HashMap::new();

    // address → (moniker, operator_address) for validator entries
    let mut validator_meta: HashMap<String, (String, String)> = HashMap::new();

    // --- Tier 1: active accounts from export (non-zero sequence) ---
    if let Some(export_path) = &args.export {
        eprintln!("Loading export: {}", export_path);
        let raw = fs::read_to_string(export_path)
            .with_context(|| format!("failed to read: {}", export_path))?;
        let export: Value =
            serde_json::from_str(&raw).with_context(|| "failed to parse export JSON")?;

        if let Some(accounts) = export
            .pointer("/app_state/auth/accounts")
            .and_then(|v| v.as_array())
        {
            let mut count = 0u64;
            for account in accounts {
                if let Some((address, sequence)) = extract_account_info(account) {
                    if sequence >= args.min_sequence {
                        tiers.entry(address).or_insert(1);
                        count += 1;
                    }
                }
            }
            eprintln!("  Tier 1 (active accounts, sequence >= {}): {}", args.min_sequence, count);
        }
    }

    // --- Tiers 2 & 3: validators from API JSON ---
    if let Some(val_path) = &args.validators {
        eprintln!("Loading validators: {}", val_path);
        let raw = fs::read_to_string(val_path)
            .with_context(|| format!("failed to read: {}", val_path))?;
        let resp: ValidatorsResponse =
            serde_json::from_str(&raw).with_context(|| "failed to parse validators JSON")?;

        let mut active_count = 0u64;
        let mut inactive_count = 0u64;

        for val in &resp.validators {
            let account_addr = match valoper_to_account(&val.operator_address, &args.prefix) {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("  warning: skipping {}: {}", val.operator_address, e);
                    continue;
                }
            };

            let tier: u8 = if val.is_active() { 3 } else { 2 };

            // Always apply the highest tier
            let current = tiers.get(&account_addr).copied().unwrap_or(0);
            if tier > current {
                tiers.insert(account_addr.clone(), tier);
            }

            validator_meta.insert(
                account_addr,
                (val.description.moniker.clone(), val.operator_address.clone()),
            );

            if val.is_active() {
                active_count += 1;
            } else {
                inactive_count += 1;
            }
        }

        eprintln!("  Tier 3 (active bonded validators):      {}", active_count);
        eprintln!("  Tier 2 (inactive/historical validators): {}", inactive_count);
    }

    // ---------------------------------------------------------------------------
    // Build rows
    // ---------------------------------------------------------------------------

    let allocation_for_tier = |tier: u8| -> u32 {
        match tier {
            1 => 3,
            2 => 6,
            3 => 18,
            _ => 0,
        }
    };

    let mut rows: Vec<(String, u8, u32)> = tiers
        .iter()
        .map(|(addr, &tier)| (addr.clone(), tier, allocation_for_tier(tier)))
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    // Summary
    let mut tier_counts: HashMap<u8, usize> = HashMap::new();
    for (_, tier, _) in &rows {
        *tier_counts.entry(*tier).or_default() += 1;
    }
    eprintln!("\nSummary:");
    for tier in 1..=3u8 {
        let count = tier_counts.get(&tier).unwrap_or(&0);
        eprintln!(
            "  Tier {}: {} addresses (allocation: {})",
            tier,
            count,
            allocation_for_tier(tier)
        );
    }
    eprintln!("  Total: {} addresses", rows.len());

    // ---------------------------------------------------------------------------
    // Write single CSV: address,operator_address,moniker,tier,allocation
    // ---------------------------------------------------------------------------

    let buf: Vec<u8> = Vec::new();
    let mut writer = csv::Writer::from_writer(buf);
    writer.write_record(["address", "operator_address", "moniker", "tier", "allocation"])?;
    for (addr, tier, alloc) in &rows {
        let (moniker, operator) = validator_meta
            .get(addr)
            .map(|(m, o)| (m.as_str(), o.as_str()))
            .unwrap_or(("", ""));
        writer.write_record([addr.as_str(), operator, moniker, &tier.to_string(), &alloc.to_string()])?;
    }
    writer.flush()?;
    let buf = writer.into_inner()?;

    match &args.output {
        Some(path) => {
            fs::write(path, &buf).with_context(|| format!("failed to write: {}", path))?;
            eprintln!("Whitelist written to: {}", path);
        }
        None => {
            use std::io::Write;
            std::io::stdout().write_all(&buf)?;
        }
    }

    Ok(())
}
