use cosmwasm_schema::write_api;
use svg_auction::{ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg};

fn main() {
    write_api! {
        instantiate: InstantiateMsg,
        execute: ExecuteMsg,
        query: QueryMsg,
        migrate: MigrateMsg,
    }
}
