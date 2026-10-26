//! Policy-checked prices, quotes and payment settlement.

use cosmwasm_std::{Coin, Decimal256, QuerierWrapper};

use crate::error::OracleError;
use crate::math::{apply_slippage, base_to_alt, combine, deviation, invert};
use crate::querier::OracleQuerier;
use crate::types::{
    AggregateMethod, AltAsset, OraclePricing, PriceAsset, PricePoint, PriceResponse, PricingPolicy,
    Quote, QuoteResponse,
};

/// Apply the policy to raw points and combine them. Pure; used by [`price_of`].
pub fn evaluate(
    alt: &AltAsset,
    policy: &PricingPolicy,
    current_height: u64,
    results: Vec<Result<PricePoint, OracleError>>,
) -> Result<PriceResponse, OracleError> {
    let mut points = Vec::new();
    let mut first_err = None;
    for r in results {
        let checked = r.and_then(|p| check_age(&p, policy, current_height).map(|_| p));
        match checked {
            Ok(p) => points.push(p),
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
    }
    let min = policy.min_sources.unwrap_or(1);
    if (points.len() as u32) < min {
        return Err(match (points.is_empty(), first_err) {
            (true, Some(e)) => e,
            _ => OracleError::NotEnoughSources {
                got: points.len() as u32,
                min,
            },
        });
    }
    let observations: u32 = points.iter().map(|p| p.observations).sum();
    if let Some(min_obs) = policy.min_observations {
        if observations < min_obs {
            return Err(OracleError::NotEnoughObservations {
                got: observations,
                min: min_obs,
            });
        }
    }
    let weighted: Vec<(Decimal256, u64)> = points
        .iter()
        .map(|p| {
            let w = match alt.combine {
                AggregateMethod::Weighted => p.observations as u64,
                _ => p.weight as u64,
            };
            (p.value, w)
        })
        .collect();
    let price = combine(&weighted, alt.combine)?;
    if price.is_zero() {
        return Err(OracleError::ZeroPrice);
    }
    let mut max_dev = Decimal256::zero();
    for p in &points {
        let d = deviation(p.value, price)?;
        if let Some(max) = policy.max_deviation {
            if d > Decimal256::from(max) {
                return Err(OracleError::Deviation {
                    feed: p.feed.clone(),
                    deviation: d.to_string(),
                    max: max.to_string(),
                });
            }
        }
        if d > max_dev {
            max_dev = d;
        }
    }
    let alt_in_base = match alt.quote {
        Quote::AltInBase => price,
        Quote::BaseInAlt => invert(price)?,
    };
    Ok(PriceResponse {
        asset: alt.asset.clone(),
        base_denom: alt.base_denom.clone(),
        quote: alt.quote,
        method: alt.combine,
        price,
        alt_in_base,
        sources: points.len() as u32,
        observations,
        oldest_height: points.iter().filter_map(|p| p.height).min(),
        newest_height: points.iter().filter_map(|p| p.height).max(),
        max_deviation: max_dev,
        points,
    })
}

fn check_age(p: &PricePoint, policy: &PricingPolicy, now: u64) -> Result<(), OracleError> {
    let Some(max_age) = policy.max_staleness_blocks else {
        return Ok(());
    };
    let Some(h) = p.height else {
        return Err(OracleError::UnknownAge {
            feed: p.feed.clone(),
        });
    };
    if h > now {
        return Err(OracleError::FutureObservation {
            feed: p.feed.clone(),
            observed: h,
            current: now,
        });
    }
    if now - h > max_age {
        return Err(OracleError::Stale {
            feed: p.feed.clone(),
            observed: h,
            current: now,
            max_age,
        });
    }
    Ok(())
}

fn lookup<'a>(
    pricing: &'a OraclePricing,
    asset: &PriceAsset,
    base_denom: &str,
) -> Result<&'a AltAsset, OracleError> {
    pricing
        .find(asset, base_denom)
        .ok_or_else(|| OracleError::UnknownAsset {
            asset: asset.to_string(),
            base_denom: base_denom.to_string(),
        })
}

