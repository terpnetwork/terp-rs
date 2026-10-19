//! cw-orch interface (feature `interface`), same shape as the other terp-rs contracts.
use crate::contract::{execute, instantiate, migrate, query, reply};
pub use crate::msg::*;
use cw_orch::prelude::*;

pub const TERP_BOUNTY_ESCROW: &str = "terp_bounty_escrow";

#[cw_orch::interface(InstantiateMsg, ExecuteMsg, QueryMsg, MigrateMsg, id = TERP_BOUNTY_ESCROW)]
pub struct TerpBountyEscrow;

impl<Chain: CwEnv> Uploadable for TerpBountyEscrow<Chain> {
    /// Optimized wasm from the workspace `artifacts/` dir (only needed for daemon chains).
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path_from_crates_label(TERP_BOUNTY_ESCROW)
            .unwrap()
    }
    /// Native wrapper for Mock / cw-multi-test.
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(
            ContractWrapper::new_with_empty(execute, instantiate, query)
                .with_reply(reply)
                .with_migrate(|deps, env, msg: MigrateMsg, _info| migrate(deps, env, msg)),
        )
    }
}
