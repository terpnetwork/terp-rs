//! Vote extension handler — re-exported from client/ve for convenience.
//!
//! The VoteExtensionHandler lives in client/ve since it's used by both
//! the client and server binaries.
pub mod attestations;
#[cfg(feature = "headstash")]
pub mod headstash;
#[cfg(feature = "headstash")]
pub mod vm_verify;
pub mod server;
pub use crate::client::ve::{SignedVoteExtension, VoteExtensionHandler};

pub use server::*;
