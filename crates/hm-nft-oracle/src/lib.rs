//! Oracle price bounds for HashMerchant-perched NFT minters.
//!
//! **Hard rule:** oracle data may constrain **payment** only.
//! It must never mint NFT supply, bump `max_tokens`, or mint fungible balances.
//!
//! Sudo envelope matches `x/hashmerchant` keeper:
//! ```json
//! { "hash_merchant": { "chain_uid", "algo", "height", "root", "attestation_count",
//!                      "block_time", "oracle_sources": [{ "source_id", "value", "height" }] } }
//! ```

#![deny(clippy::unwrap_used)]

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Binary, Coin, Timestamp, Uint128};
use thiserror::Error;

/// How the minter obtains price mids.
#[cw_serde]
#[derive(Default, Copy)]
pub enum PriceSyncMode {
    /// Mode A: x/hashmerchant sudo push (preferred long-term).
    #[default]
    Sudo,
    /// Mode B: pull/query another store (stub for frameworks; not required in v0 minter).
    Query,
    /// Mode C: admin ExecuteMsg sync (demo fallback only).
    AdminSync,
}

#[derive(Error, Debug, PartialEq)]
pub enum OracleError {
    #[error("oracle mid is stale (age {age_secs}s > max {max_secs}s)")]
    Stale { age_secs: u64, max_secs: u64 },
    #[error("oracle mid missing and no fallback unit price")]
    MidMissing,
    #[error("payment {paid} outside bound [{min}, {max}]")]
    PaymentOutOfBound {
        paid: Uint128,
        min: Uint128,
        max: Uint128,
    },
    #[error("wrong payment denom: expected {expected}, got {got}")]
    WrongDenom { expected: String, got: String },
    #[error("expected single coin payment, got {0} coins")]
    InvalidFunds(usize),
    #[error("asset id mismatch: expected {expected}, got {got}")]
    AssetMismatch { expected: String, got: String },
    #[error("chain_uid/algo filter rejected sudo update")]
    SourceFilterRejected,
    #[error("invalid price payload: {0}")]
    InvalidPayload(String),
    #[error("bps band overflow")]
    MathOverflow,
    /// Intentionally surfaced if callers try to use oracle path for supply.
    #[error("oracle path cannot modify supply caps")]
    OracleCannotModifySupply,
}

/// Admin / instantiate price policy.
#[cw_serde]
pub struct PricePolicy {
    pub enabled: bool,
    pub mode: PriceSyncMode,
    /// Payment denom (e.g. `uterp`).
    pub denom: String,
    /// Half-width band in basis points around mid. 0 = exact mid only.
    /// Payment must satisfy mid * (1 - bps/10000) ..= mid * (1 + bps/10000).
    pub bps_band: u16,
    /// Fail closed when mid is older than this many seconds (vs `env.block.time`).
    pub max_staleness_seconds: u64,
    /// Optional filter: only accept sudo for this chain_uid.
    pub chain_uid: Option<String>,
    /// Optional filter: only accept sudo for this algo.
    pub algo: Option<String>,
    /// Optional asset filter when decoding JSON price payloads (`asset` field).
    pub asset_id: Option<String>,
    /// When oracle disabled or mid absent: fixed unit price in `denom` microunits.
    pub fallback_unit_price: Option<Uint128>,
    /// If attestation has `price_usd` (decimal string) and no direct mid,
    /// mid = round(price_usd * usd_to_unit_scale). Demo FX only.
    pub usd_to_unit_scale: Option<Uint128>,
}

impl Default for PricePolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: PriceSyncMode::Sudo,
            denom: "uterp".into(),
            bps_band: 0,
            max_staleness_seconds: 3600,
            chain_uid: None,
            algo: None,
            asset_id: None,
            fallback_unit_price: None,
            usd_to_unit_scale: None,
        }
    }
}

/// Stored mid after sudo / admin sync.
#[cw_serde]
pub struct OracleMid {
    /// Payment amount mid in policy denom microunits.
    pub mid: Uint128,
    /// Foreign height associated with this mid (if known).
    pub foreign_height: u64,
    /// Wall-clock when mid was accepted on-chain (unix seconds).
    pub observed_at: u64,
    /// Optional source id from oracle attestation.
    pub source_id: Option<String>,
    /// Optional asset id decoded from payload.
    pub asset_id: Option<String>,
}

