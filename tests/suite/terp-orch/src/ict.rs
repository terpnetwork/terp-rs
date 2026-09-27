//! ict-rs glue: spawn a local Terp chain, then attach [`crate::TerpOrchSuite`].
//!
//! Requires `--features ict`. Docker TestEnv is owned by ict-rs; this module
//! only converts the running chain to a cw-orch `DaemonBuilder`.

pub use ict_rs_cw_orch::{daemon_builder_from_chain as daemon_builder, BridgeError};
