use crate::state::*;
use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::{Addr, Coin, Decimal};
use cw_infusions::{
    CompatibleTraits,
    bundles::{Bundle, BundleType},
    state::{EligibleNFTCollection, Infusion, InfusionState, UpdateInfusion},
    wavs::WavsBundle,
};

#[cw_serde]
pub struct InstantiateMsg {
    /// owner of this contract
    pub contract_owner: Option<String>,
    /// Fee from each infusion payment, if required. Goes to contract owner during any infusion. Set to 0.0 to disable.
    pub owner_fee: Decimal,
    /// Minimum fee that is required for creating an infusion
    pub min_creation_fee: Option<Coin>,
    /// Minimum fee that is required to be set when infusions occur
    pub min_infusion_fee: Option<Coin>,
    /// Minimum tokens required for any infusions eligible collections
    pub min_per_bundle: Option<u64>,
    /// Maximim tokens required for any infusions eligible collections
    pub max_per_bundle: Option<u64>,
    /// Maximum bundles any infusion is able to require
    pub max_bundles: Option<u64>,
    /// Maximum infusions that may be created at once
    pub max_infusions: Option<u64>,
    /// Code-ID of the cw721-collection
    pub cw721_code_id: u64,
    /// Optional key wavs operators make use of, if support is enabled
    pub wavs_public_key: Option<String>,
}

#[cw_serde]
#[cfg_attr(feature = "interface", derive(cw_orch::ExecuteFns))]
pub enum ExecuteMsg {
    /// Register a new infusion recipe and its collection.
    CreateInfusion {
        infusions: Vec<Infusion>,
    },
    /// Oracle/WAVS callback when the infusion is driven off-chain.
    WavsEntryPoint {
        infusions: Vec<WavsBundle>,
    },
    /// Submit an eligible NFT bundle and receive the infused token.
    Infuse {
        id: u64,
        bundle: Vec<Bundle>,
    },
    /// Close a recipe so no further infuses succeed.
    EndInfusion {
        id: u64,
    },
    /// Set metadata base URI for an infusion.
    UpdateInfusionBaseUri {
        id: u64,
        base_uri: String,
    },
    /// Global infuser config (contract owner).
    UpdateConfig {
        config: UpdatingConfig,
    },
    /// Change which collections or bundle shape a recipe accepts.
    UpdateInfusionsEligibleCollectionsOrBundleType {
        id: u64,
        to_add: Vec<EligibleNFTCollection>,
        to_remove: Vec<Addr>,
        bundle_type: BundleType,
    },
    /// Fee charged per infuse.
    UpdateInfusionMintFee {
        id: u64,
        mint_fee: Option<Coin>,
    },
    /// Other recipe parameters.
    UpdateInfusionParams {
        id: u64,
        params: UpdateInfusion,
    },
    /// Randomize remaining order when the recipe uses shuffle.
    Shuffle {
        id: u64,
    },
}

#[cw_serde]
#[cfg_attr(feature = "interface", derive(cw_orch::QueryFns))]
#[derive(QueryResponses)]
pub enum QueryMsg {
    /// Global infuser config.
    #[returns(Config)]
    Config {},
    /// One infusion by owner and id.
    #[returns(Infusion)]
    Infusion { addr: Addr, id: u64 },
    /// One infusion by id.
    #[returns(InfusionState)]
    InfusionById { id: u64 },
    /// Infusions owned by an address (page of 30 from `index`).
    #[returns(InfusionsResponse)]
    Infusions { addr: Addr, index: u64 },
    #[returns(Vec<CompatibleTraits>)]
    InfusionGenetics { id: u64 },

    #[returns(Vec<cw_infusions::wavs::WavsRecordResponse>)]
    WavsRecord {
        nfts: Vec<String>,
        burner: Option<Addr>,
    },
    // #[returns(Vec<WavsEligibleRes>)]
    // WavsEligibleCollection { nfts: Vec<String> },
}

#[cosmwasm_schema::cw_serde]
pub struct CountResponse {
    pub count: i32,
}

#[cosmwasm_schema::cw_serde]
pub struct InfusedCollectionParams {
    pub code_id: u64,
    pub name: String,
    pub symbol: String,
    pub admin: Option<String>,
}

#[cosmwasm_schema::cw_serde]
pub struct InfusionsResponse {
    pub infusions: Vec<InfusionState>,
}

#[cw_serde]
pub struct MigrateMsg {}
