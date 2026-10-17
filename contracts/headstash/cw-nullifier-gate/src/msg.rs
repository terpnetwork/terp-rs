use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::Binary;

use crate::auth_types::{
    AuthenticationRequest, ConfirmExecutionRequest, OnAuthenticatorAddedRequest,
    OnAuthenticatorRemovedRequest, TrackRequest,
};

/// Params in MsgAddAuthenticator for CosmwasmAuthenticatorV1.
#[cw_serde]
pub struct AuthenticatorParams {
    /// Ceremony module (nullifier SSOT) bech32 address.
    pub ceremony_module: String,
    pub domain: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default = "default_true")]
    pub require_session: bool,
    #[serde(default = "default_max_nf")]
    pub max_nullifiers_per_tx: u32,
    /// Prefer raw storage query of spent map (default true).
    #[serde(default = "default_true")]
    pub prefer_raw_query: bool,
}

fn default_true() -> bool {
    true
}
fn default_max_nf() -> u32 {
    8
}

#[cw_serde]
pub struct NullifierAuthPayload {
    #[serde(default)]
    pub session_id: Option<String>,
    pub nullifiers: Vec<Binary>,
}

#[cw_serde]
pub struct InstantiateMsg {}

#[cw_serde]
pub enum ExecuteMsg {}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    /// Gate holds no spent SSOT — this always documents policy.
    #[returns(GateInfoResponse)]
    GateInfo {},
}

#[cw_serde]
pub struct GateInfoResponse {
    pub owns_spent_map: bool,
    pub note: String,
}

#[cw_serde]
pub enum AuthenticatorSudoMsg {
    OnAuthenticatorAdded(OnAuthenticatorAddedRequest),
    OnAuthenticatorRemoved(OnAuthenticatorRemovedRequest),
    Authenticate(AuthenticationRequest),
    Track(TrackRequest),
    ConfirmExecution(ConfirmExecutionRequest),
}

/// Message body for ceremony module MarkSpent (wasm execute path when sudo unavailable in multi-test).
/// Production: prefer chain sudo into ceremony `SudoMsg::MarkSpent`.
#[cw_serde]
pub enum CeremonyExecuteProxy {
    /// Document-only; real MarkSpent is ceremony SudoMsg.
    /// When chain supports WasmMsg::Sudo, gate emits that.
    MarkSpent {
        domain: String,
        session_id: String,
        nullifiers: Vec<Binary>,
    },
}
