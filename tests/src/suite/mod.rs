//! SidecarFleet — composable fleet of Terp Network sidecars (Docker, subprocess, tokio).
//!
//! Authenticator cw-orch suite crate: `tests/suite/terp-authenticator-suite` (re-exported from [`auth`]).
//!
//! Combines a cw-orch `Daemon` chain environment with typed access to all
//! Terp Network sidecar services: hashmerchant, merkle-server, indexer,
//! relayer, minio-ipfs, and nostr-relay.
//!
//! Each sidecar element is a typed field with its own lifecycle, health
//! checking, and endpoint resolution. Sidecars are started in dependency
//! order using `SidecarRegistry` for inter-sidecar communication.

pub mod contracts;
pub mod coverage;
pub mod deploy_data;
#[cfg(feature = "hash-market")]
pub mod frost;
pub mod hashmerchant;
pub mod hashmerchant_client;
pub mod sidecar;

#[cfg(feature = "docker")]
pub mod indexer;
#[cfg(feature = "docker")]
pub mod minio_ipfs;
#[cfg(feature = "nostr")]
pub mod nostr;
#[cfg(feature = "docker")]
pub mod relayer;

use std::sync::Arc;

use anyhow::{Context, Result};
use std::collections::HashMap;
use cw_orch::daemon::Daemon;
use cw_orch::environment::{ChainInfoOwned, ChainKind, Environment, NetworkInfoOwned};

use ict_rs::runtime::IctRuntime;

pub use sidecar::{
    DockerSidecar, EndpointMap, HealthStatus, InProcessSidecar, SidecarBinaryResolver,
    SidecarId, SidecarRegistry, SubprocessConfig, SubprocessSidecar,
};
use sidecar::TerpSidecar;

pub use contracts::TerpNetworkSuite;
pub use contracts::TerpNetworkSuite as DeploySuite;

pub use hashmerchant::{HashMerchantSuite, MerkleServerSuite};
pub use hashmerchant_client::HashMerchantClient;
#[cfg(feature = "docker")]
use indexer::IndexerSuite;
#[cfg(feature = "docker")]
use minio_ipfs::MinioIpfsSuite;
#[cfg(feature = "nostr")]
use nostr::NostrRelaySuite;
#[cfg(feature = "docker")]
use relayer::RelayerSuite;

pub mod config;
#[cfg(feature = "auth-suite")]
pub mod auth;
#[cfg(feature = "orch-suite")]
pub mod circuit_demo;

// ---------------------------------------------------------------------------
// SidecarFleet
// ---------------------------------------------------------------------------

/// A fleet of sidecars (Docker, subprocess, tokio) with a chain Daemon.
///
/// # Example
///
/// ```ignore
/// use terp_scripts::suite::SidecarFleet;
///
/// #[tokio::test]
/// async fn test_full_suite() {
///     // Daemon mnemonic is caller-supplied (env), never a source literal.
///     let mut suite = SidecarFleet::new("my-test")
///         .with_local_chain_from_env()?
///         .with_hashmerchant_defaults("terp-test-1")
///         .with_merkle_server_defaults()
///         .with_minio_ipfs_defaults()
///         .with_nostr_relay("nostr-relay")
///         .with_minimal_indexer("argus");
///
///     suite.start_all().await.unwrap();
///
///     // Use chain
///     suite.environment().upload(&contract).unwrap();
///
///     // Use sidecars
///     let ve = suite.hashmerchant.vote_extension("terp-test").await.unwrap();
///
///     suite.stop_all().await.unwrap();
/// }
/// ```
pub struct SidecarFleet {
    /// Test name identifier
    name: String,
    /// cw-orch Daemon for chain interactions
    daemon: Option<Daemon>,
    /// Docker runtime backend
    runtime: Option<Arc<dyn ict_rs::runtime::RuntimeBackend>>,
    /// Sidecar registry for endpoint sharing
    registry: SidecarRegistry,
    /// Typed sidecar fields
    pub hashmerchant: Option<HashMerchantSuite>,
    pub merkle_server: Option<MerkleServerSuite>,
    #[cfg(feature = "docker")]
    pub minio_ipfs: Option<MinioIpfsSuite>,
    #[cfg(feature = "nostr")]
    pub nostr_relay: Option<NostrRelaySuite>,
    #[cfg(feature = "docker")]
    pub relayer: Option<RelayerSuite>,
    #[cfg(feature = "docker")]
    pub indexer: Option<IndexerSuite>,
    /// Chain info for Daemon construction
    chain_info: Option<ChainInfoOwned>,
    mnemonic: Option<String>,
}

