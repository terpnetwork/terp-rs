use crate::{
    CONTRACT_NAME, ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg, execute, instantiate, migrate,
    query,
};
use cw_orch::prelude::*;

#[cw_orch::interface(InstantiateMsg, ExecuteMsg, QueryMsg, MigrateMsg, id = CONTRACT_NAME)]
pub struct SvgAuction;

impl<Chain: CwEnv> Uploadable for SvgAuction<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path_from_crates_label(CONTRACT_NAME)
            .unwrap()
    }

    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(execute, instantiate, query).with_migrate(migrate))
    }
}
