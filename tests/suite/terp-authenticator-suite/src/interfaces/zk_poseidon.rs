//! cw-orch interface for the terp-zkposiedon scaffold authenticator.

use cosmwasm_std::Empty;
use cw_orch::{interface, prelude::*};
use terp_auth::AuthSudoMsg;
use terp_zkposiedon::{execute, instantiate, query, sudo, ExecuteMsg, InstantiateMsg, QueryMsg};

use crate::traits::AuthSudoContract;

pub const CONTRACT_ID: &str = "terp-zkposiedon";

#[interface(InstantiateMsg, ExecuteMsg, QueryMsg, AuthSudoMsg, id = CONTRACT_ID)]
pub struct TerpZkPoseidon;

impl<Chain: TxHandler> Uploadable for TerpZkPoseidon<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path("terp_zkposiedon")
            .expect("terp_zkposiedon.wasm not found — run optimizer or use Mock (wrapper)")
    }

    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(execute, instantiate, query).with_sudo(sudo))
    }
}

impl<Chain: CwEnv> AuthSudoContract for TerpZkPoseidon<Chain> {}
