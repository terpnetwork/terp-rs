//! Terp bounty escrow (DEV/TEST ONLY, not audited).
//! See DESIGN.md for the state machine, events and governance decisions.
pub mod cid;
pub mod contract;
pub mod dao;
pub mod error;
pub mod msg;
pub mod state;

pub use crate::error::ContractError;

#[cfg(feature = "interface")]
pub mod interface;
