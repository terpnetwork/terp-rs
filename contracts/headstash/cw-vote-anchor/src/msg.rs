use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::Binary;

#[cw_serde]
pub struct InstantiateMsg {
    /// Authorized root poster (M1 session admin). Optional open mode if None (tests only).
    pub admin: Option<String>,
}

#[cw_serde]
pub enum ExecuteMsg {
    /// Post checkpoint root for (session, height). Root = 32-byte LE Pallas Fp.
    SetAnchor {
        session_id: String,
        height: u64,
        root: Binary,
    },
    /// Transfer admin.
    UpdateAdmin { admin: Option<String> },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(ConfigResponse)]
    Config {},

    /// Authoritative cast PI binding: GetAnchorAtHeight.
    #[returns(AnchorResponse)]
    GetAnchorAtHeight { session_id: String, height: u64 },

    #[returns(LatestResponse)]
    GetLatest { session_id: String },
}

#[cw_serde]
pub struct ConfigResponse {
    pub admin: Option<String>,
}

#[cw_serde]
pub struct AnchorResponse {
    pub session_id: String,
    pub height: u64,
    /// Present iff posted.
    pub root: Option<Binary>,
}

#[cw_serde]
pub struct LatestResponse {
    pub session_id: String,
    pub latest_height: Option<u64>,
    pub latest_root: Option<Binary>,
}
