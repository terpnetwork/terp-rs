//! Config, feed and response types. All are `cw_serde`, so they can sit in
//! contract messages and state.

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Api, Coin, Decimal, Decimal256, Uint256};

use crate::error::OracleError;

/// Largest decimals value accepted on either side of a pair.
pub const MAX_DECIMALS: u8 = 24;
/// Largest number of feeds behind one alt asset.
pub const MAX_FEEDS: usize = 8;
/// Largest number of alt assets in one config.
pub const MAX_ALT_ASSETS: usize = 16;
/// Largest number of pairs in one `OracleAggregate` query.
pub const MAX_AGGREGATE_PAIRS: usize = 16;

/// An asset priced by the oracle: a native/IBC/factory denom or a cw20 address.
#[cw_serde]
#[derive(Eq, PartialOrd, Ord, Hash)]
pub enum PriceAsset {
    Native(String),
    Cw20(String),
}

impl PriceAsset {
    pub fn native(denom: impl Into<String>) -> Self {
        Self::Native(denom.into())
    }
    pub fn cw20(addr: impl Into<String>) -> Self {
        Self::Cw20(addr.into())
    }
    pub fn as_native(&self) -> Option<&str> {
        match self {
            Self::Native(d) => Some(d),
            Self::Cw20(_) => None,
        }
    }
}

impl std::fmt::Display for PriceAsset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Native(d) => write!(f, "native:{d}"),
            Self::Cw20(a) => write!(f, "cw20:{a}"),
        }
    }
}

/// How several values are combined into one.
#[cw_serde]
#[derive(Copy, Eq)]
pub enum AggregateMethod {
    Median,
    /// Weighted by `Feed::weight` (default 1) when combining feeds, and by
    /// nothing when combining a history window.
    Mean,
    /// Weighted by observation count (`PricePoint::observations`) when combining feeds,
    /// and by block gap (piecewise constant, like the registry TWAP) on a history window.
    Weighted,
    Min,
    Max,
}

/// What to read from one oracle contract.
#[cw_serde]
pub enum FeedKind {
    /// Registry `Latest { market_id }`.
    Spot { market_id: String },
    /// Registry `Twap` over the newest `window_blocks` heights ending at the latest observation.
    Twap {
        market_id: String,
        window_blocks: u64,
    },
    /// Registry `History { limit }`, combined locally with `method`.
    History {
        market_id: String,
        limit: u32,
        method: AggregateMethod,
    },
    /// Aggregator `Median { market_id, window }`. The height comes from the
    /// aggregator's registry `Latest`, so staleness checks still apply.
    AggregatorMedian { market_id: String, window: u32 },
    /// TWAP probe `ArithmeticTwap { pool_id }`. Carries no height.
    ProbeTwap { pool_id: u64 },
}

impl FeedKind {
    pub fn label(&self) -> String {
        match self {
            FeedKind::Spot { market_id } => format!("spot:{market_id}"),
            FeedKind::Twap {
                market_id,
                window_blocks,
            } => format!("twap:{market_id}:{window_blocks}"),
            FeedKind::History {
                market_id,
                limit,
                method,
            } => format!("history:{market_id}:{limit}:{method:?}").to_lowercase(),
            FeedKind::AggregatorMedian { market_id, window } => {
                format!("median:{market_id}:{window}")
            }
            FeedKind::ProbeTwap { pool_id } => format!("probe:{pool_id}"),
        }
    }
}

/// One oracle source.
#[cw_serde]
pub struct Feed {
    /// Oracle contract address (registry, aggregator or probe, matching `kind`).
    pub oracle: String,
    pub kind: FeedKind,
    /// Weight for `AggregateMethod::Mean` across feeds. Default 1.
    #[serde(default)]
    pub weight: Option<u32>,
}

/// Direction of the feed price.
#[cw_serde]
#[derive(Copy, Eq)]
pub enum Quote {
    /// Feed value = base units per 1 alt (in display units). E.g. feed
    /// `ATOM/TERP = 12.5`, base TERP, alt ATOM: 1 ATOM costs 12.5 TERP.
    AltInBase,
    /// Feed value = alt units per 1 base (in display units).
    BaseInAlt,
}

/// An alternate asset accepted in place of `base_denom`.
#[cw_serde]
pub struct AltAsset {
    pub asset: PriceAsset,
    /// The default mint asset this alt is priced against.
    pub base_denom: String,
    pub feeds: Vec<Feed>,
    /// How feed values are combined. Default median.
    #[serde(default = "default_method")]
    pub combine: AggregateMethod,
    pub quote: Quote,
    /// Decimals of the base denom (e.g. 6 for uterp).
    pub base_decimals: u8,
    /// Decimals of the alt asset.
    pub alt_decimals: u8,
}

fn default_method() -> AggregateMethod {
    AggregateMethod::Median
}

/// Guards applied to every price read.
#[cw_serde]
#[derive(Default)]
pub struct PricingPolicy {
    /// Max `env.block.height - observation.height`. Requires feeds that record
    /// local (terp) heights. `None` disables the check.
    #[serde(default)]
    pub max_staleness_blocks: Option<u64>,
    /// Min number of feeds that must answer. Default 1.
    #[serde(default)]
    pub min_sources: Option<u32>,
    /// Min total observations behind the combined price. Default none.
    #[serde(default)]
    pub min_observations: Option<u32>,
    /// Max relative spread between any feed and the combined price, e.g. 0.05.
    #[serde(default)]
    pub max_deviation: Option<Decimal>,
    /// Accepted underpayment against the quote, e.g. 0.01 = pay at least 99%.
    /// Overpayment is refunded.
    #[serde(default)]
    pub slippage: Option<Decimal>,
}

