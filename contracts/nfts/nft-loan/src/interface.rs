use crate::contract::{execute, instantiate, query, CONTRACT_NAME};
use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
use cw_orch::prelude::*;

#[cw_orch::interface(InstantiateMsg, ExecuteMsg, QueryMsg, Empty, id = CONTRACT_NAME)]
pub struct NftfiLoan;

impl<Chain: CwEnv> Uploadable for NftfiLoan<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path_from_crates_label(CONTRACT_NAME)
            .unwrap()
    }

    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(execute, instantiate, query))
    }
}
