//! Gate intentionally has **no** SPENT map.
//! Optional future: non-SSOT debug counters only.

use cosmwasm_schema::cw_serde;
use cw_storage_plus::Item;

#[cw_serde]
pub struct Config {
    pub note: String,
}

pub const CONFIG: Item<Config> = Item::new("config");

/// Namespace on ceremony module for spent map (must match cw-vote-ceremony).
pub const CEREMONY_SPENT_NAMESPACE: &str = "spent";
