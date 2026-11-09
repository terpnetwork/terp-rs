use crate::{nfts::NFT, state::EligibleNFTCollection};
use cosmwasm_std::Addr;

#[cosmwasm_schema::cw_serde]
pub struct Bundle {
    pub nfts: Vec<NFT>,
}

#[cosmwasm_schema::cw_serde]
pub enum BundleType {
    // Requires the minimum for all eligible collections to be included for a bundle to be accepted.
    AllOf {},
    // Any of is a list of bundles that if have their minimum provided, will be accepted.
    AnyOf { addrs: Vec<Addr> },
    // A mapping of possible combinations of eligible collections and required nfts that will be accepted.
    AnyOfBlend { blends: Vec<BundleBlend> },
}

impl Default for BundleType {
    fn default() -> Self {
        BundleType::AllOf {}
    }
}
impl BundleType {
    pub fn strain(&self) -> i32 {
        match self {
            BundleType::AllOf { .. } => 1,
            BundleType::AnyOf { .. } => 2,
            BundleType::AnyOfBlend { .. } => 3,
        }
    }
}

#[cosmwasm_schema::cw_serde]
pub struct BundleBlend {
    pub blend_nfts: Vec<BlendNFTs>,
}

#[cosmwasm_schema::cw_serde]
pub struct BlendNFTs {
    pub addr: Addr,
    pub min_req: u64,
    pub payment_substitute: bool,
}

#[cosmwasm_schema::cw_serde]
pub struct AnyOfCount {
    pub nft: EligibleNFTCollection,
    pub count: u64,
}
