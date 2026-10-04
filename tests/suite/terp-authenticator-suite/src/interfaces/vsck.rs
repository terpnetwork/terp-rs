//! cw-orch interface for terp-vsck (Shielded Vote DAO authenticator).

use cosmwasm_std::Empty;
use cw_orch::{interface, prelude::*};
use terp_auth::AuthSudoMsg;
use terp_vsck::{execute, instantiate, query, sudo, ExecuteMsg, InstantiateMsg, QueryMsg};

use crate::traits::AuthSudoContract;

pub const CONTRACT_ID: &str = "terp-vsck";

#[interface(InstantiateMsg, ExecuteMsg, QueryMsg, AuthSudoMsg, id = CONTRACT_ID)]
pub struct TerpVsck;

impl<Chain: TxHandler> Uploadable for TerpVsck<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        super::wasm_path::wasm("terp_vsck")
    }

    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(execute, instantiate, query).with_sudo(sudo))
    }
}

impl<Chain: CwEnv> AuthSudoContract for TerpVsck<Chain> {}
