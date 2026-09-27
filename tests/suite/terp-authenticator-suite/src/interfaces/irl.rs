use cosmwasm_std::Empty;
use cw_orch::{interface, prelude::*};
use terp_auth::AuthSudoMsg;
use terp_irl::{execute, instantiate, query, sudo, ExecuteMsg, InstantiateMsg, QueryMsg};

use crate::traits::AuthSudoContract;

pub const CONTRACT_ID: &str = "terp-irl";

#[interface(InstantiateMsg, ExecuteMsg, QueryMsg, AuthSudoMsg, id = CONTRACT_ID)]
pub struct TerpIrl;

impl<Chain: TxHandler> Uploadable for TerpIrl<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path("terp_irl")
            .expect("terp_irl.wasm — use Mock")
    }
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(execute, instantiate, query).with_sudo(sudo))
    }
}
impl<Chain: CwEnv> AuthSudoContract for TerpIrl<Chain> {}
