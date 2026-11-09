//! CosmwasmAuthenticator nullifier registry.
//!
//! Lifecycle (smart-account ante/post):
//! - **Authenticate** — read-only check nullifier(s) not spent (writes discarded).
//! - **Track** — no-op (do not spend; Track commits even if execute fails).
//! - **ConfirmExecution** — mark nullifier(s) spent after successful execution path.
//!
//! Design: `docs/research/NULLIFIER-AUTHENTICATOR-CONTRACT.md`

pub mod auth_types;
pub mod contract;
pub mod error;
pub mod msg;
pub mod state;

pub use crate::error::ContractError;
pub use crate::msg::{
    AuthenticatorParams, AuthenticatorSudoMsg, InstantiateMsg, NullifierAuthPayload, QueryMsg,
};
