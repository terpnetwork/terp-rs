//! Latent Benders: A Study — finalized day record -> cw721-svg collection config.
//!
//! Inputs come from tools/svg-collections (`daily.py finalize`): `day-record.json` and the
//! `packed.svp` written by `svgc symbient` in the same out dir. Both hashes are re-checked
//! here before anything touches a chain.
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use cosmwasm_std::Binary;
use cw_orch::prelude::Addr;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::collection::SvgCollectionConfig;

pub const COLLECTION_NAME: &str = "Latent Benders: A Study";
pub const SYMBOL: &str = "BENDERS";

/// Python `json.dumps(o, sort_keys=True, separators=(",", ":"), ensure_ascii=False)`.
/// Floats follow serde formatting; daily.py records only carry simple decimals.
pub fn canon(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let body: Vec<String> = keys
                .into_iter()
                .map(|k| format!("{}:{}", Value::String(k.clone()), canon(&m[k])))
                .collect();
            format!("{{{}}}", body.join(","))
        }
        Value::Array(a) => format!("[{}]", a.iter().map(canon).collect::<Vec<_>>().join(",")),
        other => other.to_string(),
    }
}

/// sha256(canon(record without `record_sha256`)), as `cmd_finalize` computes it.
pub fn record_sha256(rec: &Value) -> Result<String> {
    let mut r = rec.clone();
    r.as_object_mut()
        .context("day record is not an object")?
        .remove("record_sha256");
    Ok(hex::encode(Sha256::digest(canon(&r).as_bytes())))
}

#[derive(Clone, Debug)]
pub struct VerifiedDay {
    pub persona: String,
    pub day: String,
    pub day_index: u64,
    pub record_sha256: String,
    pub packed_sha256: String,
    pub packed: Binary,
}

pub fn verify(rec: &Value, packed: &[u8]) -> Result<VerifiedDay> {
    let s = |k: &str| -> Result<String> {
        rec.get(k)
            .and_then(Value::as_str)
            .map(str::to_string)
            .with_context(|| format!("day record missing `{k}`"))
    };
    let claimed = s("record_sha256")?;
    let computed = record_sha256(rec)?;
    ensure!(
        claimed == computed,
        "record_sha256 mismatch: record says {claimed}, computed {computed}"
    );
    let packed_claimed = s("packed_sha256")?;
    let packed_computed = hex::encode(Sha256::digest(packed));
    ensure!(
        packed_claimed == packed_computed,
        "packed_sha256 mismatch: record says {packed_claimed}, packed.svp is {packed_computed}"
    );
    if let Some(n) = rec.get("packed_bytes").and_then(Value::as_u64) {
        ensure!(
            n as usize == packed.len(),
            "packed_bytes {n} != {}",
            packed.len()
        );
    }
    Ok(VerifiedDay {
        persona: s("persona")?,
        day: s("day")?,
        day_index: rec
            .get("day_index")
            .and_then(Value::as_u64)
            .context("day_index")?,
        record_sha256: computed,
        packed_sha256: packed_computed,
        packed: Binary::from(packed.to_vec()),
    })
}

pub fn load(record: &Path, packed: Option<&Path>) -> Result<VerifiedDay> {
    let rec: Value = serde_json::from_slice(
        &std::fs::read(record).with_context(|| format!("read {}", record.display()))?,
    )?;
    let p = packed
        .map(Path::to_path_buf)
        .unwrap_or_else(|| record.with_file_name("packed.svp"));
    let bytes = std::fs::read(&p).with_context(|| format!("read {}", p.display()))?;
    verify(&rec, &bytes)
}

/// Seconds since epoch of 00:00 UTC on `YYYY-MM-DD`.
pub fn day_start_utc(day: &str) -> Result<u64> {
    let p: Vec<&str> = day.split("-").collect();
    if p.len() != 3 {
        bail!("bad day {day}");
    }
    let (y, m, d): (i64, i64, i64) = (p[0].parse()?, p[1].parse()?, p[2].parse()?);
    ensure!(
        (1..=12).contains(&m) && (1..=31).contains(&d),
        "bad day {day}"
    );
    // days_from_civil (H. Hinnant)
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    ensure!(days >= 0, "day before 1970");
    Ok(days as u64 * 86_400)
}

#[derive(Clone, Debug)]
pub struct DayMintParams {
    pub creator: Addr,
    pub total: u64,
    pub price_tiers: Vec<cw721_svg::PriceTier>,
    /// Mint opens at instantiate and closes this many seconds later (the factory series
    /// caps it at 86400).
    pub mint_window_secs: u64,
}

/// Factory series name the Benders days are created under (contract-enforced 1/day).
pub const SERIES: &str = "latent-benders";

impl DayMintParams {
    /// Free, open at instantiate, 24h window, max supply cw721-svg allows (10_000).
    pub fn defaults(creator: Addr) -> Self {
        Self {
            creator,
            total: 10_000,
            price_tiers: vec![],
            mint_window_secs: 86_400,
        }
    }
}

pub fn day_config(v: &VerifiedDay, p: &DayMintParams) -> Result<SvgCollectionConfig> {
    day_start_utc(&v.day)?; // validates the date
    let seed = hex::decode(&v.record_sha256)?;
    Ok(SvgCollectionConfig {
        name: format!("{COLLECTION_NAME} — Day {} ({})", v.day_index, v.day),
        symbol: SYMBOL.into(),
        seed: Binary::from(seed),
        packed: v.packed.clone(),
        total: p.total,
        price_tiers: p.price_tiers.clone(),
        mint_start_time: None,
        mint_end_time: None,
        mint_end_after_secs: Some(p.mint_window_secs),
        creator: p.creator.clone(),
        minter: None,
        withdraw_address: Some(p.creator.clone()),
        require_auth: false,
        label: format!("latent-benders-{}-{}", v.persona, v.day),
    })
}
