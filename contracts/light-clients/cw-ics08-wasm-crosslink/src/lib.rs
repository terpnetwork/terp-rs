#![doc = include_str!("../README.md")]
#![deny(clippy::nursery, clippy::pedantic, warnings)]
#![allow(missing_docs, unused_crate_dependencies)]
// Doc prose and the CosmWasm entry-point shapes (owned msgs) are kept as-is.
#![allow(
    clippy::doc_markdown,
    clippy::missing_errors_doc,
    clippy::needless_pass_by_value
)]

/// Terp 08-wasm advertises `hash_blake`. Block and pool-root hashes use `Api::blake3_256`.
#[unsafe(no_mangle)]
const extern "C" fn requires_hash_blake() {}

pub mod contract;
pub mod custom_query;
mod error;
pub mod instantiate;
pub mod msg;
pub mod query;
pub mod state;
pub mod sudo;

#[cfg(feature = "interface")]
pub mod interface;

pub use error::ContractError;
