use crate::{
    CONTRACT_NAME, execute, ibc_source_callback, instantiate, migrate, msg::*, query, reply, sudo,
};
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
/// multi-test calls migrate with MigrateInfo; the wasm export takes three args.
fn migrate_mt(
    deps: cosmwasm_std::DepsMut,
    env: cosmwasm_std::Env,
    msg: MigrateMsg,
    _info: cosmwasm_std::MigrateInfo,
) -> Result<cosmwasm_std::Response, cosmwasm_std::StdError> {
    migrate(deps, env, msg)
}

#[cw_orch::interface(InstantiateMsg, ExecuteMsg, QueryMsg, MigrateMsg, id = CONTRACT_NAME)]
pub struct HeadstashContract;

impl<Chain: CwEnv> Uploadable for HeadstashContract<Chain> {
    /// Path to the compiled wasm artifact (used with Daemon).
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path_from_crates_label(CONTRACT_NAME)
            .unwrap()
    }

    /// CosmWasm contract wrapper for cw-multi-test environments.
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(
            ContractWrapper::new_with_empty(execute, instantiate, query)
                .with_reply(reply)
                .with_sudo(sudo)
                .with_migrate(migrate_mt)
                .with_ibc_source_callback(ibc_source_callback),
        )
    }
}
