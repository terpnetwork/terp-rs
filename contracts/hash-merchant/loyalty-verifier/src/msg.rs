use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::Uint128;

// -- Instantiate --

#[cw_serde]
pub struct InstantiateMsg {
    /// Optional tokenfactory (or bank) denom used when minting on private claim.
    /// Example: `factory/terp1…/loyaltypts`
    #[serde(default)]
    pub reward_denom: Option<String>,
    /// When true and `reward_denom` is set, private claims emit a mint/send settlement.
    #[serde(default)]
    pub mint_enabled: bool,
}

// -- Execute --

#[cw_serde]
pub enum ExecuteMsg {
    /// Public path (existing loyalty_rewards e2e): claim keyed by claimer address.
    ClaimRewards {
        leaf_hash: String,
        proof: Vec<ProofStep>,
        root_index: u32,
    },
    /// Private path (zk-jwt smart-account elevation):
    /// - Merkle proves inventory leaf under attested root
    /// - `nullifier` is the zk-jwt spend key (anti double-claim, unlinkable to OIDC sub)
    /// - `amount` is settlement size (points → factory denom)
    /// - `info.sender` should be the smart account (not a named customer)
    ClaimRewardsPrivate {
        leaf_hash: String,
        proof: Vec<ProofStep>,
        root_index: u32,
        /// Hex nullifier from terp-zkjwt codec (64 hex chars preferred).
        nullifier: String,
        /// Amount to mint/send to `info.sender`.
        amount: Uint128,
        /// Optional action bind: hex SHA-256 of
        /// `loyalty-claim/v1 || root_index_be || amount || leaf_hash || dest`
        /// so a zk-jwt session cannot be repurposed for a different claim action.
        #[serde(default)]
        action_bind: Option<String>,
    },
    /// Admin: update mint settings (owner = instantiate sender).
    UpdateMintConfig {
        reward_denom: Option<String>,
        mint_enabled: bool,
    },
}

#[cw_serde]
pub struct ProofStep {
    pub sibling: String,
    pub is_right: bool,
}

// -- Sudo (from hashmerchant module) --

#[cw_serde]
pub enum SudoMsg {
    HashMerchant {
        chain_uid: String,
        algo: String,
        root: String,
        height: u64,
        #[serde(default)]
        attestation_count: u32,
        #[serde(default)]
        block_time: i64,
    },
}

// -- Query --

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(RootResponse)]
    GetRoot { chain_uid: String, algo: String },

    #[returns(RootCountResponse)]
    GetRootCount {},

    #[returns(RootResponse)]
    GetRootByIndex { index: u32 },

    #[returns(ClaimResponse)]
    GetClaim { address: String, root_index: u32 },

    /// Private path: was this zk-jwt nullifier already used for root_index?
    #[returns(NullifierClaimResponse)]
    GetNullifierClaim { nullifier: String, root_index: u32 },

    #[returns(MintConfigResponse)]
    GetMintConfig {},
}

// -- Response types --

#[cw_serde]
pub struct RootResponse {
    pub chain_uid: String,
    pub algo: String,
    pub root: String,
    pub height: u64,
    pub index: u32,
}

#[cw_serde]
pub struct RootCountResponse {
    pub count: u32,
}

#[cw_serde]
pub struct ClaimResponse {
    pub claimed: bool,
    pub leaf_hash: String,
    pub root_index: u32,
    pub claimer: String,
}

#[cw_serde]
pub struct NullifierClaimResponse {
    pub claimed: bool,
    pub nullifier: String,
    pub root_index: u32,
    /// Smart-account (or EOA) that executed the claim — not OIDC identity.
    pub claimer: String,
    pub amount: String,
}

#[cw_serde]
pub struct MintConfigResponse {
    pub reward_denom: Option<String>,
    pub mint_enabled: bool,
    pub admin: String,
}
