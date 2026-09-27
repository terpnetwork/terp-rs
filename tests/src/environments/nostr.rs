//! NostrTestEnv — combines a cw-orch `Daemon` chain environment with a local
//! Nostr relay container for integration testing.
//!
//! # Architecture
//!
//! `NostrTestEnv` wraps two major components:
//!
//! 1. **cw-orch `Daemon`** — the real-chain execution environment that implements
//!    [`TxHandler`], [`QueryHandler`], [`ChainState`], and therefore [`CwEnv`].
//!    Access it via [`environment()`] to perform chain operations (upload, instantiate,
//!    execute, query, migrate).
//!
//! 2. **`NostrRelayerManager`** — an ict-rs managed Docker container running a NIP-01
//!    Nostr relay. Access it via [`nostr_client()`] to publish and subscribe to events.
//!
//! # Usage
//!
//! ```ignore
//! use terp_scripts::nostr_env::NostrTestEnv;
//! use cw_orch::prelude::*;
//!
//! #[tokio::test]
//! async fn test_contract_with_nostr() {
//!     // Mnemonic from env (`ICT_TEST_MNEMONIC` / `TEST_MNEMONIC`) — never a source literal.
//!     let mnemonic = std::env::var("ICT_TEST_MNEMONIC").expect("ICT_TEST_MNEMONIC");
//!     let env = NostrTestEnv::start("my-test", &mnemonic)
//!         .await
//!         .expect("Failed to start test env");
//!
//!     // Chain operations via Environment<Daemon>
//!     let daemon: &Daemon = env.environment();
//!     let resp = daemon.upload(&contract).unwrap();
//!
//!     // Nostr operations directly
//!     let mut client = env.nostr_client().await.unwrap();
//!     client.send_event(my_event).await.unwrap();
//!
//!     // Cleanup
//!     env.stop().await.unwrap();
//! }
//! ```
//!
//! [`environment()`]: Environment::environment
//! [`CwEnv`]: cw_orch_core::environment::CwEnv
//! [`TxHandler`]: cw_orch_core::environment::TxHandler
//! [`QueryHandler`]: cw_orch_core::environment::QueryHandler
//! [`ChainState`]: cw_orch_core::environment::ChainState

use std::sync::Arc;
use anyhow::Context;
use cw_orch::daemon::{DaemonAsync, DaemonAsyncBuilder};
use cw_orch::environment::{
    ChainInfoOwned, ChainKind, Environment, NetworkInfoOwned,
};
pub use ict_rs::chain::SidecarConfig;
pub use ict_rs::nostr::NostrClient;
pub use ict_rs::nostr::NostrEvent;
pub use ict_rs::nostr::NostrRelayerManager;
use ict_rs::runtime::{IctRuntime, RuntimeBackend};
use ict_rs::cosmos::event_watcher::ChainEventWatcher;