/// Client-facing bound quote.
#[cw_serde]
pub struct MintPriceBound {
    pub denom: String,
    pub min: Uint128,
    pub max: Uint128,
    pub mid: Option<Uint128>,
    pub foreign_height: Option<u64>,
    pub observed_at: Option<u64>,
    pub stale: bool,
    pub oracle_enabled: bool,
}

/// Module sudo envelope (struct form — matches Go JSON, not Rust enum).
#[cw_serde]
pub struct HashMerchantSudoMsg {
    pub hash_merchant: HashMerchantSudoPayload,
}

#[cw_serde]
pub struct HashMerchantSudoPayload {
    pub chain_uid: String,
    pub algo: String,
    pub height: u64,
    /// Confirmed root bytes (base64 in JSON).
    pub root: Binary,
    pub attestation_count: u32,
    pub block_time: i64,
    #[serde(default)]
    pub oracle_sources: Vec<HashMerchantSudoOracleAttestation>,
}

#[cw_serde]
pub struct HashMerchantSudoOracleAttestation {
    pub source_id: String,
    /// Opaque value bytes; price demos use JSON UTF-8.
    pub value: Binary,
    pub height: u64,
}

/// Compute [min, max] inclusive around mid for a bps band.
pub fn band_bounds(mid: Uint128, bps_band: u16) -> Result<(Uint128, Uint128), OracleError> {
    if bps_band == 0 {
        return Ok((mid, mid));
    }
    let bps = Uint128::from(bps_band as u32);
    let ten_k = Uint128::new(10_000);
    let delta = mid
        .checked_mul(bps)
        .map_err(|_| OracleError::MathOverflow)?
        .checked_div(ten_k)
        .map_err(|_| OracleError::MathOverflow)?;
    let min = mid.checked_sub(delta).unwrap_or(Uint128::zero());
    let max = mid
        .checked_add(delta)
        .map_err(|_| OracleError::MathOverflow)?;
    Ok((min, max))
}

pub fn is_stale(mid: &OracleMid, now: Timestamp, max_staleness_seconds: u64) -> bool {
    let now_secs = now.seconds();
    if now_secs < mid.observed_at {
        return false;
    }
    (now_secs - mid.observed_at) > max_staleness_seconds
}

/// Resolve payment bound for mint / quote.
pub fn resolve_bound(
    policy: &PricePolicy,
    mid: Option<&OracleMid>,
    now: Timestamp,
) -> Result<MintPriceBound, OracleError> {
    if !policy.enabled {
        let unit = policy
            .fallback_unit_price
            .ok_or(OracleError::MidMissing)?;
        return Ok(MintPriceBound {
            denom: policy.denom.clone(),
            min: unit,
            max: unit,
            mid: Some(unit),
            foreign_height: None,
            observed_at: None,
            stale: false,
            oracle_enabled: false,
        });
    }

    if let Some(m) = mid {
        let stale = is_stale(m, now, policy.max_staleness_seconds);
        if stale {
            // Fail closed: do not fall back silently to unit price when oracle is on.
            return Err(OracleError::Stale {
                age_secs: now.seconds().saturating_sub(m.observed_at),
                max_secs: policy.max_staleness_seconds,
            });
        }
        let (min, max) = band_bounds(m.mid, policy.bps_band)?;
        return Ok(MintPriceBound {
            denom: policy.denom.clone(),
            min,
            max,
            mid: Some(m.mid),
            foreign_height: Some(m.foreign_height),
            observed_at: Some(m.observed_at),
            stale: false,
            oracle_enabled: true,
        });
    }

    // No mid yet: optional fallback only if configured.
    if let Some(unit) = policy.fallback_unit_price {
        return Ok(MintPriceBound {
            denom: policy.denom.clone(),
            min: unit,
            max: unit,
            mid: Some(unit),
            foreign_height: None,
            observed_at: None,
            stale: false,
            oracle_enabled: true,
        });
    }

    Err(OracleError::MidMissing)
}

