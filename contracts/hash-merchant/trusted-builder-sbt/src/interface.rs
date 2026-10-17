use cw_orch::prelude::*;

use crate::{execute, instantiate, query, ExecuteMsg, InstantiateMsg, QueryMsg};

#[cw_orch::interface(InstantiateMsg, ExecuteMsg, QueryMsg, Empty, id = "trusted-builder-sbt")]
pub struct TrustedBuilderSbt;

impl<Chain: CwEnv> Uploadable for TrustedBuilderSbt<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path("trusted_builder_sbt")
            .unwrap()
    }

    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(execute, instantiate, query))
    }
}
