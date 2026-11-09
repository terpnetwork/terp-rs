use cosmwasm_schema::write_api;
use terp_zktls::{ExecuteMsg, InstantiateMsg, QueryMsg};

fn main() {
    write_api! {
        instantiate: InstantiateMsg,
        execute: ExecuteMsg,
        query: QueryMsg,
        sudo: terp_auth::AuthSudoMsg,
    }
}
