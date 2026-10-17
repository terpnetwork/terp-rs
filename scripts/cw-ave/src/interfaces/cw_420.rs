use cw_orch::{interface, prelude::*};

use cw420::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};

pub const CONTRACT_ID: &str = "cw420";

#[interface(InstantiateMsg, ExecuteMsg, QueryMsg,Empty,  id = CONTRACT_ID)]
pub struct Cw420;

impl<Chain> Uploadable for Cw420<Chain> {
    /// Return the path to the wasm file corresponding to the contract
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path("cw420")
            .unwrap()
    }
    /// Returns a CosmWasm contract wrapper
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(
            ContractWrapper::new_with_empty(
                cw420::contract::execute,
                cw420::contract::instantiate,
                cw420::contract::query,
            ), // .with_migrate(cw420::contract::migrate),
        )
    }
}
