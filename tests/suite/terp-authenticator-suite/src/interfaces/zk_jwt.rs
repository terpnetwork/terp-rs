//! cw-orch interface for terp-zkjwt.

use cosmwasm_std::Empty;
use cw_orch::{interface, prelude::*};
use terp_auth::AuthSudoMsg;
use terp_zkjwt::{execute, instantiate, query, sudo, ExecuteMsg, InstantiateMsg, QueryMsg};

use crate::traits::AuthSudoContract;

pub const CONTRACT_ID: &str = "terp-zkjwt";

#[interface(InstantiateMsg, ExecuteMsg, QueryMsg, AuthSudoMsg, id = CONTRACT_ID)]
pub struct TerpZkJwt;

impl<Chain: TxHandler> Uploadable for TerpZkJwt<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path("terp_zkjwt")
            .expect("terp_zkjwt.wasm not found — use Mock wrapper")
    }

    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(execute, instantiate, query).with_sudo(sudo))
    }
}

impl<Chain: CwEnv> AuthSudoContract for TerpZkJwt<Chain> {}
