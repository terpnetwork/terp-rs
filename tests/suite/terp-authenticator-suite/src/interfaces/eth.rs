use cosmwasm_std::Empty;
use cw_orch::{interface, prelude::*};
use terp_auth::AuthSudoMsg;
use terp_eth::{execute, instantiate, query, sudo, ExecuteMsg, InstantiateMsg, QueryMsg};

use crate::traits::AuthSudoContract;

pub const CONTRACT_ID: &str = "terp-eth";

#[interface(InstantiateMsg, ExecuteMsg, QueryMsg, AuthSudoMsg, id = CONTRACT_ID)]
pub struct TerpEth;

impl<Chain: TxHandler> Uploadable for TerpEth<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path("terp_eth")
            .expect("terp_eth.wasm — use Mock")
    }
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(execute, instantiate, query).with_sudo(sudo))
    }
}
impl<Chain: CwEnv> AuthSudoContract for TerpEth<Chain> {}
