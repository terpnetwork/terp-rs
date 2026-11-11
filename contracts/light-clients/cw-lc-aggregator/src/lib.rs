//! `cw-lc-aggregator` — serves Zcash headers to the IBC v2 client
//! (SPEC: `~/grok-bot/privacy-products/light-client/SPEC.md` §4).
//!
//! terpz-style proactive preconfirmation: the active committee (from
//! `cw-lc-rotation`) signs every new block header commitment as soon as its
//! zakura nodes see it (NU7: 20–25 s) and any relayer submits it. Headers are
//! `Preconfirmed` on a valid group signature and must link to their parent;
//! they become `Confirmed` once the registry's k-of-n (hash-market checked)
//! tip is `confirm_depth` blocks past them on the same chain.
//!
//! Two valid committee signatures for different headers at one height in one
//! epoch are equivocation: evidence is stored and the epoch is faulted in
//! `cw-lc-rotation`. With no active or a stalled committee, `BestHeader`
//! falls back to the registry `ConsensusTip`.
pub mod contract;
pub mod error;
pub mod msg;
pub mod state;

pub use crate::error::ContractError;

#[cfg(feature = "interface")]
pub mod interface {
    use cw_orch::{interface, prelude::*};

    use crate::msg::{ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg};

    #[interface(InstantiateMsg, ExecuteMsg, QueryMsg, MigrateMsg)]
    pub struct LcAggregator;

    impl<Chain> Uploadable for LcAggregator<Chain> {
        fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
            artifacts_dir_from_workspace!()
                .find_wasm_path("cw_lc_aggregator")
                .unwrap()
        }
        fn wrapper() -> Box<dyn MockContract<Empty>> {
            Box::new(
                ContractWrapper::new_with_empty(
                    crate::contract::execute,
                    crate::contract::instantiate,
                    crate::contract::query,
                )
                .with_migrate(crate::contract::migrate),
            )
        }
    }
}
