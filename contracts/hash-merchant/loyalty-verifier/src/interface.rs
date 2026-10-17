use cw_orch::prelude::*;

use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
use crate::{execute, instantiate, query};

#[cw_orch::interface(InstantiateMsg, ExecuteMsg, QueryMsg, Empty, id = "loyalty-verifier")]
pub struct LoyaltyVerifier;

impl<Chain: CwEnv> Uploadable for LoyaltyVerifier<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path("loyalty_verifier")
            .unwrap()
    }

    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(execute, instantiate, query))
    }
}
