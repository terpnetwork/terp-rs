use cw_orch::{interface, prelude::*};

use cw_ave::msg::{ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg};

pub const CONTRACT_ID: &str = "cw_ave";

#[interface(InstantiateMsg, ExecuteMsg, QueryMsg, MigrateMsg, id = CONTRACT_ID)]
pub struct CwAve;

impl<Chain> Uploadable for CwAve<Chain> {
    /// Return the path to the wasm file corresponding to the contract
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path("cw_ave")
            .unwrap()
    }
    /// Returns a CosmWasm contract wrapper
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(
            ContractWrapper::new_with_empty(
                cw_ave::contract::execute,
                cw_ave::contract::instantiate,
                cw_ave::contract::query,
            )
            .with_migrate(cw_ave::contract::migrate),
        )
    }
}
