use cosmwasm_std::{Addr, Binary};
use cw_controllers::Admin;
use cw_storage_plus::{Item, Map};

pub type TokenUri = Addr;
pub type TokenId = String;

/// maps other bech32 address to terp addresses
pub const REVERSE_MAP_KEY: Map<&String, Binary> = Map::new("rmk");
pub const REVMAP_LIMIT: Map<&String, u32> = Map::new("rmkl");

/// Address of the text record verification oracle
pub const REVERSE_MAP: Map<&TokenUri, TokenId> = Map::new("rm");
pub const VERIFIER: Admin = Admin::new("v");
pub const SUDO_PARAMS: Item<SudoParams> = Item::new("sp");
pub const ACCOUNT_MANIFOLD: Item<Addr> = Item::new("am");

#[cosmwasm_schema::cw_serde]
pub struct SudoParams {
    pub max_record_count: u32,
    /// maximum # of items a `terp1...` addr can map to `REVERSE_MAP_KEY`
    pub max_reverse_map_key_limit: u32,
    // pub registry_addr: Addr,
}
