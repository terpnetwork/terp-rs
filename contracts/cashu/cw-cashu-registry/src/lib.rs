//! `cw-cashu-registry` — **canonical** on-chain discovery registry for Cashu mints.
//!
//! Unified schema for mint URLs + metadata. Does **not** mint ecash or Headstash
//! notes, and is **not** an “official” or endorsed mint list.
//!
//! See `docs/plans/cashu/CANONICAL-MINT-REGISTRY.md`.

pub mod contract;
pub mod error;
pub mod msg;
pub mod state;

pub use crate::error::ContractError;
pub use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
pub use crate::state::{Config, MintDescriptor, MintStatus};

#[cfg(all(feature = "interface", not(target_arch = "wasm32")))]
mod interface;
#[cfg(all(feature = "interface", not(target_arch = "wasm32")))]
pub use interface::CwCashuRegistry;

#[cfg(test)]
mod tests;
