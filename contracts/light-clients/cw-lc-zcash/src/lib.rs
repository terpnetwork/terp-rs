//! `cw-lc-zcash` — the IBC v2 Zcash light client (`terp/lc/client/<target>/v1`).
//!
//! * An operator registers the IBC client against attesting services: the
//!   client record + attestors live in `cw-lc-registry` (`RegisterClient`,
//!   `BindIbcClient` → this client's `ibc_client_id`); headers come from
//!   `cw-lc-aggregator` (committee preconf, k-of-n fallback).
//! * Anyone can read zakura node state / tips / best header through it.
//! * **Payment inclusion (zap-1 style)** for atomic settlement:
//!   `VerifyPaymentInclusion` checks a zap-1 leaf + Merkle path against the
//!   committee-attested `payment_root` of the header at `height`, and the
//!   IBC v2 `app_data_hash = H(note ‖ amount ‖ frost_group ‖ leaf)` (same
//!   binding as `cw-ics08-wasm-zap1`). `PaymentInclusion {app_data_hash}`
//!   re-checks the header live (Preconfirmed / Confirmed / Reorged), so a
//!   settlement contract can require `Confirmed` before releasing an asset.
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
    pub struct LcZcash;

    impl<Chain> Uploadable for LcZcash<Chain> {
        fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
            artifacts_dir_from_workspace!()
                .find_wasm_path("cw_lc_zcash")
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
