//! Multi-source **price bounds** aggregation (Tacit-aligned).
//!
//! # Hard rules (program thesis)
//!
//! - Oracles **price bounds only** (cUSD-like). They do **not** mint
//!   conservation balances (cBTC-like).
//! - Default hash-market path remains **state_root** VE providers; this module
//!   is the elevated sophistication path for multi-source mid / TWAP-style bounds.
//! - Patterns follow Skip Connect (providers → attribute → aggregate → publish),
//!   without forking Connect or inventing a parallel mint path.
//!
//! ## Connect mapping
//!
//! ```text
//! many sources (HTTP tickers) → attribute store → aggregate_bounds (median)
//!   → one OracleBound per market_id → GET /oracle/bounds and/or VE root
//! ```
//!
//! Each **provider** (sidecar) publishes **one** aggregated mid per pair.
//! On-chain quorum is classic power vote on that mid — not multi-source bag consensus.
//!
//! See `docs/oracle-connect-bounds.md`.

use std::collections::HashMap;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

/// Vote-extension algo for Connect-style aggregated mids (never a foreign state root).
pub const ALGO_PRICE_MEDIAN_V1: &str = "price_median_v1";

/// Wire encoding of [`OracleBound`] into `VoteExtensionHashData.root`:
/// `decimals:u32 BE || mantissa:i128 BE` (20 bytes).
pub const BOUND_ROOT_LEN: usize = 4 + 16;

#[cfg(feature = "server")]
mod feeder;
#[cfg(feature = "server")]
pub use feeder::run_price_bound_feeder;

/// What a provider is allowed to attest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// Foreign chain state root / height (current default VE path).
    #[default]
    StateRoot,
    /// Multi-source mid used only as quote / CDP **bounds**.
    PriceBound,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StateRoot => "state_root",
            Self::PriceBound => "price_bound",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "state_root" | "root" | "hash" | "" => Some(Self::StateRoot),
            "price_bound" | "price" | "bound" | "oracle" => Some(Self::PriceBound),
            _ => None,
        }
    }
}

/// How to combine multiple attributed sources for one `market_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AggregationMethod {
    /// Median of finite prices (default; Connect-like resilience).
    #[default]
    Median,
    /// Equal-weight mean.
    Mean,
    /// Weight field on each observation.
    WeightedMean,
    /// Drop one high + one low (if n≥3), then mean.
    TrimmedMean,
    /// Strict lower envelope (e.g. min_out guards).
    Min,
    /// Strict upper envelope.
    Max,
}

impl AggregationMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Median => "median",
            Self::Mean => "mean",
            Self::WeightedMean => "weighted_mean",
            Self::TrimmedMean => "trimmed_mean",
            Self::Min => "min",
            Self::Max => "max",
        }
    }
}

/// One attributed price tick from a named source (pre-aggregate).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttributedPrice {
    /// Provider / feeder name (source id).
    pub source: String,
    /// Market identifier (e.g. `ETH/USD`, `USDT/USD`).
    pub market_id: String,
    /// Price in fixed-point: `price = mantissa / 10^decimals`.
    pub mantissa: i128,
    pub decimals: u32,
    /// Unix seconds when the source observed this tick.
    pub observed_at: u64,
    /// Optional weight for [`AggregationMethod::WeightedMean`] (default 1.0).
    #[serde(default = "default_weight")]
    pub weight: f64,
}

fn default_weight() -> f64 {
    1.0
}

impl AttributedPrice {
    /// Convert to f64 for aggregation (lossy for huge mantissas; v1 bounds ok).
    pub fn as_f64(&self) -> f64 {
        let scale = 10f64.powi(self.decimals as i32);
        (self.mantissa as f64) / scale
    }
}

