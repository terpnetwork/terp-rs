//! marketplace-mirror — Nostr-mirrored inventory with buffer-escrow purchases.
//!
//! ## Content plane
//! Product `content_cid` prefers bare **sha256** (BUD primary). TreeStore is the
//! sole BlobStore on the hash-market content module — see
//! `tools/hash-market/docs/content-distribution.md`.
//!
//! ## Buffer escrow
//! On `Purchase`, inventory is reserved and payment is locked until
//! `reserve_height + min_buffer_blocks` (default **7**, range 5–10).
//! `Settle` only succeeds at/after that height; `Cancel` refunds during buffer.
//!
//! ## Hashmerchant
//! `sudo` stores last foreign root by `(chain_uid, algo)` for future
//! proof-gated restock. **Does not mint supply** from oracle fields.
//! Register with `terpd tx hashmerchant register-contract`.

pub mod contract;
pub mod error;
pub mod msg;
pub mod state;

pub use crate::error::ContractError;
pub use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg, SudoMsg};
pub use crate::state::{Config, OrderStatus, Product, PurchaseOrder, RootEntry};

#[cfg(all(feature = "interface", not(target_arch = "wasm32")))]
mod interface;
#[cfg(all(feature = "interface", not(target_arch = "wasm32")))]
pub use interface::MarketplaceMirror;

#[cfg(test)]
mod tests;
