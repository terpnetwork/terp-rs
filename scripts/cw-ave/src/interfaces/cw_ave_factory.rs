use cw_orch::{interface, prelude::*};

use cw_ave_factory::msg::{ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg};

pub const CONTRACT_ID: &str = "cw_ave_factory";

#[interface(InstantiateMsg, ExecuteMsg, QueryMsg, MigrateMsg, id = CONTRACT_ID)]
pub struct CwAveFactory;

impl<Chain> Uploadable for CwAveFactory<Chain> {
    /// Return the path to the wasm file corresponding to the contract
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path("cw_ave_factory")
            .unwrap()
    }
    /// Returns a CosmWasm contract wrapper
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(
            ContractWrapper::new_with_empty(
                cw_ave_factory::contract::execute,
                cw_ave_factory::contract::instantiate,
                cw_ave_factory::contract::query,
            )
            .with_migrate(cw_ave_factory::contract::migrate)
            .with_reply(cw_ave_factory::contract::reply),
        )
    }
}