/// Combined test environment: cw-orch `DaemonAsync` chain environment + local Nostr relay container.
///
/// Implements [`Environment<DaemonAsync>`] so that the underlying chain execution
/// environment is accessible through [`environment()`].
///
/// [`environment()`]: Environment::environment
#[derive(Clone)]
pub struct NostrTestEnv {
    /// Test identifier (used in container naming).
    name: String,
    /// cw-orch DaemonAsync for chain interactions (Arc-wrapped for Clone).
    daemon: Arc<DaemonAsync>,
    /// Managed Nostr relay Docker container.
    relayer: NostrRelayerManager,
    /// Chain WebSocket URL for event watcher.
    chain_ws_url: String,
    /// Nostr relay WebSocket URL (from the relayer manager).
    nostr_ws_url: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

impl NostrTestEnv {
    /// Start a complete test environment.
    ///
    /// This will:
    /// 1. Initialise a cw-orch `Daemon` connected to the local chain
    /// 2. Start a Nostr relay Docker container
    /// 3. Return a ready-to-use `NostrTestEnv`
    ///
    /// # Parameters
    ///
    /// * `name` — unique test name (used in container naming).
    /// * `chain_info` — chain connection info (RPC/gRPC URLs, denom, chain-id).
    /// * `mnemonic` — wallet mnemonic for the test sender.
    /// * `nostr_image` — optional custom Docker image for the Nostr relay.
    ///
    /// # Docker Requirement
    ///
    /// This method requires the Docker daemon to be running and the
    /// `mattn/nostr-relay:latest` image to be pullable (or already cached).
    /// If Docker is unavailable, the relay container creation will fail.
    ///
    /// Tests that call this should be marked `#[ignore]` or gated on a
    /// `#[cfg(feature = "docker")]` feature to avoid failing in CI environments
    /// without Docker.
    pub async fn start(
        name: &str,
        chain_info: ChainInfoOwned,
        mnemonic: &str,
    ) -> anyhow::Result<Self> {
        // ── 1. Build the ict-rs Docker runtime ──────────────────────────
        let runtime_backend = IctRuntime::Docker(Default::default())
            .into_backend()
            .await
            .context("Failed to create Docker runtime backend")?;

        // ── 2. Start the Nostr relay container ──────────────────────────
        let mut relayer = NostrRelayerManager::new(runtime_backend.clone(), name);
        let nostr_ws_url = relayer
            .start()
            .await
            .context("Failed to start Nostr relay container")?;
        tracing::info!(ws_url = %nostr_ws_url, "Nostr relay started");

        // ── 3. Build the cw-orch DaemonAsync ───────────────────────────
        let daemon = DaemonAsyncBuilder::new(chain_info)
            .mnemonic(mnemonic)
            .is_test(true)
            .build()
            .await
            .context("Failed to build cw-orch DaemonAsync")?;

        // ── 4. Determine chain WS URL from the daemon's gRPC endpoint ――――
        // The DaemonAsync's chain_info has grpc_urls. We derive the WS endpoint
        // from the first configured RPC port (default 26657).
        let rpc_port = daemon
            .chain_info()
            .grpc_urls
            .first()
            .and_then(|url| {
                // Expect "http://host:port" — extract port
                url.rsplit(':').next().and_then(|p| p.parse::<u16>().ok())
            })
            .unwrap_or(26657);
        let chain_ws_url = format!("ws://127.0.0.1:{}/websocket", rpc_port);

        Ok(Self {
            name: name.to_string(),
            daemon: Arc::new(daemon),
            relayer,
            chain_ws_url,
            nostr_ws_url,
        })
    }

    /// Create a test environment using a pre-existing runtime backend.
    ///
    /// Useful for tests that need to share a Docker connection or use
    /// a mock backend for unit testing.
    pub async fn start_with_runtime(
        name: &str,
        chain_info: ChainInfoOwned,
        mnemonic: &str,
        runtime: Arc<dyn RuntimeBackend>,
    ) -> anyhow::Result<Self> {
        let mut relayer = NostrRelayerManager::new(runtime.clone(), name);
        let nostr_ws_url = relayer
            .start()
            .await
            .context("Failed to start Nostr relay container")?;

        let daemon = DaemonAsyncBuilder::new(chain_info)
            .mnemonic(mnemonic)
            .is_test(true)
            .build()
            .await
            .context("Failed to build cw-orch DaemonAsync")?;

        let rpc_port = daemon
            .chain_info()
            .grpc_urls
            .first()
            .and_then(|url| url.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()))
            .unwrap_or(26657);
        let chain_ws_url = format!("ws://127.0.0.1:{}/websocket", rpc_port);

        Ok(Self {
            name: name.to_string(),
            daemon: Arc::new(daemon),
            relayer,
            chain_ws_url,
            nostr_ws_url,
        })
    }

    /// Create a `ChainEventWatcher` subscribed to the local chain's WebSocket.
    ///
    /// This allows tests to receive real-time chain events and bridge them
    /// to Nostr events.
    pub async fn event_watcher(&self) -> anyhow::Result<ChainEventWatcher> {
        ChainEventWatcher::connect(&self.chain_ws_url, None)
            .await
            .context("Failed to connect ChainEventWatcher")
    }

    /// Connect a `NostrClient` to the local relay.
    ///
    /// Returns a client that can publish and subscribe to Nostr events
    /// via NIP-01 messages.
    pub async fn nostr_client(&self) -> anyhow::Result<NostrClient> {
        NostrClient::connect(&self.nostr_ws_url)
            .await
            .context("Failed to connect NostrClient to local relay")
    }

    /// The Nostr relay WebSocket URL.
    pub fn nostr_ws_url(&self) -> &str {
        &self.nostr_ws_url
    }

    /// The chain WebSocket URL for Tendermint event subscriptions.
    pub fn chain_ws_url(&self) -> &str {
        &self.chain_ws_url
    }

    /// The test name identifier.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Stop the Nostr relay and clean up resources.
    ///
    /// The Daemon (cw-orch) is left intact — it can continue to be used
    /// for queries after the relay is stopped.
    pub async fn stop(&mut self) -> anyhow::Result<()> {
        self.relayer
            .stop()
            .await
            .context("Failed to stop Nostr relay")?;
        Ok(())
    }

