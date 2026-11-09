//! DAO ceremony module contract — **nullifier system of record**.
//!
//! Owns:
//! - ceremonies (session lifecycle)
//! - registrations
//! - spent nullifiers `(domain, session_id, nullifier)`
//!
//! Does **not** own smart-account ante hooks (V2 thin gate raw-queries this module).
//!
//! Design: `docs/research/DAO-CEREMONY-NULLIFIER-AUTH.md`

pub mod contract;
pub mod error;
pub mod msg;
pub mod raw_keys;
pub mod state;

pub use crate::error::ContractError;
pub use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg, SudoMsg};

#[cfg(feature = "interface")]
pub mod interface;
