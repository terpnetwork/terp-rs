use cosmwasm_schema::cw_serde;
use cosmwasm_std::Empty;
use cw_storage_plus::{Item, Map};

/// Contract-level config (optional admin for future ops; not required for auth path).
#[cw_serde]
pub struct Config {
    pub admin: Option<String>,
}

pub const CONFIG: Item<Config> = Item::new("config");

/// Spent nullifiers keyed by (domain, session_id, nullifier_bytes).
/// Presence means spent. Value is Empty.
pub const SPENT: Map<(&str, &str, &[u8]), Empty> = Map::new("spent");

/// Build map key components from resolved scope.
pub fn spent_key<'a>(
    domain: &'a str,
    session_id: &'a str,
    nullifier: &'a [u8],
) -> (&'a str, &'a str, &'a [u8]) {
    (domain, session_id, nullifier)
}
