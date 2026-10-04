use crate::{
    contract::{execute, instantiate, query, WLIST_MERKLETREE},
    msg::*,
};
use cw_orch::prelude::*;

#[cw_orch::interface(
    InstantiateMsg,
    ExecuteMsg,
    QueryMsg,
    Empty,
    id = WLIST_MERKLETREE
)]
pub struct WhitelistMerkleTree;

impl<Chain: CwEnv> Uploadable for WhitelistMerkleTree<Chain> {
    /// Return the path to the wasm file corresponding to the contract
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path_from_crates_label(WLIST_MERKLETREE)
            .unwrap()
    }
    /// Returns a CosmWasm contract wrapper
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(execute, instantiate, query))
    }
}
