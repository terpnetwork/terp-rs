use crate::{contract::{execute, instantiate, migrate, query},msg::*};
use cw_orch::prelude::*;

#[cw_orch::interface(InstantiateMsg, ExecuteMsg, QueryMsg, Empty, id = "cw_headstash_manifold")]
pub struct CwHeadstashManifold;

impl<Chain: CwEnv> Uploadable for CwHeadstashManifold<Chain> {
    /// Return the path to the wasm file corresponding to the contract
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path_from_crates_label("cw_headstash_manifold")
            .unwrap()
    }
    /// Returns a CosmWasm contract wrapper
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(execute, instantiate, query).with_migrate(migrate))
    }
}