/// Read every feed for `asset`/`base_denom`, apply the policy, combine.
pub fn price_of(
    querier: &QuerierWrapper,
    current_height: u64,
    pricing: &OraclePricing,
    asset: &PriceAsset,
    base_denom: &str,
) -> Result<PriceResponse, OracleError> {
    let alt = lookup(pricing, asset, base_denom)?;
    let q = OracleQuerier::new(querier);
    let results = alt.feeds.iter().map(|f| q.point(f)).collect();
    evaluate(alt, &pricing.policy, current_height, results)
}

/// Build a quote from an already evaluated price. Pure.
pub fn quote_from_price(
    pricing: &OraclePricing,
    alt: &AltAsset,
    base: &Coin,
    price: PriceResponse,
) -> Result<QuoteResponse, OracleError> {
    let required = base_to_alt(
        base.amount,
        price.price,
        alt.quote,
        alt.base_decimals,
        alt.alt_decimals,
    )?;
    let minimum = apply_slippage(required, pricing.policy.slippage)?;
    Ok(QuoteResponse {
        base: base.clone(),
        asset: alt.asset.clone(),
        required,
        minimum,
        enforced: pricing.enforce,
        price,
    })
}

/// Amount of `asset` needed to pay `base` now.
pub fn quote(
    querier: &QuerierWrapper,
    current_height: u64,
    pricing: &OraclePricing,
    base: &Coin,
    asset: &PriceAsset,
) -> Result<QuoteResponse, OracleError> {
    let alt = lookup(pricing, asset, &base.denom)?;
    let price = price_of(querier, current_height, pricing, asset, &base.denom)?;
    quote_from_price(pricing, alt, base, price)
}

/// Outcome of [`settle_payment`].
#[derive(Debug, Clone, PartialEq)]
pub struct Settlement {
    /// Coin the contract keeps (route it to the payee / pot).
    pub charged: Coin,
    /// Overpayment to send back to the payer (zero amount when exact).
    pub refund: Option<Coin>,
    /// `true` when paid with an alt asset.
    pub alt: bool,
    pub quote: Option<QuoteResponse>,
}

/// Check `funds` against a payment of `base`.
///
/// * Funds in `base.denom`: must equal `base.amount` exactly (unchanged behavior).
/// * Funds in another denom: needs `pricing` with `enforce == true` and a
///   matching alt asset. Pays at least the slippage minimum; anything above the
///   quote is refunded.
///
/// Exactly one coin must be sent. A zero `base` accepts no funds (callers
/// usually skip payment in that case).
#[inline(never)]
pub fn settle_payment(
    querier: &QuerierWrapper,
    current_height: u64,
    pricing: Option<&OraclePricing>,
    base: &Coin,
    funds: &[Coin],
) -> Result<Settlement, OracleError> {
    if funds.len() != 1 {
        return Err(OracleError::WrongFundsCount);
    }
    let paid = &funds[0];
    if paid.denom == base.denom {
        if paid.amount != base.amount {
            return Err(OracleError::Underpaid {
                paid: paid.amount,
                required: base.amount,
                denom: base.denom.clone(),
            });
        }
        return Ok(Settlement {
            charged: paid.clone(),
            refund: None,
            alt: false,
            quote: None,
        });
    }
    let pricing = pricing.ok_or(OracleError::NotConfigured)?;
    if !pricing.enforce {
        return Err(OracleError::NotEnforced);
    }
    let asset = PriceAsset::Native(paid.denom.clone());
    let q = quote(querier, current_height, pricing, base, &asset)?;
    settle_with_quote(paid, q)
}

/// Settle against a known quote. Pure.
pub fn settle_with_quote(paid: &Coin, q: QuoteResponse) -> Result<Settlement, OracleError> {
    if paid.amount < q.minimum || paid.amount.is_zero() {
        return Err(OracleError::Underpaid {
            paid: paid.amount,
            required: q.minimum,
            denom: paid.denom.clone(),
        });
    }
    let charged = paid.amount.min(q.required).max(q.minimum);
    let refund = paid.amount - charged;
    Ok(Settlement {
        charged: Coin::new(charged, paid.denom.clone()),
        refund: (!refund.is_zero()).then(|| Coin::new(refund, paid.denom.clone())),
        alt: true,
        quote: Some(q),
    })
}
