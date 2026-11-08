//! Pure math: combine values and convert base amounts to alt amounts.
//! Every conversion rounds up, so the contract is never short.

use cosmwasm_std::{Decimal, Decimal256, Fraction, Uint256, Uint512};

use crate::error::OracleError;
use crate::types::{AggregateMethod, Quote};

const ATOMICS: u128 = 1_000_000_000_000_000_000; // 10^18, Decimal256 fractional digits

pub fn to_256(d: Decimal) -> Decimal256 {
    Decimal256::from(d)
}

/// Parse a decimal string as returned by Osmosis-style TWAP queries.
pub fn parse_price(s: &str) -> Result<Decimal256, OracleError> {
    s.trim()
        .parse::<Decimal256>()
        .map_err(|e| OracleError::BadPrice {
            feed: "probe".into(),
            reason: e.to_string(),
        })
}

fn pow10(exp: u8) -> Uint512 {
    Uint512::from(10u128).pow(exp as u32)
}

fn div_ceil(num: Uint512, den: Uint512) -> Result<Uint512, OracleError> {
    if den.is_zero() {
        return Err(OracleError::ZeroPrice);
    }
    let q = num / den;
    if (q * den) == num {
        Ok(q)
    } else {
        q.checked_add(Uint512::one())
            .map_err(|_| OracleError::Overflow)
    }
}

fn narrow(v: Uint512) -> Result<Uint256, OracleError> {
    Uint256::try_from(v).map_err(|_| OracleError::Overflow)
}

/// Alt raw amount that equals `base_amount` raw base units, rounded up.
///
/// * `AltInBase` (price = base per alt): `alt = base * 10^ad / (10^bd * price)`
/// * `BaseInAlt` (price = alt per base): `alt = base * price * 10^ad / 10^bd`
pub fn base_to_alt(
    base_amount: Uint256,
    price: Decimal256,
    quote: Quote,
    base_decimals: u8,
    alt_decimals: u8,
) -> Result<Uint256, OracleError> {
    if price.is_zero() {
        return Err(OracleError::ZeroPrice);
    }
    if base_amount.is_zero() {
        return Ok(Uint256::zero());
    }
    let p = Uint512::from(price.atomics());
    let base = Uint512::from(base_amount);
    let one = Uint512::from(ATOMICS);
    let (num, den) = match quote {
        Quote::AltInBase => (
            base.checked_mul(pow10(alt_decimals))
                .and_then(|v| v.checked_mul(one))
                .map_err(|_| OracleError::Overflow)?,
            pow10(base_decimals)
                .checked_mul(p)
                .map_err(|_| OracleError::Overflow)?,
        ),
        Quote::BaseInAlt => (
            base.checked_mul(p)
                .and_then(|v| v.checked_mul(pow10(alt_decimals)))
                .map_err(|_| OracleError::Overflow)?,
            one.checked_mul(pow10(base_decimals))
                .map_err(|_| OracleError::Overflow)?,
        ),
    };
    narrow(div_ceil(num, den)?)
}

/// Inverse helper: base raw amount worth `alt_amount`, rounded down.
pub fn alt_to_base(
    alt_amount: Uint256,
    price: Decimal256,
    quote: Quote,
    base_decimals: u8,
    alt_decimals: u8,
) -> Result<Uint256, OracleError> {
    if price.is_zero() {
        return Err(OracleError::ZeroPrice);
    }
    let p = Uint512::from(price.atomics());
    let alt = Uint512::from(alt_amount);
    let one = Uint512::from(ATOMICS);
    let (num, den) = match quote {
        Quote::AltInBase => (
            alt.checked_mul(p)
                .and_then(|v| v.checked_mul(pow10(base_decimals)))
                .map_err(|_| OracleError::Overflow)?,
            one.checked_mul(pow10(alt_decimals))
                .map_err(|_| OracleError::Overflow)?,
        ),
        Quote::BaseInAlt => (
            alt.checked_mul(pow10(base_decimals))
                .and_then(|v| v.checked_mul(one))
                .map_err(|_| OracleError::Overflow)?,
            pow10(alt_decimals)
                .checked_mul(p)
                .map_err(|_| OracleError::Overflow)?,
        ),
    };
    narrow(num / den)
}

