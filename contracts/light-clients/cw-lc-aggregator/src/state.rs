use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Binary};
use cw_storage_plus::{Item, Map};

use crate::msg::Header;

#[cw_serde]
pub struct Config {
    pub label: String,
    pub target: String,
    pub registry: Addr,
    pub rotation: Addr,
    pub client_id: String,
    /// Registry tip must be this many blocks above a preconf to confirm it.
    pub confirm_depth: u64,
    /// Active epoch with no preconf for this many Terp blocks is stalled.
    pub stall_blocks: u64,
    pub max_confirm_walk: u32,
}

#[cw_serde]
pub enum PreconfStatus {
    Preconfirmed,
    Confirmed,
}

#[cw_serde]
pub struct PreconfRecord {
    pub header: Header,
    pub epoch: u64,
    pub signature: Binary,
    pub submitted_at: u64,
    pub status: PreconfStatus,
}

#[cw_serde]
pub struct Equivocation {
    pub epoch: u64,
    pub height: u64,
    pub first: Header,
    pub first_sig: Binary,
    pub second: Header,
    pub second_sig: Binary,
    pub recorded_at: u64,
}

#[cw_serde]
pub struct Liveness {
    pub last_preconf_block: Option<u64>,
    pub latest_height: Option<u64>,
    pub latest_epoch: Option<u64>,
}

pub const CONFIG: Item<Config> = Item::new("config");
pub const PRECONFS: Map<u64, PreconfRecord> = Map::new("preconfs");
pub const EVIDENCE: Map<u64, Equivocation> = Map::new("evidence");
pub const LIVENESS: Item<Liveness> = Item::new("liveness");
