use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;
use cw_storage_plus::{Item, Map};

#[cw_serde]
pub struct Config {
    pub label: String,
    pub target: String,
    pub registry: Addr,
    pub aggregator: Addr,
    /// Client id inside `cw-lc-registry`.
    pub registry_client_id: String,
    /// IBC v2 client id packets must name as `dest_client`.
    pub ibc_client_id: String,
    /// Accept inclusion against Preconfirmed headers (`false` = Confirmed only).
    pub accept_preconfirmed: bool,
}

#[cw_serde]
pub struct Inclusion {
    pub app_data_hash: String,
    pub height: u64,
    pub block_hash: String,
    pub payment_root: String,
    pub leaf: String,
    pub source_client: String,
    pub sequence: u64,
    pub note: String,
    pub amount_zat: u64,
    pub frost_group: String,
    pub recorded_at: u64,
}

pub const CONFIG: Item<Config> = Item::new("config");
/// app_data_hash → inclusion.
pub const INCLUSIONS: Map<&str, Inclusion> = Map::new("inclusions");
/// (source_client, sequence) → app_data_hash (replay guard).
pub const PACKETS: Map<(&str, u64), String> = Map::new("packets");
