pub mod contract;
pub mod error;
pub mod msg;
pub mod state;
#[cfg(test)]
mod tests;
pub use crate::error::ContractError;

#[cfg(feature = "interface")]
pub mod interface {
    use cw_orch::{interface, prelude::*};

    use crate::msg::{ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg};

    #[interface(InstantiateMsg, ExecuteMsg, QueryMsg, MigrateMsg)]
    pub struct OracleRegistryContract;

    impl<Chain> Uploadable for OracleRegistryContract<Chain> {
        fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
            artifacts_dir_from_workspace!()
                .find_wasm_path("cw_oracle_registry")
                .unwrap()
        }
        fn wrapper() -> Box<dyn MockContract<Empty>> {
            Box::new(
                ContractWrapper::new_with_empty(
                    crate::contract::execute,
                    crate::contract::instantiate,
                    crate::contract::query,
                )
                .with_sudo(crate::contract::sudo)
                .with_migrate(crate::contract::migrate),
            )
        }
    }
}
