//! Attach flags for sidecar deploy.
//!
//! The contract manifold is [`terp_orch::TerpArtifactStack`] under `--features orch-suite`.
//! Contract e2e cases live next to the contract, not in another suite crate.

use cosmwasm_std::Addr;

/// What to attach when a caller still drives auth or private-dex beside the artifact stack.
#[derive(Clone, Debug, Default)]
pub struct TerpNetworkDeployData {
    pub admin: Option<Addr>,
    /// When true, `load_from` / `store_if_needed` attach the authenticator suite
    /// (`auth-suite-full`).
    pub attach_auth: bool,
    /// When true, attach unified `terp-orch` (circuits + private-bridge).
    pub attach_orch: bool,
    /// When true, attach `PrivateDexSuite` (`zk` feature).
    pub attach_pdex: bool,
}

impl TerpNetworkDeployData {
    pub fn attach_existing() -> Self {
        Self {
            admin: None,
            attach_auth: true,
            attach_orch: true,
            attach_pdex: true,
        }
    }

    /// Lab default: bind whatever is in state; do not force unused families.
    pub fn local_default(sender: Addr) -> Self {
        Self {
            admin: Some(sender),
            attach_auth: true,
            attach_orch: true,
            attach_pdex: true,
        }
    }
}
