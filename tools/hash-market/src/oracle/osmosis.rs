//! Parse Osmosis `ArithmeticTwapToNow` REST/gRPC JSON into an attributed bound.
//!
//! Relayers POST the resulting mid to `cw-oracle-registry` (`Submit`). This
//! module does not open a light client; it is the sidecar feeder shape.

use super::{AttributedPrice, parse_decimal_str};

/// Default decimals for Osmosis decimal strings (18 matches sdk.Dec).
pub const OSMO_DECIMALS: u32 = 18;

/// `GET /osmosis/poolmanager/v1beta1/arithmetic_twap_to_now` body.
pub fn parse_arithmetic_twap_json(
    body: &str,
    market_id: &str,
    source: &str,
    observed_at: u64,
) -> Option<AttributedPrice> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let s = v
        .get("arithmetic_twap")
        .or_else(|| v.get("arithmeticTwap"))
        .and_then(|x| x.as_str())?;
    let mantissa = parse_decimal_str(s, OSMO_DECIMALS)?;
    Some(AttributedPrice {
        source: source.into(),
        market_id: market_id.into(),
        mantissa,
        decimals: OSMO_DECIMALS,
        observed_at,
        weight: 1.0,
    })
}

/// Decimal string suitable for `cw-oracle-registry` `Submit.mid`.
pub fn mid_string(price: &AttributedPrice) -> String {
    let scale = 10u128.pow(price.decimals.min(18));
    let mag = price.mantissa.unsigned_abs();
    let whole = mag / scale;
    let frac = mag % scale;
    let sign = if price.mantissa < 0 { "-" } else { "" };
    format!(
        "{sign}{whole}.{frac:0width$}",
        width = price.decimals as usize
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_poolmanager_shape() {
        let body = r#"{"arithmetic_twap":"1.250000000000000000"}"#;
        let p = parse_arithmetic_twap_json(body, "OSMO/USD", "osmosis-rest", 1).unwrap();
        assert_eq!(p.mantissa, 1_250_000_000_000_000_000);
        assert_eq!(mid_string(&p), "1.250000000000000000");
    }
}
