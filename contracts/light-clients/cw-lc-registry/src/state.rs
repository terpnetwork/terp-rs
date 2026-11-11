use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Binary};
use cw_storage_plus::{Item, Map};

use crate::msg::{ClientKind, NodeState, TargetChain};
use crate::quorum::HashMarketCapability;

#[cw_serde]
pub struct Config {
    pub admin: Addr,
    pub label: String,
    pub target: String,
    pub hash_market_capability: Option<HashMarketCapability>,
    pub max_report_age_blocks: u64,
}

#[cw_serde]
pub struct OperatorInfo {
    pub operator: Addr,
    pub peer_id: String,
    pub endpoint: String,
    pub pubkey: Binary,
    pub capabilities: Vec<String>,
    /// Committee weight (FROST shares) set by the admin; 0 = not eligible.
    pub weight: u64,
    pub registered_height: u64,
}

#[cw_serde]
pub struct ClientRecord {
    pub client_id: String,
    pub owner: Addr,
    pub kind: ClientKind,
    pub target: TargetChain,
    pub attestors: Vec<Addr>,
    pub min_attestations: u32,
    pub ibc_client_id: Option<String>,
    pub created_height: u64,
}

#[cw_serde]
pub struct StoredReport {
    pub operator: Addr,
    pub state: NodeState,
    pub reported_height: u64,
}

pub const CONFIG: Item<Config> = Item::new("config");
pub const OPERATORS: Map<&Addr, OperatorInfo> = Map::new("operators");
pub const CLIENTS: Map<&str, ClientRecord> = Map::new("clients");
/// (client_id, operator) → latest report.
pub const REPORTS: Map<(&str, &Addr), StoredReport> = Map::new("reports");