/// `ceil(required * (1 - slippage))`.
pub fn apply_slippage(
    required: Uint256,
    slippage: Option<Decimal>,
) -> Result<Uint256, OracleError> {
    let Some(s) = slippage else {
        return Ok(required);
    };
    if s >= Decimal::one() {
        return Err(OracleError::InvalidConfig(
            "slippage must be below 1".into(),
        ));
    }
    let keep = Uint512::from(ATOMICS) - Uint512::from(Uint256::from(s.atomics()));
    let num = Uint512::from(required)
        .checked_mul(keep)
        .map_err(|_| OracleError::Overflow)?;
    narrow(div_ceil(num, Uint512::from(ATOMICS))?)
}

/// Invert a price (`1 / p`).
pub fn invert(p: Decimal256) -> Result<Decimal256, OracleError> {
    if p.is_zero() {
        return Err(OracleError::ZeroPrice);
    }
    p.inv().ok_or(OracleError::ZeroPrice)
}

/// `|a - b| / b`.
pub fn deviation(a: Decimal256, b: Decimal256) -> Result<Decimal256, OracleError> {
    if b.is_zero() {
        return Err(OracleError::ZeroPrice);
    }
    let diff = if a > b { a - b } else { b - a };
    diff.checked_div(b).map_err(|_| OracleError::Overflow)
}

/// Combine `(value, weight)` pairs. Weights only matter for `Mean` and `Weighted`.
pub fn combine(
    values: &[(Decimal256, u64)],
    method: AggregateMethod,
) -> Result<Decimal256, OracleError> {
    if values.is_empty() {
        return Err(OracleError::NoObservations {
            feed: "combine".into(),
        });
    }
    match method {
        AggregateMethod::Median => {
            let mut v: Vec<Decimal256> = values.iter().map(|(x, _)| *x).collect();
            v.sort();
            let n = v.len();
            if n % 2 == 1 {
                Ok(v[n / 2])
            } else {
                let sum = v[n / 2 - 1]
                    .checked_add(v[n / 2])
                    .map_err(|_| OracleError::Overflow)?;
                sum.checked_div(Decimal256::from_ratio(2u128, 1u128))
                    .map_err(|_| OracleError::Overflow)
            }
        }
        AggregateMethod::Min => Ok(values.iter().map(|(x, _)| *x).min().unwrap()),
        AggregateMethod::Max => Ok(values.iter().map(|(x, _)| *x).max().unwrap()),
        AggregateMethod::Mean | AggregateMethod::Weighted => {
            let mut acc = Decimal256::zero();
            let mut total: u128 = 0;
            for (x, w) in values {
                let w = (*w).max(if method == AggregateMethod::Mean {
                    1
                } else {
                    0
                });
                if w == 0 {
                    continue;
                }
                acc = acc
                    .checked_add(
                        x.checked_mul(Decimal256::from_ratio(w as u128, 1u128))
                            .map_err(|_| OracleError::Overflow)?,
                    )
                    .map_err(|_| OracleError::Overflow)?;
                total += w as u128;
            }
            if total == 0 {
                return Err(OracleError::NoObservations {
                    feed: "combine".into(),
                });
            }
            acc.checked_div(Decimal256::from_ratio(total, 1u128))
                .map_err(|_| OracleError::Overflow)
        }
    }
}

/// Combine a height-sorted (any order) history window.
/// `Weighted` = piecewise-constant TWAP (each value holds until the next height,
/// the newest holds for 1 block), matching the registry `Twap` query.
pub fn combine_history(
    rows: &[(Decimal256, u64)],
    method: AggregateMethod,
) -> Result<Decimal256, OracleError> {
    if method != AggregateMethod::Weighted {
        let v: Vec<(Decimal256, u64)> = rows.iter().map(|(x, _)| (*x, 1)).collect();
        return combine(&v, method);
    }
    let mut r = rows.to_vec();
    r.sort_by_key(|(_, h)| *h);
    let mut weighted = Vec::with_capacity(r.len());
    for i in 0..r.len() {
        let dt = if i + 1 < r.len() {
            r[i + 1].1.saturating_sub(r[i].1).max(1)
        } else {
            1
        };
        weighted.push((r[i].0, dt));
    }
    combine(&weighted, AggregateMethod::Weighted)
}
