use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::Binary;

use crate::types::{ClientState, ConsensusState, SettlementUpdate};

#[cw_serde]
pub struct InstantiateMsg {
    pub client_state: ClientState,
    /// Initial consensus (usually genesis lanes, height 0).
    pub consensus_state: ConsensusState,
    /// Optional 08-wasm bytecode checksum (stored for operators).
    pub checksum: Option<Binary>,
}

#[cw_serde]
pub enum ExecuteMsg {
    /// App-level settle (same as UpdateState body) for demos without IBC router.
    Settle(SettlementUpdate),
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(ClientState)]
    ClientState {},
    #[returns(ConsensusState)]
    ConsensusState {},
    #[returns(HeightResponse)]
    ProvenHeight {},
    #[returns(RootResponse)]
    ProvenRoot {},
    #[returns(BoolResponse)]
    IsProvenRoot { root: String },
    #[returns(StatusResult)]
    Status {},
    #[returns(VerifyClientMessageResult)]
    VerifyClientMessage { client_message: Binary },
}

/// 08-wasm-shaped sudo surface (subset for B2).
#[cw_serde]
pub enum SudoMsg {
    UpdateState { client_message: Binary },
    VerifyMembership {
        /// unused in B2
        height: u64,
        proof: Binary,
        path: Vec<Binary>,
        value: Binary,
    },
    VerifyNonMembership {
        height: u64,
        proof: Binary,
        path: Vec<Binary>,
    },
    UpdateStateOnMisbehaviour { client_message: Binary },
}

#[cw_serde]
pub struct HeightResponse {
    pub height: u64,
}

#[cw_serde]
pub struct RootResponse {
    pub root: String,
}

#[cw_serde]
pub struct BoolResponse {
    pub yes: bool,
}

#[cw_serde]
pub struct StatusResult {
    pub status: String,
    pub claim_class: String,
}

#[cw_serde]
pub struct VerifyClientMessageResult {
    pub ok: bool,
    pub reason: Option<String>,
}
