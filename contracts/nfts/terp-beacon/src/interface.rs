use crate::contract::{CONTRACT_NAME, execute, instantiate, migrate, query, reply};
use crate::msg::{ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg};
use cw_orch::prelude::*;

#[cw_orch::interface(InstantiateMsg, ExecuteMsg, QueryMsg, MigrateMsg, id = CONTRACT_NAME)]
pub struct TerpBeacon;

impl<Chain: CwEnv> Uploadable for TerpBeacon<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path_from_crates_label(CONTRACT_NAME)
            .unwrap()
    }

    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(
            ContractWrapper::new_with_empty(execute, instantiate, query)
                .with_reply_empty(reply)
                .with_migrate(migrate),
        )
    }
}
