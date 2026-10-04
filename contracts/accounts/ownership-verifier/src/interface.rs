use cw_orch::{interface, prelude::*};

use crate::{execute, instantiate, query, ExecuteMsg, InstantiateMsg, QueryMsg, CONTRACT_NAME};

/// Uploadable trait for terp721_account_manifold & use with cw-orchestrator library
#[interface(InstantiateMsg, ExecuteMsg, QueryMsg, Empty)]
pub struct TestingOwnershipVerifier;

impl<Chain> Uploadable for TestingOwnershipVerifier<Chain> {
    /// Return the path to the wasm file corresponding to the contract
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path_from_crates_label(CONTRACT_NAME)
            .unwrap()
    }
    /// Returns a CosmWasm contract wrapper
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(execute, instantiate, query))
    }
}
