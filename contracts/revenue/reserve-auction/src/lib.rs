//! Reserve (English) auction for cw721 NFTs.
//!
//! Ported from public-awesome `reserve-auction`. The first bid starts the clock.
//! A bid near the end extends it. Anyone can settle after `end_time`. Sudo
//! begin/end block handles chain-halt postponement and batch settlement.

mod error;
pub mod execute;
mod helpers;
pub mod instantiate;
pub mod msg;
pub mod query;
pub mod state;
pub mod sudo;

pub use crate::error::ContractError;
pub use crate::execute::execute;
pub use crate::instantiate::instantiate;
pub use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg, SudoMsg};
pub use crate::query::query;
pub use crate::sudo::sudo;

#[cfg(feature = "interface")]
mod interface;
#[cfg(feature = "interface")]
pub use interface::RevenueReserveAuction;

pub const CONTRACT_NAME: &str = "revenue-reserve-auction";
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod multitest;