impl SidecarFleet {
    /// Create a new suite builder. Call `.with_*()` methods to add sidecars,
    /// then call `.start_all()` to launch everything.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            daemon: None,
            runtime: None,
            registry: SidecarRegistry::new(),
            hashmerchant: None,
            merkle_server: None,
            #[cfg(feature = "docker")]
            minio_ipfs: None,
            #[cfg(feature = "nostr")]
            nostr_relay: None,
            #[cfg(feature = "docker")]
            relayer: None,
            #[cfg(feature = "docker")]
            indexer: None,
            chain_info: None,
            mnemonic: None,
        }
    }

    /// Create a suite with local chain info and a **caller-supplied** mnemonic for the Daemon.
    /// Never pass a hardcoded seed from source; load from env (`ICT_TEST_MNEMONIC` / `TEST_MNEMONIC`).
    pub fn with_local_chain(mut self, mnemonic: &str) -> Self {
        self.chain_info = Some(ChainInfoOwned {
            chain_id: "terp-test-1".to_string(),
            gas_denom: "uterp".to_string(),
            gas_price: 0.025,
            grpc_urls: vec!["http://127.0.0.1:9090".to_string()],
            lcd_url: None,
            fcd_url: None,
            network_info: NetworkInfoOwned {
                chain_name: "terp".to_string(),
                pub_address_prefix: "terp".to_string(),
                coin_type: 118,
            },
            kind: ChainKind::Local,
        });
        self.mnemonic = Some(mnemonic.to_string());
        self
    }

    /// Same as [`Self::with_local_chain`] but reads `ICT_TEST_MNEMONIC` then `TEST_MNEMONIC`.
    /// Errors if neither is set (CI unit tests should not call this).
    pub fn with_local_chain_from_env(self) -> Result<Self> {
        let mnemonic = std::env::var("ICT_TEST_MNEMONIC")
            .or_else(|_| std::env::var("TEST_MNEMONIC"))
            .context(
                "set ICT_TEST_MNEMONIC or TEST_MNEMONIC (never hardcode a Daemon seed in source)",
            )?;
        if mnemonic.trim().is_empty() {
            anyhow::bail!("ICT_TEST_MNEMONIC / TEST_MNEMONIC is empty");
        }
        Ok(self.with_local_chain(&mnemonic))
    }

    // ── Builder methods ──────────────────────────────────────────────────

    /// Add a HashMerchant suite with default config.
    pub fn with_hashmerchant_defaults(mut self, chain_id: &str) -> Self {
        self.hashmerchant = Some(HashMerchantSuite::with_defaults(chain_id));
        self
    }

    /// Add a custom HashMerchant suite.
    pub fn with_hashmerchant(mut self, suite: HashMerchantSuite) -> Self {
        self.hashmerchant = Some(suite);
        self
    }

    /// HashMerchant sidecar with embedded `/onboard/*` grant plane.
    pub fn with_hashmerchant_onboard(
        self,
        chain_id: &str,
        dao: impl Into<String>,
        runtime_key: impl Into<String>,
    ) -> Self {
        self.with_hashmerchant(
            HashMerchantSuite::with_defaults(chain_id).with_onboard_roles(dao, runtime_key),
        )
    }

    /// HTTP client for the running HashMerchant sidecar (after `start_all`).
    pub fn hashmerchant_http(&self) -> anyhow::Result<HashMerchantClient> {
        let hm = self
            .hashmerchant
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("SidecarFleet has no hashmerchant"))?;
        let url = hm
            .server_http_url()
            .ok_or_else(|| anyhow::anyhow!("hashmerchant not started"))?;
        Ok(HashMerchantClient::from_url(url))
    }

    /// Add a Merkle server with a fresh keypair.
    #[allow(clippy::result_unit_err)]
    pub fn with_merkle_server_defaults(mut self) -> Self {
        if let Ok(suite) = MerkleServerSuite::with_defaults() {
            self.merkle_server = Some(suite);
        }
        self
    }

    /// Add a custom Merkle server.
    pub fn with_merkle_server(mut self, suite: MerkleServerSuite) -> Self {
        self.merkle_server = Some(suite);
        self
    }

    /// Add a MinIO-IPFS sidecar with default credentials.
    #[cfg(feature = "docker")]
    pub fn with_minio_ipfs_defaults(mut self) -> Self {
        self.minio_ipfs = Some(MinioIpfsSuite::with_defaults("minio-ipfs"));
        self
    }

    /// Add a Nostr relay.
    #[cfg(feature = "nostr")]
    pub fn with_nostr_relay(mut self, id: &str) -> Self {
        self.nostr_relay = Some(NostrRelaySuite::new(id));
        self
    }

    /// Add an IBC relayer.
    #[cfg(feature = "docker")]
    pub fn with_relayer(mut self, suite: RelayerSuite) -> Self {
        self.relayer = Some(suite);
        self
    }

    /// Add a minimal Argus indexer (server + Postgres backends only).
    #[cfg(feature = "docker")]
    pub fn with_minimal_indexer(mut self, id: &str) -> Self {
        self.indexer = Some(IndexerSuite::with_defaults(id).with_minimal());
        self
    }

    /// Add a full Argus indexer (all infrastructure).
    #[cfg(feature = "docker")]
    pub fn with_full_indexer(mut self, id: &str) -> Self {
        self.indexer = Some(IndexerSuite::with_defaults(id));
        self
    }

    // ── Lifecycle ─────────────────────────────────────────────────────────

    /// Start all configured sidecars in dependency order, plus the Daemon.
    ///
    /// Dependency order:
    /// 1. MinIO-IPFS, Nostr relay (no deps)
    /// 2. Merkle server (no deps)
    /// 3. Hashmerchant server + client (depends on chain for VE, but starts independently)
    /// 4. Indexer (depends on chain for RPC)
    /// 5. Relayer (depends on both chains)
    pub async fn start_all(&mut self) -> Result<()> {
        // Initialize Docker runtime if not set
        if self.runtime.is_none() {
            let rt = IctRuntime::Docker(Default::default())
                .into_backend()
                .await
                .context("Failed to create Docker runtime backend")?;
            self.runtime = Some(rt);
        }

        // Inject runtime into sidecars that need it
        if let Some(ref rt) = self.runtime {
            #[cfg(feature = "docker")]
            #[cfg(feature = "docker")]
        if let Some(ref mut idx) = self.indexer {
                idx.runtime = Some(rt.clone());
            }
            #[cfg(feature = "nostr")]
            if let Some(ref mut nostr) = self.nostr_relay {
                nostr.runtime = Some(rt.clone());
            }
            #[cfg(feature = "docker")]
            if let Some(ref mut relayer) = self.relayer {
                relayer.runtime = Some(rt.clone());
            }
            #[cfg(feature = "docker")]
            if let Some(ref mut minio) = self.minio_ipfs {
                minio.runtime = Some(rt.clone());
            }
        }

        // Start sidecars in dependency order
        let start_order: Vec<&mut dyn TerpSidecar> = vec![
            // Tier 0: no deps
            // MinIO-IPFS and Nostr are Docker-sidecars handled via suite ops
        ];

        // Due to borrow issues with the Option fields, we start sidecars
        // by taking and replacing them. This avoids mutable borrow conflicts.

        if let Some(ref mut hm) = self.hashmerchant {
            let ep = hm.start(&self.registry).await?;
            self.registry.register("hashmerchant".into(), ep);
        }

        if let Some(ref mut ms) = self.merkle_server {
            let ep = ms.start(&self.registry).await?;
            self.registry.register("merkle-server".into(), ep);
        }

        #[cfg(feature = "docker")]
        if let Some(ref mut mini) = self.minio_ipfs {
            let ep = mini.start(&self.registry).await?;
            self.registry.register("minio-ipfs".into(), ep);
        }

        #[cfg(feature = "nostr")]
        if let Some(ref mut nr) = self.nostr_relay {
            let ep = nr.start(&self.registry).await?;
            self.registry.register("nostr-relay".into(), ep);
        }

        #[cfg(feature = "docker")]
        if let Some(ref mut rel) = self.relayer {
            let ep = rel.start(&self.registry).await?;
            self.registry.register("relayer".into(), ep);
        }

        #[cfg(feature = "docker")]
        if let Some(ref mut idx) = self.indexer {
            let ep = idx.start(&self.registry).await?;
            self.registry.register("indexer".into(), ep);
        }

        // Build Daemon if chain_info is set
        if let Some(ref chain_info) = self.chain_info {
            let mnemonic = self.mnemonic.as_deref().context("no mnemonic set")?;
            let daemon = cw_orch::daemon::DaemonBuilder::new(chain_info.clone())
                .handle(&tokio::runtime::Handle::current())
                .mnemonic(mnemonic)
                .is_test(true)
                .build()
                .context("Failed to build cw-orch Daemon")?;
            self.daemon = Some(daemon);
        }

        Ok(())
    }

    /// Stop all sidecars and clean up.
    pub async fn stop_all(&mut self) -> Result<()> {
        // Stop in reverse dependency order
        #[cfg(feature = "docker")]
        if let Some(ref mut idx) = self.indexer {
            idx.stop().await.ok();
        }
        #[cfg(feature = "docker")]
        if let Some(ref mut rel) = self.relayer {
            rel.stop().await.ok();
        }
        #[cfg(feature = "nostr")]
        if let Some(ref mut nr) = self.nostr_relay {
            nr.stop().await.ok();
        }
        #[cfg(feature = "docker")]
        if let Some(ref mut mini) = self.minio_ipfs {
            mini.stop().await.ok();
        }
        if let Some(ref mut ms) = self.merkle_server {
            ms.stop().await.ok();
        }
        if let Some(ref mut hm) = self.hashmerchant {
            hm.stop().await.ok();
        }
        Ok(())
    }

    /// Check health of all sidecars. Returns a map of sidecar_id -> health.
    pub async fn health_all(&self) -> Result<HashMap<String, HealthStatus>> {
        let mut results = HashMap::new();

        if let Some(ref hm) = self.hashmerchant {
            results.insert("hashmerchant".into(), hm.health().await.unwrap_or(HealthStatus::Unknown));
        }
        if let Some(ref ms) = self.merkle_server {
            results.insert("merkle-server".into(), ms.health().await.unwrap_or(HealthStatus::Unknown));
        }
        #[cfg(feature = "docker")]
        if let Some(ref mini) = self.minio_ipfs {
            results.insert("minio-ipfs".into(), mini.health().await.unwrap_or(HealthStatus::Unknown));
        }
        #[cfg(feature = "nostr")]
        if let Some(ref nr) = self.nostr_relay {
            results.insert("nostr-relay".into(), nr.health().await.unwrap_or(HealthStatus::Unknown));
        }
        #[cfg(feature = "docker")]
        if let Some(ref rel) = self.relayer {
            results.insert("relayer".into(), rel.health().await.unwrap_or(HealthStatus::Unknown));
        }
        #[cfg(feature = "docker")]
        if let Some(ref idx) = self.indexer {
            results.insert("indexer".into(), idx.health().await.unwrap_or(HealthStatus::Unknown));
        }

        Ok(results)
    }

    /// Get a specific endpoint from a running sidecar.
    /// Returns `None` if the sidecar isn't started or the endpoint doesn't exist.
    pub fn endpoint(&self, sidecar: &str, endpoint: &str) -> Option<String> {
        self.registry.get_endpoint(sidecar, endpoint)
    }
}

// ── Environment<Daemon> implementation ─────────────────────────────────────

impl Environment<Daemon> for SidecarFleet {
    fn environment(&self) -> &Daemon {
        self.daemon.as_ref().expect("Daemon not initialized — call start_all() first or set with_local_chain()")
    }
}

// ── Drop ────────────────────────────────────────────────────────────────────

impl Drop for SidecarFleet {
    fn drop(&mut self) {
        // Best-effort cleanup in drop context
        let _ = self.runtime.take();
    }
}