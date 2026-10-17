use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::Binary;

use crate::auth_types::{
    AuthenticationRequest, ConfirmExecutionRequest, OnAuthenticatorAddedRequest,
    OnAuthenticatorRemovedRequest, TrackRequest,
};

/// Params stored in smart-account module via CosmwasmAuthenticatorV1 init data.
#[cw_serde]
pub struct AuthenticatorParams {
    /// Product / stage domain, e.g. `vote.v1.cast`.
    pub domain: String,
    /// Optional fixed session; if set, payload session must match or be omitted.
    #[serde(default)]
    pub session_id: Option<String>,
    /// If true (default), effective session_id must be non-empty.
    #[serde(default = "default_true")]
    pub require_session: bool,
    /// Cap nullifiers per tx (default 8).
    #[serde(default = "default_max_nf")]
    pub max_nullifiers_per_tx: u32,
}

fn default_true() -> bool {
    true
}

fn default_max_nf() -> u32 {
    8
}

impl Default for AuthenticatorParams {
    fn default() -> Self {
        Self {
            domain: String::new(),
            session_id: None,
            require_session: true,
            max_nullifiers_per_tx: 8,
        }
    }
}

/// Auth data carried in AuthenticationRequest.signature (and optionally mirrored in msg).
#[cw_serde]
pub struct NullifierAuthPayload {
    #[serde(default)]
    pub session_id: Option<String>,
    pub nullifiers: Vec<Binary>,
}

#[cw_serde]
pub struct InstantiateMsg {
    pub admin: Option<String>,
}

/// No user execute path required for authenticator lifecycle (sudo-only spends).
#[cw_serde]
pub enum ExecuteMsg {}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(IsSpentResponse)]
    IsSpent {
        domain: String,
        session_id: String,
        nullifier: Binary,
    },
    #[returns(ConfigResponse)]
    Config {},
}

#[cw_serde]
pub struct IsSpentResponse {
    pub spent: bool,
}

#[cw_serde]
pub struct ConfigResponse {
    pub admin: Option<String>,
}

/// Sudo entry matching smart-account CosmwasmAuthenticatorV1.
#[cw_serde]
pub enum AuthenticatorSudoMsg {
    OnAuthenticatorAdded(OnAuthenticatorAddedRequest),
    OnAuthenticatorRemoved(OnAuthenticatorRemovedRequest),
    Authenticate(AuthenticationRequest),
    Track(TrackRequest),
    ConfirmExecution(ConfirmExecutionRequest),
}
