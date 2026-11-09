//! cw-orch interface for `cw-private-dex` (Daemon upload + multi-test wrapper).
//!
//! Mirrors `cw-headstash` interface shape. Artifact label: `cw_private_dex`.

use crate::{execute, instantiate, msg::*, query};
use cosmwasm_std::Empty;
use cw_orch::prelude::*;

/// cw-orch interface for the private settle contract.
///
/// # Example (Daemon)
/// ```ignore
/// let dex = CwPrivateDexContract::new(chain);
/// dex.upload()?;
/// dex.instantiate(&InstantiateMsg { mock_verify: true, zkid: None, allowed_root: None }, None, &[])?;
/// ```
#[cw_orch::interface(InstantiateMsg, ExecuteMsg, QueryMsg, Empty, id = "cw_private_dex")]
pub struct CwPrivateDexContract;

impl<Chain: CwEnv> Uploadable for CwPrivateDexContract<Chain> {
    /// Path to the compiled wasm artifact (used with Daemon).
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path_from_crates_label("cw_private_dex")
            .unwrap()
    }

    /// CosmWasm contract wrapper for cw-multi-test / Mock environments.
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(execute, instantiate, query))
    }
}
