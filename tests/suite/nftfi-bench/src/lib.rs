//! Host harness for nftfi gas rows.
//!
//! The marketplace, raffle, and loan contracts in this crate record the
//! brief's actions on top of cw721-base. They are not the sibling ports.
//! Mock gas is Cosmos SDK KV storage gas, the same schedule as
//! `contracts/nfts/raffle/tests/gas.rs`. The daemon path does not broadcast.

pub mod daemon;
pub mod kv;
pub mod loan;
pub mod marketplace;
pub mod nft;
pub mod pay;
pub mod raffle;
pub mod run;
pub mod table;

pub use daemon::{DaemonGate, daemon_gate};
pub use run::run_mock;
pub use table::Table;