/// Aggregated bound for a market — **bound only, never a mint instruction**.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OracleBound {
    pub market_id: String,
    pub method: AggregationMethod,
    /// Same fixed-point convention as sources (uses max decimals among inputs).
    pub mantissa: i128,
    pub decimals: u32,
    pub n_sources: u32,
    pub sources: Vec<String>,
    /// Max observed_at among contributors.
    pub as_of: u64,
    /// Explicit role tag for API consumers (private DEX, CDP).
    #[serde(default = "bound_only_role")]
    pub role: String,
}

fn bound_only_role() -> String {
    "bound_only".into()
}

/// Policy for accepting / aggregating sources.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AggregationPolicy {
    pub method: AggregationMethod,
    /// Minimum fresh sources required.
    #[serde(default = "default_min_sources")]
    pub min_sources: usize,
    /// Drop observations older than this many seconds relative to `now`.
    #[serde(default = "default_max_age")]
    pub max_age_secs: u64,
}

fn default_min_sources() -> usize {
    1
}
fn default_max_age() -> u64 {
    120
}

impl Default for AggregationPolicy {
    fn default() -> Self {
        Self {
            method: AggregationMethod::Median,
            min_sources: 1,
            max_age_secs: 120,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AggregateError {
    #[error("no observations for market")]
    Empty,
    #[error("need at least {need} fresh sources, have {have}")]
    InsufficientSources { need: usize, have: usize },
    #[error("market_id mismatch in batch")]
    MarketMismatch,
}

/// Aggregate attributed prices for a single market under policy.
///
/// Callers must pre-filter to one `market_id`. Stale ticks (older than
/// `now - max_age_secs`) are dropped before the min_sources check.
pub fn aggregate_bounds(
    market_id: &str,
    observations: &[AttributedPrice],
    policy: &AggregationPolicy,
    now: u64,
) -> Result<OracleBound, AggregateError> {
    if observations.is_empty() {
        return Err(AggregateError::Empty);
    }
    for o in observations {
        if o.market_id != market_id {
            return Err(AggregateError::MarketMismatch);
        }
    }

    let fresh: Vec<&AttributedPrice> = observations
        .iter()
        .filter(|o| now.saturating_sub(o.observed_at) <= policy.max_age_secs)
        .collect();

    if fresh.len() < policy.min_sources {
        return Err(AggregateError::InsufficientSources {
            need: policy.min_sources,
            have: fresh.len(),
        });
    }

    let decimals = fresh.iter().map(|o| o.decimals).max().unwrap_or(0);
    let mut prices: Vec<(f64, f64, &str, u64)> = fresh
        .iter()
        .map(|o| {
            (
                o.as_f64(),
                o.weight.max(0.0),
                o.source.as_str(),
                o.observed_at,
            )
        })
        .collect();

    let value = match policy.method {
        AggregationMethod::Median => {
            prices.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            let n = prices.len();
            if n % 2 == 1 {
                prices[n / 2].0
            } else {
                (prices[n / 2 - 1].0 + prices[n / 2].0) / 2.0
            }
        }
        AggregationMethod::Mean => {
            let s: f64 = prices.iter().map(|p| p.0).sum();
            s / prices.len() as f64
        }
        AggregationMethod::WeightedMean => {
            let wsum: f64 = prices.iter().map(|p| p.1).sum();
            if wsum <= 0.0 {
                let s: f64 = prices.iter().map(|p| p.0).sum();
                s / prices.len() as f64
            } else {
                prices.iter().map(|p| p.0 * p.1).sum::<f64>() / wsum
            }
        }
        AggregationMethod::TrimmedMean => {
            prices.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            let slice = if prices.len() >= 3 {
                &prices[1..prices.len() - 1]
            } else {
                prices.as_slice()
            };
            let s: f64 = slice.iter().map(|p| p.0).sum();
            s / slice.len() as f64
        }
        AggregationMethod::Min => prices.iter().map(|p| p.0).fold(f64::INFINITY, f64::min),
        AggregationMethod::Max => prices.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max),
    };

    let scale = 10f64.powi(decimals as i32);
    let mantissa = (value * scale).round() as i128;
    let mut sources: Vec<String> = fresh.iter().map(|o| o.source.clone()).collect();
    sources.sort();
    let as_of = fresh.iter().map(|o| o.observed_at).max().unwrap_or(0);

    Ok(OracleBound {
        market_id: market_id.to_string(),
        method: policy.method,
        mantissa,
        decimals,
        n_sources: fresh.len() as u32,
        sources,
        as_of,
        role: bound_only_role(),
    })
}

// ── VE root packing (one mid per market; Connect-style) ─────────────────────

/// Pack fixed-point mid into VE `root` bytes (`price_median_v1`).
pub fn encode_bound_root(decimals: u32, mantissa: i128) -> Vec<u8> {
    let mut out = Vec::with_capacity(BOUND_ROOT_LEN);
    out.extend_from_slice(&decimals.to_be_bytes());
    out.extend_from_slice(&mantissa.to_be_bytes());
    out
}

/// Decode [`encode_bound_root`] payload.
pub fn decode_bound_root(root: &[u8]) -> Option<(u32, i128)> {
    if root.len() != BOUND_ROOT_LEN {
        return None;
    }
    let mut d = [0u8; 4];
    d.copy_from_slice(&root[0..4]);
    let mut m = [0u8; 16];
    m.copy_from_slice(&root[4..20]);
    Some((u32::from_be_bytes(d), i128::from_be_bytes(m)))
}

/// Chain UID used when a price_bound provider omits `chain_uid`.
pub fn price_chain_uid(market_id: &str) -> String {
    format!("price:{}", market_id)
}

/// Build VE payload from an aggregated bound (empty attestations — mid is the commitment).
pub fn bound_to_vote_extension(
    bound: &OracleBound,
    runtime_id: &str,
    chain_uid: &str,
    algo: &str,
) -> crate::msg::VoteExtensionHashData {
    crate::msg::VoteExtensionHashData {
        runtime_id: runtime_id.to_string(),
        chain_uid: chain_uid.to_string(),
        algo: algo.to_string(),
        root: encode_bound_root(bound.decimals, bound.mantissa),
        foreign_height: bound.as_of,
        foreign_block_time: bound.as_of as i64,
        ics23_proof: Vec::new(),
    }
}

// ── Ticker JSON ingress ─────────────────────────────────────────────────────

/// Parse a Connect-style ticker JSON body into an attributed price.
///
/// Accepted shapes (flexible for demos / exchange stubs):
/// ```json
/// {"price": "3450.12"}
/// {"price": 3450.12, "timestamp": 1700000000}
/// {"market_id": "ETH/USD", "price": "3450.12"}
/// {"data": {"price": "3450.12"}}
/// ```
///
/// When `market_id` is absent in JSON, `default_market_id` is used.
/// `decimals` scales the price into a fixed-point mantissa (default 8).
pub fn parse_ticker_json(
    body: &str,
    source: &str,
    default_market_id: &str,
    weight: f64,
    default_decimals: u32,
    now: u64,
) -> Result<AttributedPrice, String> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("ticker json: {e}"))?;

