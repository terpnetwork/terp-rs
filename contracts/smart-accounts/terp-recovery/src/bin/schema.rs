use cosmwasm_schema::write_api;
use terp_auth::AuthSudoMsg;

use terp_recovery::{ExecuteMsg, InstantiateMsg, QueryMsg};

fn main() {
    write_api! {
        instantiate: InstantiateMsg,
        execute: ExecuteMsg,
        query: QueryMsg,
        sudo: AuthSudoMsg,
    }
}
