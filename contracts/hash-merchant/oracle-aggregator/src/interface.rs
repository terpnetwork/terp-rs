use cw_orch::prelude::*;

use crate::{execute, instantiate, query, ExecuteMsg, InstantiateMsg, QueryMsg};

#[cw_orch::interface(InstantiateMsg, ExecuteMsg, QueryMsg, Empty, id = "cw-oracle-aggregator")]
pub struct OracleAggregator;

impl<Chain: CwEnv> Uploadable for OracleAggregator<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path("cw_oracle_aggregator")
            .unwrap()
    }

    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(execute, instantiate, query))
    }
}