    let obj = if let Some(data) = v.get("data").filter(|d| d.is_object()) {
        data
    } else {
        &v
    };

    let market_id = obj
        .get("market_id")
        .or_else(|| obj.get("symbol"))
        .and_then(|x| x.as_str())
        .unwrap_or(default_market_id)
        .to_string();

    let price_val = obj
        .get("price")
        .or_else(|| obj.get("last"))
        .or_else(|| obj.get("mid"))
        .ok_or_else(|| "ticker missing price field".to_string())?;

    let decimals = obj
        .get("decimals")
        .and_then(|d| d.as_u64())
        .map(|d| d as u32)
        .unwrap_or(default_decimals);

    let mantissa = price_value_to_mantissa(price_val, decimals)
        .ok_or_else(|| "invalid price value".to_string())?;

    let observed_at = obj
        .get("timestamp")
        .or_else(|| obj.get("observed_at"))
        .or_else(|| obj.get("ts"))
        .and_then(|t| {
            t.as_u64()
                .or_else(|| t.as_i64().map(|i| i as u64))
                .or_else(|| t.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(now);

    Ok(AttributedPrice {
        source: source.to_string(),
        market_id,
        mantissa,
        decimals,
        observed_at,
        weight: if weight > 0.0 { weight } else { 1.0 },
    })
}

fn price_value_to_mantissa(v: &serde_json::Value, decimals: u32) -> Option<i128> {
    if let Some(s) = v.as_str() {
        return parse_decimal_str(s, decimals);
    }
    if let Some(n) = v.as_f64() {
        let scale = 10f64.powi(decimals as i32);
        return Some((n * scale).round() as i128);
    }
    if let Some(i) = v.as_i64() {
        let scale = 10i128.pow(decimals);
        return Some(i as i128 * scale);
    }
    if let Some(u) = v.as_u64() {
        let scale = 10i128.pow(decimals);
        return Some(u as i128 * scale);
    }
    None
}

/// Parse `"3450.12"` into fixed-point mantissa at `decimals`.
pub fn parse_decimal_str(s: &str, decimals: u32) -> Option<i128> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let neg = s.starts_with('-');
    let s = s
        .strip_prefix('+')
        .or_else(|| s.strip_prefix('-'))
        .unwrap_or(s);
    let (whole, frac) = match s.split_once('.') {
        Some((w, f)) => (w, f),
        None => (s, ""),
    };
    if whole.is_empty() && frac.is_empty() {
        return None;
    }
    let whole_n: i128 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let frac_digits: String = frac.chars().filter(|c| c.is_ascii_digit()).collect();
    let frac_n: i128 = if frac_digits.is_empty() {
        0
    } else {
        frac_digits.parse().ok()?
    };
    let frac_len = frac_digits.len() as u32;
    let scale = 10i128.pow(decimals);
    let whole_part = whole_n.checked_mul(scale)?;
    let frac_part = if frac_len >= decimals {
        // truncate extra digits
        let trim = frac_len - decimals;
        frac_n / 10i128.pow(trim)
    } else {
        frac_n * 10i128.pow(decimals - frac_len)
    };
    let mag = whole_part.checked_add(frac_part)?;
    Some(if neg { -mag } else { mag })
}

// ── In-process attribute + bounds store (Connect oracle map) ────────────────

/// Thread-safe store: latest tick per (source, market_id), latest bound per market.
#[derive(Debug, Default)]
pub struct OracleAttributeStore {
    /// key = `source\0market_id`
    ticks: RwLock<HashMap<String, AttributedPrice>>,
    bounds: RwLock<HashMap<String, OracleBound>>,
}

fn tick_key(source: &str, market_id: &str) -> String {
    format!("{source}\0{market_id}")
}

impl OracleAttributeStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Upsert a source tick and re-aggregate that market under `policy`.
    pub fn upsert_and_aggregate(
        &self,
        tick: AttributedPrice,
        policy: &AggregationPolicy,
        now: u64,
    ) -> Result<OracleBound, AggregateError> {
        let market_id = tick.market_id.clone();
        {
            let mut map = self.ticks.write().expect("oracle ticks lock");
            map.insert(tick_key(&tick.source, &market_id), tick);
        }
        self.reaggregate(&market_id, policy, now)
    }

    /// Recompute bound for `market_id` from all stored ticks.
    pub fn reaggregate(
        &self,
        market_id: &str,
        policy: &AggregationPolicy,
        now: u64,
    ) -> Result<OracleBound, AggregateError> {
        let obs: Vec<AttributedPrice> = {
            let map = self.ticks.read().expect("oracle ticks lock");
            map.values()
                .filter(|t| t.market_id == market_id)
                .cloned()
                .collect()
        };
        let bound = aggregate_bounds(market_id, &obs, policy, now)?;
        self.bounds
            .write()
            .expect("oracle bounds lock")
            .insert(market_id.to_string(), bound.clone());
        Ok(bound)
    }

    pub fn get_bound(&self, market_id: &str) -> Option<OracleBound> {
        self.bounds
            .read()
            .expect("oracle bounds lock")
            .get(market_id)
            .cloned()
    }

    pub fn list_bounds(&self) -> Vec<OracleBound> {
        self.bounds
            .read()
            .expect("oracle bounds lock")
            .values()
            .cloned()
            .collect()
    }

    pub fn list_ticks(&self, market_id: Option<&str>) -> Vec<AttributedPrice> {
        let map = self.ticks.read().expect("oracle ticks lock");
        map.values()
            .filter(|t| market_id.map(|m| t.market_id == m).unwrap_or(true))
            .cloned()
            .collect()
    }
}

/// JSON body for `GET /oracle/bounds` (API sketch from design doc).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleBoundsResponse {
    pub market_id: String,
    pub method: String,
    pub price: String,
    pub decimals: u32,
    pub mantissa: i128,
    pub n_sources: u32,
    pub sources: Vec<String>,
    pub as_of: u64,
    pub role: String,
}

