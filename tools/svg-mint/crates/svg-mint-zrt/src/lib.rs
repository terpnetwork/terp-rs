//! Durable house wallet. One directory, one spend seed, one note ledger.
//!
//! `init` is the only path that creates a seed. Every other command fails closed
//! when the seed is missing. Send policy checks do not mark notes spent.

mod account;

pub use account::*;
