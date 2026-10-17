use crate::{execute, instantiate, query, reply, sudo, msg::*};
use cosmwasm_std::Empty;
use cw_orch::prelude::*;

/// cw-orch interface for the `cw-headstash` contract.
///
/// # Example (scripting)
/// ```ignore
/// let chain = DaemonBuilder::new(TERP_MAINNET).build()?;
/// let headstash = HeadstashContract::new(chain);
/// headstash.upload()?;
/// headstash.instantiate(&InstantiateMsg { .. }, None, &[])?;
/// ```
///
/// # Example (multi-test)
/// ```ignore
/// let app = Mock::new("sender");
/// let headstash = HeadstashContract::new(app);
/// headstash.upload()?;
/// ```
#[cw_orch::interface(InstantiateMsg, ExecuteMsg, QueryMsg, Empty, id = "cw_headstash")]
pub struct HeadstashContract;

impl<Chain: CwEnv> Uploadable for HeadstashContract<Chain> {
    /// Path to the compiled wasm artifact (used with Daemon).
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path_from_crates_label("cw_headstash")
            .unwrap()
    }

    /// CosmWasm contract wrapper for cw-multi-test environments.
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(
            ContractWrapper::new_with_empty(execute, instantiate, query)
                .with_reply(reply)
                .with_sudo(sudo),
        )
    }
}