impl From<&OracleBound> for OracleBoundsResponse {
    fn from(b: &OracleBound) -> Self {
        let scale = 10i128.pow(b.decimals);
        let whole = b.mantissa / scale;
        let frac = (b.mantissa % scale).unsigned_abs();
        let price = if b.decimals == 0 {
            whole.to_string()
        } else {
            format!("{whole}.{:0width$}", frac, width = b.decimals as usize)
        };
        Self {
            market_id: b.market_id.clone(),
            method: b.method.as_str().to_string(),
            price,
            decimals: b.decimals,
            mantissa: b.mantissa,
            n_sources: b.n_sources,
            sources: b.sources.clone(),
            as_of: b.as_of,
            role: b.role.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(source: &str, price: f64, at: u64) -> AttributedPrice {
        AttributedPrice {
            source: source.into(),
            market_id: "ETH/USD".into(),
            mantissa: (price * 100.0).round() as i128,
            decimals: 2,
            observed_at: at,
            weight: 1.0,
        }
    }

    #[test]
    fn median_three_sources() {
        let obs = vec![
            tick("a", 100.0, 1000),
            tick("b", 110.0, 1000),
            tick("c", 102.0, 1000),
        ];
        let policy = AggregationPolicy {
            method: AggregationMethod::Median,
            min_sources: 2,
            max_age_secs: 60,
        };
        let b = aggregate_bounds("ETH/USD", &obs, &policy, 1000).unwrap();
        assert_eq!(b.role, "bound_only");
        assert_eq!(b.n_sources, 3);
        // median of 100, 102, 110 = 102.00 → mantissa 10200 @ 2 decimals
        assert_eq!(b.mantissa, 10200);
        assert_eq!(b.decimals, 2);
    }

    #[test]
    fn stale_dropped_insufficient() {
        let obs = vec![tick("a", 100.0, 1000), tick("b", 110.0, 100)];
        let policy = AggregationPolicy {
            method: AggregationMethod::Mean,
            min_sources: 2,
            max_age_secs: 60,
        };
        // now=1000 → b is stale
        let err = aggregate_bounds("ETH/USD", &obs, &policy, 1000).unwrap_err();
        assert!(matches!(
            err,
            AggregateError::InsufficientSources { need: 2, have: 1 }
        ));
    }

    #[test]
    fn min_max_envelope() {
        let obs = vec![
            tick("a", 99.0, 50),
            tick("b", 101.0, 50),
            tick("c", 100.0, 50),
        ];
        let min_p = AggregationPolicy {
            method: AggregationMethod::Min,
            ..Default::default()
        };
        let max_p = AggregationPolicy {
            method: AggregationMethod::Max,
            ..Default::default()
        };
        let lo = aggregate_bounds("ETH/USD", &obs, &min_p, 50).unwrap();
        let hi = aggregate_bounds("ETH/USD", &obs, &max_p, 50).unwrap();
        assert_eq!(lo.mantissa, 9900);
        assert_eq!(hi.mantissa, 10100);
    }

    #[test]
    fn provider_kind_default_is_state_root() {
        assert_eq!(ProviderKind::default(), ProviderKind::StateRoot);
        assert_eq!(ProviderKind::parse(""), Some(ProviderKind::StateRoot));
        assert_eq!(
            ProviderKind::parse("price_bound"),
            Some(ProviderKind::PriceBound)
        );
    }

    #[test]
    fn bound_root_roundtrip() {
        let root = encode_bound_root(8, 3450_1200_0000);
        assert_eq!(root.len(), BOUND_ROOT_LEN);
        let (d, m) = decode_bound_root(&root).unwrap();
        assert_eq!(d, 8);
        assert_eq!(m, 3450_1200_0000);
    }

    #[test]
    fn parse_decimal_str_basic() {
        assert_eq!(parse_decimal_str("3450.12", 2), Some(345012));
        assert_eq!(parse_decimal_str("3450.12", 8), Some(3450_1200_0000));
        assert_eq!(parse_decimal_str("1", 8), Some(100_000_000));
    }

    #[test]
    fn parse_ticker_json_shapes() {
        let a =
            parse_ticker_json(r#"{"price":"100.50"}"#, "binance", "ETH/USD", 1.0, 2, 999).unwrap();
        assert_eq!(a.mantissa, 10050);
        assert_eq!(a.market_id, "ETH/USD");
        assert_eq!(a.observed_at, 999);

        let b = parse_ticker_json(
            r#"{"market_id":"BTC/USD","price":50000,"timestamp":42}"#,
            "coinbase",
            "ETH/USD",
            1.0,
            0,
            0,
        )
        .unwrap();
        assert_eq!(b.market_id, "BTC/USD");
        assert_eq!(b.mantissa, 50000);
        assert_eq!(b.observed_at, 42);

        let c =
            parse_ticker_json(r#"{"data":{"price":"1.5"}}"#, "okx", "USDT/USD", 1.0, 2, 1).unwrap();
        assert_eq!(c.mantissa, 150);
    }

    #[test]
    fn attribute_store_aggregates_one_mid_per_market() {
        let store = OracleAttributeStore::new();
        let policy = AggregationPolicy {
            method: AggregationMethod::Median,
            min_sources: 2,
            max_age_secs: 120,
        };
        store
            .upsert_and_aggregate(tick("binance", 100.0, 1000), &policy, 1000)
            .unwrap_err(); // only 1 source
        store
            .upsert_and_aggregate(tick("coinbase", 110.0, 1000), &policy, 1000)
            .unwrap();
        let b = store
            .upsert_and_aggregate(tick("okx", 102.0, 1000), &policy, 1000)
            .unwrap();
        assert_eq!(b.mantissa, 10200);
        assert_eq!(b.n_sources, 3);
        assert_eq!(store.get_bound("ETH/USD").unwrap().mantissa, 10200);
        let resp = OracleBoundsResponse::from(&b);
        assert_eq!(resp.role, "bound_only");
        assert_eq!(resp.price, "102.00");
    }

    #[test]
    fn bound_to_ve_uses_price_algo() {
        let b = OracleBound {
            market_id: "ETH/USD".into(),
            method: AggregationMethod::Median,
            mantissa: 345012,
            decimals: 2,
            n_sources: 3,
            sources: vec!["a".into(), "b".into(), "c".into()],
            as_of: 1700,
            role: "bound_only".into(),
        };
        let ve = bound_to_vote_extension(&b, "sidecar-1", "price:ETH/USD", ALGO_PRICE_MEDIAN_V1);
        assert_eq!(ve.algo, ALGO_PRICE_MEDIAN_V1);
        assert_eq!(decode_bound_root(&ve.root), Some((2, 345012)));
        assert!(ve.ics23_proof.is_empty());
    }
}
