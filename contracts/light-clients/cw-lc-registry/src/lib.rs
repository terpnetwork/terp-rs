//! `cw-lc-registry` — first contract of the Zcash IBC v2 light-client family
//! (see `~/grok-bot/privacy-products/light-client/SPEC.md`).
//!
//! * Operators (attestation services backed by zakura p2pv2 nodes) register an
//!   Ed25519 key (a FROST-Ed25519 share group key verifies the same way).
//! * A client owner registers an IBC client record for a PoW chain and picks
//!   which operators attest to it (k-of-n).
//! * Operators push signed internal node state (tip, chain work, finality);
//!   anyone can query each report and the k-of-n agreed tip.
//!
//! * Operators carry an admin-set `weight` (committee shares); `cw-lc-rotation`
//!   derives the dynamic FROST committee from this set each epoch and
//!   `cw-lc-aggregator` serves preconfirmed headers, falling back to the
//!   k-of-n `ConsensusTip` here (SPEC §4–5).
//! * Optional hash-market capability: the tip is cross-checked against the
//!   hm-vext capability quorum root (`quorum.rs`).
pub mod contract;
pub mod digest;
pub mod error;
pub mod label;
pub mod msg;
pub mod quorum;
pub mod state;
#[cfg(test)]
mod tests;

pub use crate::error::ContractError;

#[cfg(feature = "interface")]
pub mod interface {
    use cw_orch::{interface, prelude::*};

    use crate::msg::{ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg};

    #[interface(InstantiateMsg, ExecuteMsg, QueryMsg, MigrateMsg)]
    pub struct LcRegistry;

    impl<Chain> Uploadable for LcRegistry<Chain> {
        fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
            artifacts_dir_from_workspace!()
                .find_wasm_path("cw_lc_registry")
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
