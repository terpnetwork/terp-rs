//! Unified Terp cw-orch test suite (circuits + private-bridge + optional auth/ict).
//!
//! This is the home for product `Deploy` bags. Do not add more sibling suite
//! crates under `tests/suite/` — add a module here instead.
//!
//! | Module | Role |
//! |--------|------|
//! | [`suite`] | Path A verify contract + Stwo/Flock `circuit_interface` |
//! | [`bridge`] | `cw-private-dex` corridor settle + spend circuit |
//! | [`headstash`] | Product A airdrop claim → **clearnet** (`--features headstash`) |
//! | [`ict`] | ict-rs chain → cw-orch Daemon (`--features ict`) |
//!
//! Auth stays `terp-authenticator-suite` (halo2-heavy); re-export with `--features auth`.

pub mod artifacts;
#[cfg(feature = "bridge")]
pub mod bridge;
#[cfg(feature = "ict")]
pub mod ict;
pub mod interfaces;
pub mod orch;
pub mod suite;
#[cfg(feature = "headstash")]
pub mod headstash;

pub use artifacts::write_demo_circuit_artifacts;
pub use interfaces::{CwZkVerifyDemo, FlockLigeritoCircuit, StwoSsleCircuit};
pub use orch::{TerpOrchDeployData, TerpOrchSuite};
pub use suite::{TerpCircuitDemoDeployData, TerpCircuitDemoSuite};

#[cfg(feature = "bridge")]
pub use bridge::{TerpBridgeDeployData, TerpBridgeSuite};

#[cfg(feature = "auth")]
pub use terp_authenticator_suite::{TerpAuthenticatorDeployData, TerpAuthenticatorSuite};

#[cfg(feature = "headstash")]
pub use headstash::{run_headstash_claim, write_headstash_circuit, HeadstashClaimReport};
