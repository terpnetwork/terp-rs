use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::Binary;

#[cw_serde]
pub struct InstantiateMsg {
    /// Circuit id registered for Path A `proof_instance_verify`.
    pub zkid: u64,
}

#[cw_serde]
pub enum ExecuteMsg {
    /// Verify `proof` against `instances` on the host circuit `zkid`.
    Verify { proof: Binary, instances: Binary },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(ConfigResponse)]
    Config {},
}

#[cw_serde]
pub struct ConfigResponse {
    pub zkid: u64,
}
