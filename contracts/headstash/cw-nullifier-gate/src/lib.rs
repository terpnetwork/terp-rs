//! Thin CosmwasmAuthenticator **nullifier gate**.
//!
//! - Authenticate: raw-query ceremony module spent key (no local SSOT map)
//! - Track: no-op (do not burn)
//! - ConfirmExecution: wasm execute/sudo MarkSpent on ceremony module
//!
//! Design: `docs/research/DAO-CEREMONY-NULLIFIER-AUTH.md`

pub mod auth_types;
pub mod contract;
pub mod error;
pub mod msg;
pub mod state;

pub use crate::error::ContractError;
pub use crate::msg::{AuthenticatorParams, AuthenticatorSudoMsg, InstantiateMsg, QueryMsg};
