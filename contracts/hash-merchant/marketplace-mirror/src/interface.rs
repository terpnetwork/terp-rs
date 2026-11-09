use cw_orch::prelude::*;

use crate::contract::{execute, instantiate, query};
use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};

#[cw_orch::interface(InstantiateMsg, ExecuteMsg, QueryMsg, Empty, id = "marketplace-mirror")]
pub struct MarketplaceMirror;

impl<Chain: CwEnv> Uploadable for MarketplaceMirror<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path("marketplace_mirror")
            .unwrap()
    }

    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(execute, instantiate, query))
    }
}
