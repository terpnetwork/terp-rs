use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::Binary;

#[cw_serde]
pub struct InstantiateMsg {
    /// Accepted proof bytes for lab/localterp (production: wasmvm Authenticate).
    pub accepted_proof: Binary,
    /// Label: dummy smart-account this verifier stands in for.
    #[serde(default)]
    pub dummy_account: Option<String>,
}

#[cw_serde]
pub enum ExecuteMsg {
    /// Rotate accepted fixture (admin/dummy). Lab only.
    SetAcceptedProof { proof: Binary },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(VerifyResponse)]
    VerifyMinHolders {
        session_id: String,
        registrant: String,
        proof: Binary,
    },
    #[returns(ConfigResponse)]
    Config {},
}

#[cw_serde]
pub struct VerifyResponse {
    pub ok: bool,
}

#[cw_serde]
pub struct ConfigResponse {
    pub dummy_account: Option<String>,
    pub note: String,
}