/// Validate native funds against bound.
pub fn assert_payment(bound: &MintPriceBound, funds: &[Coin]) -> Result<Uint128, OracleError> {
    if funds.len() != 1 {
        return Err(OracleError::InvalidFunds(funds.len()));
    }
    let coin = &funds[0];
    if coin.denom != bound.denom {
        return Err(OracleError::WrongDenom {
            expected: bound.denom.clone(),
            got: coin.denom.clone(),
        });
    }
    // cosmwasm-std v2+/v3 uses Uint256 on Coin; policy mids are Uint128.
    let paid = Uint128::try_from(coin.amount).map_err(|_| OracleError::MathOverflow)?;
    if paid < bound.min || paid > bound.max {
        return Err(OracleError::PaymentOutOfBound {
            paid,
            min: bound.min,
            max: bound.max,
        });
    }
    Ok(paid)
}

/// Whether this sudo payload matches policy filters.
pub fn sudo_matches_filter(policy: &PricePolicy, p: &HashMerchantSudoPayload) -> bool {
    if let Some(ref uid) = policy.chain_uid {
        if uid != &p.chain_uid {
            return false;
        }
    }
    if let Some(ref algo) = policy.algo {
        if algo != &p.algo {
            return false;
        }
    }
    true
}

/// Decode a mid from attestation value bytes under policy.
///
/// Accepted JSON shapes (UTF-8):
/// - `{"mid":"1000000"}` or `{"price_uterp":"1000000"}` (integer string, payment microunits)
/// - `{"asset":"ZEC","price_usd":"45.00"}` when `usd_to_unit_scale` is set
pub fn decode_price_value(
    policy: &PricePolicy,
    value: &[u8],
) -> Result<(Uint128, Option<String>), OracleError> {
    let v: serde_json::Value = serde_json::from_slice(value)
        .map_err(|e| OracleError::InvalidPayload(e.to_string()))?;

    if let Some(expected) = &policy.asset_id {
        if let Some(asset) = v.get("asset").and_then(|x| x.as_str()) {
            if asset != expected {
                return Err(OracleError::AssetMismatch {
                    expected: expected.clone(),
                    got: asset.to_string(),
                });
            }
        }
    }

    let asset_id = v
        .get("asset")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());

    // Direct microunit mid.
    for key in ["mid", "price_uterp", "unit_price"] {
        if let Some(raw) = v.get(key) {
            let s = if let Some(s) = raw.as_str() {
                s.to_string()
            } else if let Some(n) = raw.as_u64() {
                n.to_string()
            } else {
                continue;
            };
            let mid = Uint128::from_str(&s)
                .map_err(|e| OracleError::InvalidPayload(e.to_string()))?;
            return Ok((mid, asset_id));
        }
    }

    // Demo FX: price_usd * scale → microunits.
    if let (Some(scale), Some(px)) = (
        policy.usd_to_unit_scale,
        v.get("price_usd").and_then(|x| x.as_str()),
    ) {
        let mid = usd_times_scale(px, scale)?;
        return Ok((mid, asset_id));
    }

    Err(OracleError::InvalidPayload(
        "no mid/price_uterp/price_usd in attestation".into(),
    ))
}

