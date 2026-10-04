use cw_orch::{interface, prelude::*};

use crate::entry::{execute, instantiate, query, sudo};
use crate::msg::{ExecuteMsg, InstantiateMsg, Terp721AccountsQueryMsg as QueryMsg};
use crate::ACCOUNT_CONTRACT;
use terp_account::Metadata;

/// Uploadable trait for terp721_account & use with cw-orchestrator library
#[interface(InstantiateMsg, ExecuteMsg::<Metadata>, QueryMsg, Empty)]
pub struct TerpAccountCollection;

impl<Chain> Uploadable for TerpAccountCollection<Chain, Metadata> {
    /// Return the path to the wasm file corresponding to the contract
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path_from_crates_label(ACCOUNT_CONTRACT)
            .unwrap()
    }
    /// Returns a CosmWasm contract wrapper
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(execute, instantiate, query).with_sudo(sudo))
    }
}
