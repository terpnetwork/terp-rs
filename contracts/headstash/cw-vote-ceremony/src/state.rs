use cosmwasm_schema::cw_serde;
use cosmwasm_std::Empty;
use cw_storage_plus::{Item, Map};

use crate::msg::{CeremonyStatus, RegistrationGate};

#[cw_serde]
pub struct Config {
    pub admin: Option<String>,
    pub max_nullifiers_per_mark: u32,
}

#[cw_serde]
pub struct Ceremony {
    pub domain: String,
    pub status: CeremonyStatus,
    pub registration_open: bool,
    pub anchor_policy: Option<String>,
    #[serde(default)]
    pub registration_gate: RegistrationGate,
    #[serde(default)]
    pub clerk: Option<String>,
    #[serde(default)]
    pub membership_module: Option<String>,
    #[serde(default)]
    pub vm_verifier: Option<String>,
}

/// Clerk-attested leaf: unbound until first Register consumes it for one addr.
#[cw_serde]
pub struct ClerkLeaf {
    pub consumed_by: Option<String>,
}

pub const CONFIG: Item<Config> = Item::new("config");

/// ceremonies[session_id] → Ceremony
pub const CEREMONIES: Map<&str, Ceremony> = Map::new("ceremony");

/// registrations[session_id][addr] → leaf_commit (empty string if Open gate / no leaf)
pub const REGISTRATIONS: Map<(&str, &str), String> = Map::new("reg");

/// clerk_leaves[session_id][leaf_commit] → ClerkLeaf
pub const CLERK_LEAVES: Map<(&str, &str), ClerkLeaf> = Map::new("clerk_leaf");

/// spent[domain][session_id][nullifier] → Empty
pub const SPENT: Map<(&str, &str, &[u8]), Empty> = Map::new("spent");

pub fn spent_key<'a>(
    domain: &'a str,
    session_id: &'a str,
    nullifier: &'a [u8],
) -> (&'a str, &'a str, &'a [u8]) {
    (domain, session_id, nullifier)
}
