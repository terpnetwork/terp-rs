use cosmwasm_schema::cw_serde;
use cosmwasm_std::Binary;
use cw_storage_plus::{Item, Map};

#[cw_serde]
pub struct Config {
    pub admin: Option<String>,
}

#[cw_serde]
pub struct SessionMeta {
    pub latest_height: u64,
    pub latest_root: Binary,
}

pub const CONFIG: Item<Config> = Item::new("config");

/// anchor/{session_id}/{height} → root[32]
pub const ANCHORS: Map<(&str, u64), Binary> = Map::new("anchor");

/// session/{session_id}/meta
pub const SESSION_META: Map<&str, SessionMeta> = Map::new("session");

pub const ROOT_LEN: usize = 32;
