//! Unified Terp cw-orch test suite (circuits + private-bridge + optional auth/ict).
//!
//! This is the home for product `Deploy` bags. Do not add more sibling suite
//! crates under `tests/suite/` — add a module here instead.
//!
//! | Module | Role |
//! |--------|------|
//! | [`suites`] | Deploy bags: circuits, private-dex, ave, shitstrap, nftfi, svg, bounties, accounts |
//! | [`bridge`] | `cw-private-dex` corridor settle + spend circuit |
//! | [`headstash`] | Product A airdrop claim → **clearnet** (`--features headstash`) |
//! | [`stack`] | Every contract interface (`--features stack`) |
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
#[cfg(feature = "stack")]
pub mod stack;
pub mod suites;

pub use artifacts::write_demo_circuit_artifacts;
pub use interfaces::{CwZkVerifyDemo, FlockLigeritoCircuit, StwoSsleCircuit};
pub use suites::circuit::{CircuitDemoDeployData, CircuitDemoSuite};

#[cfg(feature = "bridge")]
pub use bridge::{TerpBridgeDeployData, TerpBridgeSuite};
#[cfg(feature = "bridge")]
pub use suites::private_dex::PrivateDexSuite;

#[cfg(feature = "stack")]
pub use dao_testing::DaoDaoSuite;
#[cfg(feature = "stack")]
pub use suites::ave::CwAveSuite;
#[cfg(feature = "stack")]
pub use suites::bounties::{
    BountyEscrowInitMsg, BountyEscrowSuite, expiry, init_msg, param_denoms, param_min_submit_stake,
    param_min_upload_stake,
};
#[cfg(feature = "stack")]
pub use suites::nftfi::{NftfiDeployData, NftfiSuite};
#[cfg(feature = "stack")]
pub use suites::shitstrap::{
    CwShitstrapSuite, CwShitstrapSuiteDeployData, ShitInitMsg, UncheckedDenom, build_shit_init,
    calc_rates, new_default_ibc_set, shit_deploy_data_full, shit_deploy_data_single,
};
#[cfg(feature = "stack")]
pub use suites::svg::{CwSvgSuite, CwSvgSuiteDeployData};

#[cfg(feature = "auth")]
pub use terp_authenticator_suite::{TerpAuthenticatorDeployData, TerpAuthenticatorSuite};

#[cfg(feature = "stack")]
pub use stack::{
    TerpAccountSuite, TerpArtifactStack, TerpBillboardSuite, TerpInfuserSuite,
    TerpLightClientSuite, TerpProductSuite,
};
#[cfg(feature = "stack")]
pub use terp_bounty_escrow::interface::TerpBountyEscrow;

#[cfg(feature = "headstash")]
pub use headstash::{HeadstashClaimReport, run_headstash_claim, write_headstash_circuit};
