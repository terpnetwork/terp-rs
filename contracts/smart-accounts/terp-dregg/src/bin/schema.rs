use cosmwasm_schema::write_api;
use terp_auth::AuthSudoMsg;

use terp_dregg::{ExecuteMsg, InstantiateMsg, QueryMsg};

fn main() {
    write_api! {
        instantiate: InstantiateMsg,
        execute: ExecuteMsg,
        query: QueryMsg,
        sudo: AuthSudoMsg,
    }
}
