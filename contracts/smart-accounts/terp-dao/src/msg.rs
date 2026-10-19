use cosmwasm_schema::{QueryResponses, cw_serde};

#[cw_serde]
pub struct InstantiateMsg {
    pub dao_addr: String,
}

#[cw_serde]
pub enum ExecuteMsg {}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(String)]
    GetDao {},
}
