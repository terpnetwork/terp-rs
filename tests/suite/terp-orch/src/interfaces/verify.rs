//! cw-orch interface for `cw-zk-verify-demo`.

use cosmwasm_std::Empty;
use cw_orch::{interface, prelude::*};
use cw_zk_verify_demo::{execute, instantiate, query, ExecuteMsg, InstantiateMsg, QueryMsg};

pub const CONTRACT_ID: &str = "cw-zk-verify-demo";

#[interface(InstantiateMsg, ExecuteMsg, QueryMsg, Empty, id = CONTRACT_ID)]
pub struct CwZkVerifyDemo;

impl<Chain: TxHandler> Uploadable for CwZkVerifyDemo<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        super::wasm_path::wasm("cw_zk_verify_demo")
    }

    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(execute, instantiate, query))
    }
}
