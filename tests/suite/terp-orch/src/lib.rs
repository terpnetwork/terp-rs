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
//! | [`stack`] | Accounts, light clients, and product contracts (`--features stack`) |
//! | [`ict`] | ict-rs chain → cw-orch Daemon (`--features ict`) |
//! | [`gov`] | morocco-1 gov drafts, cw-orch dry-run (`--features gov`) |
//!
//! Auth stays `terp-authenticator-suite` (halo2-heavy); re-export with `--features auth`.

pub mod artifacts;
#[cfg(feature = "bridge")]
pub mod bridge;
#[cfg(feature = "gov")]
pub mod gov;
#[cfg(feature = "headstash")]
pub mod headstash;
#[cfg(feature = "ict")]
pub mod ict;
pub mod interfaces;
pub mod orch;
#[cfg(feature = "stack")]
pub mod stack;
pub mod suite;

pub use artifacts::write_demo_circuit_artifacts;
pub use interfaces::{CwZkVerifyDemo, FlockLigeritoCircuit, StwoSsleCircuit};
pub use orch::{TerpOrchDeployData, TerpOrchSuite};
pub use suite::{TerpCircuitDemoDeployData, TerpCircuitDemoSuite};

#[cfg(feature = "bridge")]
pub use bridge::{TerpBridgeDeployData, TerpBridgeSuite};

#[cfg(feature = "auth")]
pub use terp_authenticator_suite::{TerpAuthenticatorDeployData, TerpAuthenticatorSuite};

#[cfg(feature = "stack")]
pub use stack::{TerpAccountSuite, TerpArtifactStack, TerpLightClientSuite};
#[cfg(feature = "stack")]
pub use terp_bounty_escrow::interface::TerpBountyEscrow;

#[cfg(feature = "billboards")]
pub use terp_account_scripts as billboards;

#[cfg(feature = "infuser")]
pub use cw_infuser_scripts as infuser;

#[cfg(feature = "headstash")]
pub use headstash::{HeadstashClaimReport, run_headstash_claim, write_headstash_circuit};
