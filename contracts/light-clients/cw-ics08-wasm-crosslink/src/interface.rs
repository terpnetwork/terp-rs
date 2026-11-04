//! cw-orch interface for the Crosslink IBC light client.
//!
//! Feature-gated behind `interface`. Provides `CrosslinkLightClient<Chain>`
//! for use in cw-orchestrator test suites and deployment scripts.

use cosmwasm_std::Empty;
use cw_orch::{interface, prelude::*};

use crate::contract::{CONTRACT_NAME, execute, instantiate, migrate, query, sudo};
use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};

/// cw-orch interface for the crosslink light client contract.
#[interface(InstantiateMsg, ExecuteMsg, QueryMsg, Empty, id = CONTRACT_NAME )]
pub struct CrosslinkLightClient;

impl<Chain: TxHandler> Uploadable for CrosslinkLightClient<Chain> {
    /// Return the path to the wasm file corresponding to the contract
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path("cw_ics08_wasm_crosslink")
            .unwrap()
    }
    /// Returns a `CosmWasm` contract wrapper
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(
            ContractWrapper::new_with_empty(execute, instantiate, query)
                .with_migrate(migrate)
                .with_sudo(sudo),
        )
    }
}
