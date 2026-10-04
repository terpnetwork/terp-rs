use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Addr, Uint128};
use hm_loyalty_whitelist::{LoyaltyProof, WhitelistConfig};
use hm_nft_oracle::{MintPriceBound, OracleMid, PricePolicy, PriceSyncMode};

#[cw_serde]
pub struct InstantiateMsg {
    /// Optional owner (defaults to sender).
    pub owner: Option<String>,
    /// Existing CW721 (e.g. cw721-svg) address where this minter is authorized.
    /// If None, admin must call `LinkCollection` before mint.
    pub cw721_address: Option<String>,
    /// Hard supply cap. Oracle/sudo can NEVER increase this.
    pub max_tokens: u32,
    /// Base token URI / template prefix (collection-specific; optional).
    pub token_uri_prefix: Option<String>,
    /// Whitelist gate (hm-loyalty/v1).
    pub whitelist: WhitelistConfig,
    /// Oracle / fixed price policy.
    pub price: PricePolicy,
    /// Address that receives mint proceeds (defaults to owner).
    pub proceeds: Option<String>,
    /// Start paused (default false).
    pub paused: Option<bool>,
}

#[cw_serde]
#[cfg_attr(feature = "interface", derive(cw_orch::ExecuteFns))]
pub enum ExecuteMsg {
    /// Mint next token to sender if whitelist + price checks pass.
    Mint {
        /// Merkle proof when whitelist.enabled.
        proof: Option<LoyaltyProof>,
        /// Optional token_uri override; otherwise `{prefix}{token_id}`.
        token_uri: Option<String>,
    },

    /// Admin: set loyalty root + codec.
    SetWhitelistRoot {
        root: String,
        codec: Option<String>,
        enabled: Option<bool>,
    },

    /// Admin: update price policy (bps, staleness, fallback, filters).
    SetPricePolicy { price: PricePolicy },

    /// Mode C only: admin-pushed mid (demo fallback).
    AdminSyncPrice {
        mid: Uint128,
        foreign_height: Option<u64>,
        asset_id: Option<String>,
    },

    /// Admin: attach CW721 collection (once).
    LinkCollection { cw721_address: String },

    /// Admin: pause/unpause minting.
    SetPaused { paused: bool },

    /// Admin: update proceeds recipient.
    SetProceeds { proceeds: String },

    /// Owner: withdraw native proceeds to `proceeds` (or specified recipient).
    Withdraw { amount: Option<Uint128> },

    /// cw-ownable ownership update.
    UpdateOwnership(cw_ownable::Action),
}

/// Sudo: only HashMerchant envelope. Never mutates max_tokens.
pub type SudoMsg = hm_nft_oracle::HashMerchantSudoMsg;

#[cw_serde]
#[derive(QueryResponses)]
#[cfg_attr(feature = "interface", derive(cw_orch::QueryFns))]
pub enum QueryMsg {
    #[returns(ConfigResponse)]
    Config {},

    #[returns(MintPriceBound)]
    MintPriceBound {},

    #[returns(WhitelistRootResponse)]
    WhitelistRoot {},

    #[returns(IsEligibleResponse)]
    IsEligible {
        address: String,
        proof: LoyaltyProof,
    },

    #[returns(NextTokenIdResponse)]
    NextTokenId {},

    #[returns(OracleMidResponse)]
    OracleMid {},

    #[returns(cw_ownable::Ownership<Addr>)]
    Ownership {},
}

#[cw_serde]
pub struct ConfigResponse {
    pub owner: Option<Addr>,
    pub cw721_address: Option<Addr>,
    pub max_tokens: u32,
    pub next_token_id: u32,
    pub token_uri_prefix: Option<String>,
    pub whitelist: WhitelistConfig,
    pub price: PricePolicy,
    pub proceeds: Addr,
    pub paused: bool,
}

#[cw_serde]
pub struct WhitelistRootResponse {
    pub enabled: bool,
    pub codec: String,
    pub root: Option<String>,
}

#[cw_serde]
pub struct IsEligibleResponse {
    pub eligible: bool,
    pub error: Option<String>,
}

#[cw_serde]
pub struct NextTokenIdResponse {
    pub next_token_id: u32,
    pub max_tokens: u32,
}

#[cw_serde]
pub struct OracleMidResponse {
    pub mid: Option<OracleMid>,
    pub mode: PriceSyncMode,
}

// Re-export for schema consumers.
pub use hm_nft_oracle::HashMerchantSudoMsg;
