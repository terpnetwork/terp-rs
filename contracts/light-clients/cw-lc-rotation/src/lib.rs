//! `cw-lc-rotation` — committee epochs for the Zcash IBC v2 light client
//! (SPEC: `~/grok-bot/privacy-products/light-client/SPEC.md` §4–5).
//!
//! Membership is never configured by hand: each epoch snapshots the
//! `cw-lc-registry` operator set for one client (attestors that are
//! registered, advertise the `preconf` capability, reported recently and have
//! weight > 0). Weight becomes FROST shares, and the threshold is a weighted
//! supermajority of the snapshot (`threshold_bps`, default 2/3). No single
//! operator may reach the threshold alone.
//!
//! Lifecycle: `Pending` (off-chain DKG / reshare) → `Active` → `Retired` |
//! `Faulted`. A new epoch is proposed by `Bootstrap`, `RotateShard` (planned;
//! the old key stays active and must sign the hand-off), `ReportLivenessFault`
//! (aggregator stalled) or `ReportFault` (aggregator saw equivocation). While no
//! epoch is active, `cw-lc-aggregator` serves the registry k-of-n tip.
pub mod committee;
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
    pub struct LcRotation;

    impl<Chain> Uploadable for LcRotation<Chain> {
        fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
            artifacts_dir_from_workspace!()
                .find_wasm_path("cw_lc_rotation")
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
