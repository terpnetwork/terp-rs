#![cfg(not(test))]
use cosmwasm_schema::write_api;

use terp_account::manifold::{ExecuteMsg, InstantiateMsg, QueryMsg, SudoMsg};

fn main() {
    write_api! {
        instantiate: InstantiateMsg,
        execute: ExecuteMsg,
        query: QueryMsg,
        sudo: SudoMsg,
    }
}
