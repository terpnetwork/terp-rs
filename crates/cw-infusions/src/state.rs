use cosmwasm_std::{Addr, Coin};

use crate::{bundles::BundleType, nfts::InfusedCollection, BurnParams};

#[cosmwasm_schema::cw_serde]
pub struct UpdateInfusion {
    /// Owner of the infusion.
    /// Optional description of this infusion
    pub infusion_description: Option<String>,
    pub owner: Option<Addr>,
    pub payment_recipient: Option<Addr>,
    pub infused_collection_state: Option<String>, // / NFT collections eligible for a specific infusion
                                                  // pub collections: Vec<EligibleNFTCollection>,
                                                  // / Parameters of a specific infusion
                                                  // pub infusion_params: Option<InfusionParamState>,
}

#[cosmwasm_schema::cw_serde]
pub struct Infusion {
    /// Optional description of this infusion
    pub description: Option<String>,
    /// Owner of the infusion.
    pub owner: Option<Addr>,
    /// NFT collections eligible for a specific infusion
    pub collections: Vec<EligibleNFTCollection>,
    /// Current data of the new infused collection
    pub infused_collection: InfusedCollection,
    /// Parameters of a specific infusion
    pub infusion_params: InfusionParamState,
    pub payment_recipient: Option<Addr>,
}

#[cosmwasm_schema::cw_serde]
pub struct InfusionState {
    pub enabled: bool,
    pub owner: Addr,
    /// NFT collections eligible for a specific infusion
    pub collections: Vec<EligibleNFTCollection>,
    /// Current data of the new infused collection
    pub infused_collection: InfusedCollection,
    /// Parameters of a specific infusion
    pub infusion_params: InfusionParamState,
    pub payment_recipient: Addr,
}

#[cosmwasm_schema::cw_serde]
pub struct InfusionParamState {
    pub bundle_type: BundleType,
    /// required alongside a bundle. Sent to infusion owner
    pub mint_fee: Option<Coin>,
    pub params: Option<BurnParams>,
    pub wavs_enabled: bool,
}

#[cosmwasm_schema::cw_serde]
pub struct EligibleNFTCollection {
    /// collection address
    pub addr: Addr,
    /// Minimum tokens required to infuse
    pub min_req: u64,
    /// Optional, maximum tokens able to be infused.
    ///  If not set, contract expects exact # of min_req per collection in bundle.
    pub max_req: Option<u64>,
    /// If set, infuser can send exact amount of tokens to replace eligil
    pub payment_substitute: Option<Coin>,
}

impl PartialEq<String> for EligibleNFTCollection {
    fn eq(&self, other: &String) -> bool {
        self.addr.to_string().eq(other)
    }
}
