use cw_orch::{interface, prelude::*};

use crate::contract::{execute, instantiate, query, sudo, ACCOUNT_MANIFOLD_CONTRACT};
use crate::hooks::reply;
use terp_account::manifold::{ExecuteMsg, InstantiateMsg, QueryMsg};
/// Uploadable trait for terp721_account_manifold & use with cw-orchestrator library
#[interface(InstantiateMsg, ExecuteMsg, QueryMsg, cosmwasm_std::Empty)]
pub struct TerpAccountMinter;

impl<Chain> Uploadable for TerpAccountMinter<Chain> {
    /// Return the path to the wasm file corresponding to the contract
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path_from_crates_label(ACCOUNT_MANIFOLD_CONTRACT)
            .unwrap()
    }
    /// Returns a CosmWasm contract wrapper
    fn wrapper() -> Box<dyn MockContract<cosmwasm_std::Empty>> {
        Box::new(
            ContractWrapper::new_with_empty(execute, instantiate, query)
                .with_sudo(sudo)
                .with_reply(reply),
        )
    }
}
