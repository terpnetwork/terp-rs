//! Terp integration + harness types (`terp-scripts` package, path `tests/`).
//!
//! Phase B calendar Nostr lives under [`environments::nostr`] and
//! `tests/nostr_orch_suite.rs`.
//!
//! Agents: read `tests/agent/COMMANDS.md` first for offline vs live verbs.

pub mod environments;
pub mod ibc;
pub mod ibc_core;
pub mod report;
pub mod suite;

pub use report::{
    preflight_missing, required_env_for_mode, ArtifactRef, CapabilityMode, OutputFormat, RunEnv,
    RunReport,
};

#[cfg(feature = "nostr")]
pub use environments::nostr;

#[cfg(feature = "docker")]
pub use environments::quickspawn;

#[cfg(all(feature = "nostr", feature = "hash-market"))]
pub mod prelude {
    pub use crate::environments::nostr::{
        bind_and_bridge_offchain, calendar_meta_to_nostr, chain_event_to_nostr, nip01_to_ict,
        NostrClient, NostrEvent, NostrRelayerManager, NostrTestEnv,
    };
}
