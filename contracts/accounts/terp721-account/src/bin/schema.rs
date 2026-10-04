#![cfg(not(test))]
use cosmwasm_schema::write_api;

use terp721_account::{
    msg::{ExecuteMsg, InstantiateMsg},
    QueryMsg,
};

fn main() {
    write_api! {
        instantiate: InstantiateMsg,
        execute: ExecuteMsg<terp_account::Metadata>,
        query: QueryMsg,
    }
}