    /// Build a default `ChainInfoOwned` for a local Terp testnet.
    ///
    /// This is a convenience for quick test setup. For custom chain
    /// configurations, construct `ChainInfoOwned` directly.
    pub fn local_terp_chain_info(
        chain_id: &str,
        grpc_url: &str,
        gas_denom: &str,
    ) -> ChainInfoOwned {
        ChainInfoOwned {
            chain_id: chain_id.to_string(),
            gas_denom: gas_denom.to_string(),
            gas_price: 0.025,
            grpc_urls: vec![grpc_url.to_string()],
            lcd_url: None,
            fcd_url: None,
            network_info: NetworkInfoOwned {
                chain_name: "terp".to_string(),
                pub_address_prefix: "terp".to_string(),
                coin_type: 118,
            },
            kind: ChainKind::Local,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Environment<DaemonAsync> implementation
// ─────────────────────────────────────────────────────────────────────────────

impl Environment<DaemonAsync> for NostrTestEnv {
    /// Access the underlying cw-orch `DaemonAsync` for chain operations.
    ///
    /// The returned `DaemonAsync` implements `TxHandler`, `QueryHandler`, `ChainState`,
    /// and therefore `CwEnv`. Use it to upload, instantiate, execute, migrate,
    /// and query contracts.
    fn environment(&self) -> &DaemonAsync {
        self.daemon.as_ref()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Drop — warn on missing cleanup
// ─────────────────────────────────────────────────────────────────────────────

impl Drop for NostrTestEnv {
    fn drop(&mut self) {
        if !self.nostr_ws_url.is_empty() {
            tracing::warn!(
                test = %self.name,
                "NostrTestEnv dropped without calling stop() — relay container may leak. \
                 Call .stop().await before dropping."
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper: build a Nostr event from chain event data
// ─────────────────────────────────────────────────────────────────────────────

/// Convert hash-market `Nip01Event` (or compatible) into ict-rs `NostrEvent`.
#[cfg(feature = "hash-market")]
pub fn nip01_to_ict(ev: hash_market::Nip01Event) -> NostrEvent {
    NostrEvent {
        id: ev.id,
        pubkey: ev.pubkey,
        created_at: ev.created_at,
        kind: ev.kind,
        tags: ev.tags,
        content: ev.content,
        sig: ev.sig,
    }
}

/// Create a NostrEvent from chain metadata.
///
/// Bridge function: wasm attributes → NIP-52-shaped kind 31922 event for the
/// relay. Prefer [`calendar_meta_to_nostr`] when MetadataExt / content body is
/// available (full Phase B egress).
pub fn chain_event_to_nostr(
    attrs: &[(String, String)],
    height: u64,
    action: Option<&str>,
    contract: Option<&str>,
    pubkey: &str,
) -> NostrEvent {
    #[cfg(feature = "hash-market")]
    {
        return nip01_to_ict(hash_market::chain_attrs_to_nostr(
            attrs, height, action, contract, pubkey,
        ));
    }
    #[cfg(not(feature = "hash-market"))]
    {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        use std::time::{SystemTime, UNIX_EPOCH};
        let content = serde_json::json!({
            "chain_height": height,
            "action": action,
            "contract": contract,
            "attributes": attrs,
        })
        .to_string();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        NostrEvent {
            id: format!("{:x}", hasher.finish()),
            pubkey: pubkey.to_string(),
            created_at: now,
            kind: 31922,
            tags: vec![
                vec!["t".to_string(), "chain-event".to_string()],
                vec!["h".to_string(), height.to_string()],
            ],
            content,
            sig: "00".repeat(32),
        }
    }
}

/// Phase B egress: calendar metadata (+ optional resolved off-chain body) → NIP-52.
///
/// * Off-chain: pass body from `hash_market::resolve_local` / `resolve_http`.
/// * On-chain: set `meta.e_json` and omit body (or pass same bytes).
#[cfg(feature = "hash-market")]
pub fn calendar_meta_to_nostr(
    meta: &hash_market::CalendarMetaView,
    body: Option<&[u8]>,
    action: Option<&hash_market::CalendarChainAction>,
    pubkey: &str,
) -> anyhow::Result<NostrEvent> {
    let ev = hash_market::metadata_to_nip52_event(meta, body, action, pubkey)
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(nip01_to_ict(ev))
}

/// End-to-end helper for tests: bind body → off-chain meta → NIP-52 event.
///
/// Uses TreeStore as sole BlobStore (no second calendar storage path).
#[cfg(feature = "hash-market")]
pub fn bind_and_bridge_offchain(
    store: &hash_market::store::TreeStore,
    body: &[u8],
    kind: u16,
    d_tag: Option<&str>,
    calendar_d: Option<&str>,
    pubkey: &str,
    action: Option<&hash_market::CalendarChainAction>,
) -> anyhow::Result<(hash_market::OffchainBind, NostrEvent)> {
    let (bind, mut meta) = hash_market::bind_offchain_event(store, body, kind)
        .map_err(|e| anyhow::anyhow!(e))?;
    if let Some(d) = d_tag {
        meta.d_tag = Some(d.to_string());
    }
    if let Some(c) = calendar_d {
        meta.calendar_d = Some(c.to_string());
    }
    meta.author_pubkey = Some(pubkey.to_string());
    let resolved = hash_market::resolve_local(store, &meta.cid).map_err(|e| anyhow::anyhow!(e))?;
    let view = hash_market::CalendarMetaView {
        on_chain: false,
        e_json: None,
        cid: Some(meta.cid),
        kind: meta.kind,
        d_tag: meta.d_tag,
        calendar_d: meta.calendar_d,
        author_pubkey: meta.author_pubkey,
        nostr_e_d: None,
    };
    let ev = calendar_meta_to_nostr(&view, Some(&resolved), action, pubkey)?;
    Ok((bind, ev))
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ict_rs::runtime::mock::MockRuntime;

    /// Test that NostrTestEnv can be created with a mock backend.
    #[tokio::test]
    async fn test_nostr_test_env_create_with_mock() {
        let runtime = Arc::new(MockRuntime::new());

        let chain_info = ChainInfoOwned {
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
        };

        // Note: DaemonBuilder will fail with MockBackend because it needs a real chain.
        // This test only validates the relayer manager integration.
        let relayer_only = NostrRelayerManager::new(runtime, "mock-test");
        let mut relayer = relayer_only;
        let url = relayer.start().await.unwrap();
        assert!(url.starts_with("ws://127.0.0.1:"));

        // Create the env through start_with_runtime — this will fail on Daemon build
        // but the relayer container was already started successfully.
        // Daemon path requires a caller-supplied mnemonic; this unit test expects
        // gRPC/Daemon build to fail on MockRuntime. Empty seed is intentional (not a BIP-39).
        let mnemonic = std::env::var("ICT_TEST_MNEMONIC").unwrap_or_default();
        let result = NostrTestEnv::start_with_runtime(
            "mock-test",
            chain_info.clone(),
            &mnemonic,
            Arc::new(MockRuntime::new()),
        )
        .await;

        // The Daemon build fails because MockBackend doesn't provide a real gRPC endpoint.
        assert!(result.is_err(), "Expected Daemon build to fail without real chain");

        // Cleanup the relay
        relayer.stop().await.unwrap();
    }

    #[test]
    fn test_chain_event_to_nostr_conversion() {
        let attrs = vec![("action".into(), "create_event".into())];
        let result = chain_event_to_nostr(
            &attrs,
            42,
            Some("create_event"),
            Some("terp1test123"),
            "test_pubkey_abc",
        );
        assert_eq!(result.kind, 31922);
        assert!(result.content.contains("chain_height"));
        assert!(result.content.contains("42"));
    }

    #[cfg(feature = "hash-market")]
    #[test]
    fn test_bind_and_bridge_offchain_nip52() {
        use hash_market::store::TreeStore;
        use hash_market::{CalendarChainAction, KIND_TIME_BASED};

        let dir = std::env::temp_dir().join(format!("nostr-env-bind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = TreeStore::open(dir.join("trees")).unwrap();
        let body = br#"{"title":"Suite Event","content":"hi","d_tag":"s1"}"#;
        let action = CalendarChainAction {
            action: "create_event".into(),
            e_d: Some("evt/cal/1/1".into()),
            d: Some("cal/1".into()),
            contract: Some("terp1cal".into()),
            height: 10,
        };
        let (bind, ev) = bind_and_bridge_offchain(
            &store,
            body,
            KIND_TIME_BASED,
            Some("s1"),
            Some("cal/1"),
            "pk_test",
            Some(&action),
        )
        .unwrap();
        assert_eq!(bind.chain_cid.len(), 64);
        assert_eq!(ev.kind, KIND_TIME_BASED as u32);
        assert!(ev.tags.iter().any(|t| t.first().map(|s| s.as_str()) == Some("cid")));
        assert!(ev.content.contains("Suite Event"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_env_cloneable() {
        // Verify that NostrTestEnv is Clone (required for CwEnv compat)
        fn assert_clone<T: Clone>() {}
        assert_clone::<NostrTestEnv>();
    }
}