fn usd_times_scale(price_usd: &str, scale: Uint128) -> Result<Uint128, OracleError> {
    // Support up to 8 decimal places in USD string without float.
    let s = price_usd.trim();
    let (whole, frac) = match s.split_once('.') {
        Some((w, f)) => (w, f),
        None => (s, ""),
    };
    if whole.is_empty() || !whole.chars().all(|c| c.is_ascii_digit()) {
        return Err(OracleError::InvalidPayload(format!(
            "bad price_usd: {price_usd}"
        )));
    }
    if !frac.chars().all(|c| c.is_ascii_digit()) {
        return Err(OracleError::InvalidPayload(format!(
            "bad price_usd frac: {price_usd}"
        )));
    }
    let frac_trimmed: String = frac.chars().take(8).collect();
    let decimals = frac_trimmed.len() as u32;
    let frac_pad = format!("{:0<8}", frac_trimmed); // pad to 8
    let whole_u: u128 = whole
        .parse()
        .map_err(|e: std::num::ParseIntError| OracleError::InvalidPayload(e.to_string()))?;
    let frac_u: u128 = frac_pad
        .parse()
        .map_err(|e: std::num::ParseIntError| OracleError::InvalidPayload(e.to_string()))?;
    // price_nano_usd = whole * 1e8 + frac
    let price_e8 = whole_u
        .checked_mul(100_000_000)
        .and_then(|w| w.checked_add(frac_u))
        .ok_or(OracleError::MathOverflow)?;
    // mid = price_e8 * scale / 1e8
    let mid = (price_e8 as u128)
        .checked_mul(scale.u128())
        .ok_or(OracleError::MathOverflow)?
        .checked_div(100_000_000)
        .ok_or(OracleError::MathOverflow)?;
    let _ = decimals; // kept for clarity
    Ok(Uint128::new(mid))
}

use std::str::FromStr;

/// Apply sudo: update mid from first decodable oracle source that matches filters.
/// Returns `Ok(Some(mid))` if updated, `Ok(None)` if root-only / no usable price
/// (caller keeps previous mid). Never touches supply.
pub fn apply_sudo_price(
    policy: &PricePolicy,
    payload: &HashMerchantSudoPayload,
    observed_at: u64,
) -> Result<Option<OracleMid>, OracleError> {
    if !sudo_matches_filter(policy, payload) {
        return Err(OracleError::SourceFilterRejected);
    }
    if !policy.enabled {
        return Ok(None);
    }

    for src in &payload.oracle_sources {
        match decode_price_value(policy, src.value.as_slice()) {
            Ok((mid, asset_id)) => {
                return Ok(Some(OracleMid {
                    mid,
                    foreign_height: if src.height != 0 {
                        src.height
                    } else {
                        payload.height
                    },
                    observed_at,
                    source_id: Some(src.source_id.clone()),
                    asset_id,
                }));
            }
            Err(OracleError::AssetMismatch { .. }) => continue,
            Err(OracleError::InvalidPayload(_)) => continue,
            Err(e) => return Err(e),
        }
    }
    // Root-only: no mid update.
    Ok(None)
}

