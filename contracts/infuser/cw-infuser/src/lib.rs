// ContractError carries cw-utils/std errors by value; boxing would change every handler.
#![allow(clippy::result_large_err)]
pub mod contract;
mod error;
pub mod msg;
pub mod state;
// pub mod upgrades;
pub use crate::error::{AnyOfErr, ContractError};

#[cfg(feature = "interface")]
pub mod interface;
