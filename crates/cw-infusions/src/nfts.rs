use cosmwasm_std::{Addr, Decimal, Timestamp};

#[cosmwasm_schema::cw_serde]
pub struct NFT {
    pub addr: Addr,
    pub token_id: u64,
}

#[cosmwasm_schema::cw_serde]
#[derive(Default)]
pub struct InfusedCollection {
    pub sg: bool,
    pub admin: Option<String>,
    pub name: String,
    /// infused collection description
    pub description: String,
    /// symbol of infused collection
    pub symbol: String,
    /// ipfs base uri containing metadata and nft images. ensure ipfs:// prefix is included.
    pub base_uri: String,
    /// cover image of infused collection.
    pub image: String,
    /// total supply.
    pub num_tokens: u32,
    /// royality params for secondary market sales.
    pub royalty_info: Option<RoyaltyInfoResponse>,
    /// time in which trading can begin of infused collection.
    pub start_trading_time: Option<Timestamp>,
    /// whether explicit content is present.
    pub explicit_content: Option<bool>,
    /// optional external link.
    pub external_link: Option<String>,
    /// exists to reuse InfusedCollection struct in contract.
    /// value is disregarded if present in new infusion creation msg.
    pub addr: Option<String>,
}

#[cosmwasm_schema::cw_serde]
pub struct RoyaltyInfoResponse {
    pub payment_address: String,
    pub share: Decimal,
}

#[cosmwasm_schema::cw_serde]
pub struct CollectionInfo<T> {
    pub creator: String,
    pub description: String,
    pub image: String,
    pub external_link: Option<String>,
    pub explicit_content: Option<bool>,
    pub start_trading_time: Option<Timestamp>,
    pub royalty_info: Option<T>,
}

#[cosmwasm_schema::cw_serde]
pub struct SgInstantiateMsg {
    pub name: String,
    pub symbol: String,
    pub minter: String,
    pub collection_info: CollectionInfo<RoyaltyInfoResponse>,
}
