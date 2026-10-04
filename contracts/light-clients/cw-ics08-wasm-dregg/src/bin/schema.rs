use cosmwasm_schema::write_api;

use cw_ics08_wasm_dregg::msg::{ExecuteMsg, InstantiateMsg, QueryMsg, SudoMsg};

fn main() {
    write_api! {
        instantiate: InstantiateMsg,
        execute: ExecuteMsg,
        query: QueryMsg,
        sudo: SudoMsg,
    }
}
