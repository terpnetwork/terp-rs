use cosmwasm_std::{Addr, Coin, Decimal, HexBinary};
use cw_infusions::state::InfusionState;
use cw_storage_plus::{Item, Map};

/// Global contract config
pub const CONFIG: Item<Config> = Item::new("cfg");
/// Map of the infusion id with the infused collection addr
pub const INFUSION_ID: Map<u64, (Addr, u64)> = Map::new("infusion_id");
/// infusions saved to map with key of (infused_collection_addr, infusion_id )
pub const INFUSION: Map<(Addr, u64), InfusionState> = Map::new("infusion");
// map of infusion id's a given nft collection is eligible for. Used by WAVS service to filter trigger events.
pub const ELIGIBLE_COLLECTION: Map<&String, Vec<u64>> = Map::new("eligible-collections");
// map of index position and token id
pub const MINT_COUNT: Item<u64> = Item::new("mtc");
/// map of minting positions for infusions:  (infusion_id,mint_position), token-id
pub const MINTABLE_TOKEN_VECTORS: Map<u64, Vec<u32>> = Map::new("mt_vectors");
/// Number of mintable tokens for a given infused NFT collection
pub const MINTABLE_NUM_TOKENS: Map<String, u32> = Map::new("mnt");
/// map to count tokens burnt for (token_burner, collection addr) as key.
pub const WAVS_TRACKED: Map<(&Addr, String), u64> = Map::new("wt");

pub const WAVS_ADMIN: Item<String> = Item::new("wavs_admin");
/// Global con
pub const COUNT: Item<i32> = Item::new("count");
/// map of infusion
// pub const INFUSION_CREATOR: Map<u64, Addr> = Map::new("ic");

#[cosmwasm_schema::cw_serde]
pub struct Config {
    // Default at 0.
    pub latest_infusion_id: u64,
    pub contract_owner: Addr,
    /// Fee required to contribute to randomness
    // pub shuffle_fee: Option<Coin>,
    /// % Fee from any infusion fee set to go to contract owner. 10 == 10% , 71 == 71%
    pub owner_fee: Decimal,
    /// Minimum fee that is required for creating an infusion.
    pub min_creation_fee: Option<Coin>,
    /// Minimum fee that is required to be set when new infusions are being created
    pub min_infusion_fee: Option<Coin>,
    /// Maximum unique infusion that can be created at once. Defaults to 2
    pub max_infusions: u64,
    /// Contract global param enforcing minimum nfts each collection in an infusion must require to burn. hard coded to 1.
    pub min_per_bundle: u64,
    /// Contract global param enforcing maximum nfts bundles can require.
    pub max_per_bundle: u64,
    /// maximum bundles allowed per infusion
    pub max_bundles: u64,
    /// cw721-base code_id
    pub code_id: u64,
    /// code hash of cw721. used for instantitate2 during infusion creation.
    pub code_hash: HexBinary,
}

#[cosmwasm_schema::cw_serde]
pub struct UpdatingConfig {
    pub contract_owner: Option<String>,
    pub owner_fee: Option<Decimal>,
    pub min_creation_fee: Option<Coin>,
    pub min_infusion_fee: Option<Coin>,
    pub max_infusions: Option<u64>,
    pub min_per_bundle: Option<u64>,
    pub max_bundles: Option<u64>,
    pub code_id: Option<u64>,
}

#[cosmwasm_schema::cw_serde]
pub struct TokenPositionMapping {
    pub position: u32,
    pub token_id: u32,
}
