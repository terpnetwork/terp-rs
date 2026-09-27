use crate::state::{Epoch, Witness};
use crate::TerpAccountIrl;
use terp_account::traits::default::TerpAccountTrait;
use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Coin, Uint128};

#[cw_serde]
pub struct InstantiateMsg {
    /// if enabled, this contract is also being used as a x/smart-account authenticator, in order to be managed by an AVS.
    pub enable_zktls: bool,
    /// Params for your events fungible token
    pub minter_params: Option<FantokenMinterParams>,
}

#[cw_serde]
pub enum ExecuteMsg {
    /// Mint fantokens to recipients
    MintFantokens { data: Vec<MintTicketObject> },
    /// Update the URI for the fantoken
    SetUri { uri: String },
    /// Update operator set for this given event
    AddEpoch {
        witness: Vec<Witness>,
        minimum_witness: Uint128,
    },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    /// Get fantoken information
    #[returns(Option<FantokenInfo>)]
    GetFantokenInfo {},
    /// Get total minted amount
    #[returns(Uint128)]
    GetMintedAmount {},
    #[returns(GetAllEpochResponse)]
    GetAllEpoch {},
    #[returns(GetEpochResponse)]
    GetEpoch { id: u128 },
}

#[cw_serde]
pub struct GetAllEpochResponse {
    pub ids: Vec<u128>,
}

#[cw_serde]
pub struct GetEpochResponse {
    pub epoch: Epoch,
}

#[cw_serde]
pub struct MintTicketObject {
    pub ticket: String, // recipient address
    pub amount: u64,    // amount to mint
}

#[cw_serde]
pub struct FantokenMinterParams {
    pub symbol: String,
    pub name: String,
    pub max_supply: Uint128,
    pub uri: String,
}

#[cw_serde]
pub struct FantokenInfo {
    pub symbol: String,
    pub name: String,
    pub max_supply: Uint128,
    pub authority: String,
    pub uri: String,
    pub minter: String,
    pub denom: String,
}

// Message types for the fantoken module
#[cw_serde]
pub struct CreateFantokenMsg {
    pub symbol: String,
    pub name: String,
    pub max_supply: String, // Using String to handle sdk.Int serialization
    pub authority: String,
    pub uri: String,
    pub minter: String,
}

#[cw_serde]
pub struct MintFantokenMsg {
    pub recipient: String,
    pub coin: Coin,
    pub minter: String,
}

#[cw_serde]
pub struct SetFantokenUriMsg {
    pub denom: String,
    pub uri: String,
    pub authority: String,
}
