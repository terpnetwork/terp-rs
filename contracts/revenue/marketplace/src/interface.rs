//! cw-orch interface. Host-only (`interface` feature). Artifact label `revenue_marketplace`.

use crate::{execute, instantiate, query, ExecuteMsg, InstantiateMsg, QueryMsg};
use cosmwasm_std::Empty;
use cw_orch::prelude::*;

#[cw_orch::interface(
    InstantiateMsg,
    ExecuteMsg,
    QueryMsg,
    Empty,
    id = "revenue_marketplace"
)]
pub struct RevenueMarketplace;

impl<Chain: CwEnv> Uploadable for RevenueMarketplace<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifacts_dir_from_workspace!()
            .find_wasm_path_from_crates_label("revenue_marketplace")
            .unwrap()
    }

    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(execute, instantiate, query))
    }
}
