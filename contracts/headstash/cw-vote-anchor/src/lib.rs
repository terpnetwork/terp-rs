//! P0 on-chain **session root registry** for vote-sdk-variant tree host.
//!
//! Hybrid default (VOTE-TREE-HOST-TERP.md):
//! - Off-chain: vote-commitment-tree depth-24 Poseidon
//! - On-chain: `GetAnchorAtHeight(session, height) → [32]byte`
//!
//! Non-goals: full Merkle append in CosmWasm; Path A; nullifier SSOT.

pub mod contract;
pub mod error;
pub mod msg;
pub mod state;

pub use crate::error::ContractError;
pub use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
