#[cosmwasm_schema::cw_serde]
pub struct WavsBundle {
    /// Address of the account that infused the nfts
    pub infuser: String,
    /// Address of nft that was infused
    pub nft_addr: String,
    /// nft-id's of nfts that were infused
    pub infused_ids: Vec<String>,
}

/// An bech32 address and its burn count
#[cosmwasm_schema::cw_serde]
pub struct WavsRecordRequest {
    /// contract address of infuser
    pub infuser: String,
    /// optional burner to query  how many nfts have already been  burnt by this address
    pub burner: Option<String>,
}

/// An bech32 address and its burn count
#[cosmwasm_schema::cw_serde]
pub struct WavsRecordResponse {
    // burner or nft contract
    pub addr: String,
    // count of nfts  burned for specific burner, or will be `None`,
    // if nft contract is not eligible for any contract
    pub count: Option<u64>,
}

/// Response on if a given nft collection is eligible for one of existing infusions.
/// This is for the wavs services to query & filter out performing state transitions on unregistered nft collections.
#[cosmwasm_schema::cw_serde]
pub struct WavsEligibleRes {
    pub addr: String,
    pub exists: bool,
}

#[cosmwasm_schema::cw_serde]
pub struct WavsMintCountResponse {
    pub to_mint: u64,
    pub remaining: u64,
}
