use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Coin, Decimal};
use cw_storage_plus::{Item, Map};

#[cw_serde]
pub struct Config {
    pub admin: Addr,
    pub open_submit: bool,
    pub bounty: Option<Coin>,
    /// Polytone note allowed to deliver `Callback`. None = lab (any sender).
    pub note: Option<Addr>,
    /// This chain's IBC v2 client for the dex-quote counterparty.
    #[serde(default)]
    pub ibc2_source_client: Option<String>,
    /// Destination port. `Some("dex-quote")` after `RegisterIbcV2`.
    #[serde(default)]
    pub ibc2_dest_port: Option<String>,
}

#[cw_serde]
pub enum SourceKind {
    /// Sidecar / IBC packet processor posted the Osmosis query result.
    Relayer,
    /// Polytone callback wrote the same schema.
    Polytone,
    /// x/hashmerchant vote-extension quorum (`price_median_v1`).
    VeQuorum,
    /// IBC v2 ack from port `dex-quote`. Posted liquidity, not a fill.
    DexQuote,
}

#[cw_serde]
pub struct Observation {
    pub market_id: String,
    pub source: SourceKind,
    pub source_chain: String,
    pub pool_id: Option<u64>,
    pub height: u64,
    pub mid: Decimal,
    pub recorder: Addr,
    /// Set by a dex-quote ack. Absent for Polytone, relayer, and vote-extension rows.
    #[serde(default)]
    pub position_id: Option<String>,
}

pub const CONFIG: Item<Config> = Item::new("cfg");
/// (market_id, height) → observation. First writer wins.
pub const BY_HEIGHT: Map<(&str, u64), Observation> = Map::new("h");
/// Latest height per market.
pub const HEAD: Map<&str, u64> = Map::new("hd");
pub const RELAYERS: Map<&str, bool> = Map::new("rel");
/// Quote requested at `(market_id, height)` and not yet acked or timed out.
pub const PENDING_QUOTE: Map<(&str, u64), crate::msg::DexQuoteRequest> = Map::new("pq");