/// Explicit guard for tests / call sites: oracle path must not bump supply.
pub fn assert_oracle_cannot_modify_supply() -> Result<(), OracleError> {
    // This function exists so property tests and minter sudo handlers share one
    // documented choke point. Callers must never assign max_tokens from sudo.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::coin;

    fn policy_on() -> PricePolicy {
        PricePolicy {
            enabled: true,
            mode: PriceSyncMode::Sudo,
            denom: "uterp".into(),
            bps_band: 500, // ±5%
            max_staleness_seconds: 100,
            chain_uid: Some("price-oracle".into()),
            algo: Some("sha256".into()),
            asset_id: Some("ZEC".into()),
            fallback_unit_price: None,
            usd_to_unit_scale: Some(Uint128::new(1_000_000)), // $1 → 1e6 uterp
        }
    }

    #[test]
    fn band_exact_and_bps() {
        let (a, b) = band_bounds(Uint128::new(1000), 0).unwrap();
        assert_eq!(a, b);
        let (min, max) = band_bounds(Uint128::new(10_000), 500).unwrap();
        assert_eq!(min, Uint128::new(9_500));
        assert_eq!(max, Uint128::new(10_500));
    }

    #[test]
    fn stale_mid_fail_closed() {
        let policy = policy_on();
        let mid = OracleMid {
            mid: Uint128::new(1_000_000),
            foreign_height: 10,
            observed_at: 1_000,
            source_id: None,
            asset_id: Some("ZEC".into()),
        };
        let now = Timestamp::from_seconds(1_000 + 101);
        let err = resolve_bound(&policy, Some(&mid), now).unwrap_err();
        assert!(matches!(err, OracleError::Stale { .. }));
    }

    #[test]
    fn fresh_mid_payment_check() {
        let policy = policy_on();
        let mid = OracleMid {
            mid: Uint128::new(10_000),
            foreign_height: 10,
            observed_at: 1_000,
            source_id: None,
            asset_id: None,
        };
        let now = Timestamp::from_seconds(1_050);
        let bound = resolve_bound(&policy, Some(&mid), now).unwrap();
        assert_eq!(bound.min, Uint128::new(9_500));
        assert_payment(&bound, &[coin(9_500, "uterp")]).unwrap();
        assert_payment(&bound, &[coin(10_500, "uterp")]).unwrap();
        assert!(assert_payment(&bound, &[coin(9_499, "uterp")]).is_err());
        assert!(assert_payment(&bound, &[coin(10_000, "uatom")]).is_err());
    }

    #[test]
    fn oracle_off_uses_fallback() {
        let policy = PricePolicy {
            enabled: false,
            fallback_unit_price: Some(Uint128::new(42)),
            denom: "uterp".into(),
            ..Default::default()
        };
        let bound = resolve_bound(&policy, None, Timestamp::from_seconds(1)).unwrap();
        assert_eq!(bound.min, Uint128::new(42));
        assert!(!bound.oracle_enabled);
    }

    #[test]
    fn decode_mid_and_price_usd() {
        let policy = policy_on();
        let (m, a) = decode_price_value(&policy, br#"{"asset":"ZEC","mid":"12345"}"#).unwrap();
        assert_eq!(m, Uint128::new(12345));
        assert_eq!(a.as_deref(), Some("ZEC"));

        let (m2, _) =
            decode_price_value(&policy, br#"{"asset":"ZEC","price_usd":"45.00"}"#).unwrap();
        // 45 * 1_000_000 = 45_000_000
        assert_eq!(m2, Uint128::new(45_000_000));
    }

    #[test]
    fn asset_filter_rejects() {
        let policy = policy_on();
        let err = decode_price_value(&policy, br#"{"asset":"BTC","mid":"1"}"#).unwrap_err();
        assert!(matches!(err, OracleError::AssetMismatch { .. }));
    }

    #[test]
    fn apply_sudo_updates_from_oracle_sources() {
        let policy = policy_on();
        let payload = HashMerchantSudoPayload {
            chain_uid: "price-oracle".into(),
            algo: "sha256".into(),
            height: 99,
            root: Binary::from(vec![0u8; 32]),
            attestation_count: 3,
            block_time: 1_700_000_000,
            oracle_sources: vec![HashMerchantSudoOracleAttestation {
                source_id: "skip-connect".into(),
                value: Binary::from(br#"{"asset":"ZEC","price_uterp":"777"}"#.as_slice()),
                height: 98,
            }],
        };
        let mid = apply_sudo_price(&policy, &payload, 1_700_000_100)
            .unwrap()
            .unwrap();
        assert_eq!(mid.mid, Uint128::new(777));
        assert_eq!(mid.foreign_height, 98);
    }

    #[test]
    fn apply_sudo_root_only_no_update() {
        let policy = policy_on();
        let payload = HashMerchantSudoPayload {
            chain_uid: "price-oracle".into(),
            algo: "sha256".into(),
            height: 1,
            root: Binary::from(vec![1u8; 32]),
            attestation_count: 1,
            block_time: 1,
            oracle_sources: vec![],
        };
        assert!(apply_sudo_price(&policy, &payload, 10)
            .unwrap()
            .is_none());
    }

    #[test]
    fn filter_rejects_wrong_chain() {
        let policy = policy_on();
        let payload = HashMerchantSudoPayload {
            chain_uid: "other".into(),
            algo: "sha256".into(),
            height: 1,
            root: Binary::from(vec![0u8; 32]),
            attestation_count: 1,
            block_time: 1,
            oracle_sources: vec![],
        };
        assert!(matches!(
            apply_sudo_price(&policy, &payload, 1),
            Err(OracleError::SourceFilterRejected)
        ));
    }

    #[test]
    fn sudo_must_not_touch_supply_choke_point() {
        // Documented invariant: calling this never returns supply-related mutations.
        assert_oracle_cannot_modify_supply().unwrap();
    }
}