/// Optional oracle pricing for a minting contract.
///
/// Absent, or `enforce == false`: only the default asset is accepted and
/// payment behaves as before. The oracle queries still answer, so a frontend
/// can show prices before enforcement is switched on.
#[cw_serde]
pub struct OraclePricing {
    pub enforce: bool,
    pub alt_assets: Vec<AltAsset>,
    #[serde(default)]
    pub policy: PricingPolicy,
}

impl OraclePricing {
    /// Validate addresses and bounds. Call on instantiate and on update.
    pub fn validate(&self, api: &dyn Api) -> Result<(), OracleError> {
        let bad = |s: String| Err(OracleError::InvalidConfig(s));
        if self.alt_assets.len() > MAX_ALT_ASSETS {
            return bad(format!("at most {MAX_ALT_ASSETS} alt assets"));
        }
        let mut seen = std::collections::BTreeSet::new();
        for a in &self.alt_assets {
            match &a.asset {
                PriceAsset::Native(d) if d.is_empty() => return bad("empty alt denom".into()),
                PriceAsset::Native(d) if *d == a.base_denom => {
                    return bad(format!("alt {d} equals its base denom"));
                }
                PriceAsset::Cw20(addr) => {
                    api.addr_validate(addr)?;
                }
                _ => {}
            }
            if a.base_denom.is_empty() {
                return bad("empty base denom".into());
            }
            if !seen.insert((a.asset.clone(), a.base_denom.clone())) {
                return bad(format!("duplicate pair {} / {}", a.asset, a.base_denom));
            }
            if a.feeds.is_empty() {
                return bad(format!("{} has no feeds", a.asset));
            }
            if a.feeds.len() > MAX_FEEDS {
                return bad(format!("at most {MAX_FEEDS} feeds per asset"));
            }
            if a.base_decimals > MAX_DECIMALS || a.alt_decimals > MAX_DECIMALS {
                return bad(format!("decimals above {MAX_DECIMALS}"));
            }
            for f in &a.feeds {
                api.addr_validate(&f.oracle)?;
                if f.weight == Some(0) {
                    return bad("feed weight 0".into());
                }
                match &f.kind {
                    FeedKind::Spot { market_id }
                    | FeedKind::Twap { market_id, .. }
                    | FeedKind::History { market_id, .. }
                    | FeedKind::AggregatorMedian { market_id, .. }
                        if market_id.is_empty() =>
                    {
                        return bad("empty market_id".into());
                    }
                    FeedKind::Twap {
                        window_blocks: 0, ..
                    } => {
                        return bad("twap window_blocks 0".into());
                    }
                    FeedKind::History { limit: 0, .. }
                    | FeedKind::AggregatorMedian { window: 0, .. } => {
                        return bad("history/median window 0".into());
                    }
                    _ => {}
                }
            }
            let need = self.policy.min_sources.unwrap_or(1);
            if need as usize > a.feeds.len() {
                return bad(format!(
                    "{} has {} feeds but min_sources is {need}",
                    a.asset,
                    a.feeds.len()
                ));
            }
        }
        if self.policy.min_sources == Some(0) {
            return bad("min_sources 0".into());
        }
        if let Some(d) = self.policy.max_deviation
            && d > Decimal::one()
        {
            return bad("max_deviation above 1".into());
        }
        if let Some(s) = self.policy.slippage
            && s >= Decimal::one()
        {
            return bad("slippage must be below 1".into());
        }
        Ok(())
    }

    pub fn find(&self, asset: &PriceAsset, base_denom: &str) -> Option<&AltAsset> {
        self.alt_assets
            .iter()
            .find(|a| &a.asset == asset && a.base_denom == base_denom)
    }

    /// Every oracle address referenced by the config.
    pub fn oracles(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .alt_assets
            .iter()
            .flat_map(|a| a.feeds.iter().map(|f| f.oracle.clone()))
            .collect();
        v.sort();
        v.dedup();
        v
    }
}

/// One feed's answer.
#[cw_serde]
pub struct PricePoint {
    pub feed: String,
    pub oracle: String,
    pub value: Decimal256,
    /// Height of the newest observation, when the oracle exposes one.
    pub height: Option<u64>,
    /// Observations behind this value (1 for spot).
    pub observations: u32,
    pub weight: u32,
}

/// Combined price for one pair.
#[cw_serde]
pub struct PriceResponse {
    pub asset: PriceAsset,
    pub base_denom: String,
    pub quote: Quote,
    pub method: AggregateMethod,
    /// Combined value in the feed direction (`quote`).
    pub price: Decimal256,
    /// Base units (display) per 1 alt, whatever the feed direction.
    pub alt_in_base: Decimal256,
    pub sources: u32,
    pub observations: u32,
    pub oldest_height: Option<u64>,
    pub newest_height: Option<u64>,
    /// Largest relative spread of a feed from `price`.
    pub max_deviation: Decimal256,
    pub points: Vec<PricePoint>,
}

#[cw_serde]
pub struct AssetPair {
    pub asset: PriceAsset,
    pub base_denom: String,
}

#[cw_serde]
pub struct AggregateEntry {
    pub pair: AssetPair,
    pub price: Option<PriceResponse>,
    pub error: Option<String>,
}

/// Prices for a set of pairs.
#[cw_serde]
pub struct AggregateResponse {
    pub height: u64,
    pub entries: Vec<AggregateEntry>,
}

/// Quote for paying `base` with `asset`.
#[cw_serde]
pub struct QuoteResponse {
    pub base: Coin,
    pub asset: PriceAsset,
    /// Alt amount (raw units) that settles `base`, rounded up.
    pub required: Uint256,
    /// Smallest alt amount accepted after `policy.slippage`.
    pub minimum: Uint256,
    pub enforced: bool,
    pub price: PriceResponse,
}
