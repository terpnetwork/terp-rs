//! cw-orch interface for `cw-zk-verify-demo`.

use cosmwasm_std::Empty;
use cw_orch::{interface, prelude::*};
use cw_zk_verify_demo::{execute, instantiate, query, ExecuteMsg, InstantiateMsg, QueryMsg};

pub const CONTRACT_ID: &str = "cw-zk-verify-demo";

#[interface(InstantiateMsg, ExecuteMsg, QueryMsg, Empty, id = CONTRACT_ID)]
pub struct CwZkVerifyDemo;

impl<Chain: TxHandler> Uploadable for CwZkVerifyDemo<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path("cw_zk_verify_demo")
            .expect("cw_zk_verify_demo.wasm not found — use Mock wrapper")
    }

    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(execute, instantiate, query))
    }
}
