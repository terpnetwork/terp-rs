use cw_orch::prelude::*;

use crate::{ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg, execute, instantiate, migrate, query};

#[cw_orch::interface(
    InstantiateMsg,
    ExecuteMsg,
    QueryMsg,
    MigrateMsg,
    id = "cw-oracle-twap-probe"
)]
pub struct OracleTwapProbe;

impl<Chain: CwEnv> Uploadable for OracleTwapProbe<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path("cw_oracle_twap_probe")
            .unwrap()
    }

    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(execute, instantiate, query).with_migrate(migrate))
    }
}
