//! HashMerchantSuite — server and client sidecars for the hashmerchant module.
//!
//! The hash-market-server is a Rust binary that receives transformed hashes
//! and produces signed vote extensions via ABCI++. The client polls foreign
//! chain data and streams it to the server.
//!
//! Config types are the canonical `hash_market::config` structs — single source
//! of truth shared with the binary entry points. The suite wraps them with
//! `binary_path` for test-side binary resolution.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use async_trait::async_trait;
use k256::elliptic_curve::rand_core::OsRng;
use serde::{Deserialize, Serialize};
use tracing::info;

use hash_market::config::{ClientConfig, Config, ProviderConfig};
use hash_market::ve::ProviderStatus;

use crate::suite::SidecarBinaryResolver;
use crate::suite::sidecar::{
    EndpointMap, HealthStatus, SidecarId, SidecarRegistry, SubprocessConfig, SubprocessSidecar,
    TerpSidecar,
};

// ---------------------------------------------------------------------------
// MerchantSuiteConfig — thin wrapper around canonical Config
// ---------------------------------------------------------------------------

/// Test-suite wrapper for the canonical `hash_market::config::Config`.
///
/// Adds a `binary_path` override that is not part of the TOML serialization.
/// `SubprocessConfig::write_config` serializes `self.inner` to TOML directly.
#[derive(Clone, Debug)]
pub struct MerchantSuiteConfig {
    pub inner: Config,
    /// Optional override for the binary path (auto-resolved if None)
    pub binary_path: Option<PathBuf>,
}

impl MerchantSuiteConfig {
    pub fn new(config: Config) -> Self {
        Self {
            inner: config,
            binary_path: None,
        }
    }

    pub fn with_binary_path(mut self, path: PathBuf) -> Self {
        self.binary_path = Some(path);
        self
    }
}

impl SubprocessConfig for MerchantSuiteConfig {
    fn binary_path(&self) -> Result<PathBuf> {
        if let Some(ref p) = self.binary_path {
            if p.exists() {
                return Ok(p.canonicalize()?);
            }
        }

        // Check well-known locations relative to CARGO_MANIFEST_DIR
        let resolver = SidecarBinaryResolver::cargo_binary(
            vec![
                "../target/debug/hash-market-server",
                "../target/release/hash-market-server",
                "../../tools/hash-market/target/debug/hash-market-server",
            ],
            "hash-market-server",
        );
        resolver.resolve()
    }

    fn args(&self, config_path: &Path) -> Vec<String> {
        vec!["-c".to_string(), config_path.to_string_lossy().to_string()]
    }

    fn env(&self) -> Vec<(String, String)> {
        vec![]
    }

    fn write_config(&self, config_dir: &Path) -> Result<PathBuf> {
        let config_path = config_dir.join("config.toml");
        let content =
            toml::to_string(&self.inner).context("failed to serialize hashmerchant config")?;
        std::fs::write(&config_path, &content).context("failed to write hashmerchant config")?;
        info!(
            path = %config_path.display(),
            "wrote hashmerchant server config"
        );
        Ok(config_path)
    }

    fn health_check(&self) -> Option<(String, String)> {
        Some(("GET".to_string(), "/health".to_string()))
    }

    fn bind_port(&self) -> u16 {
        self.inner
            .bind
            .rsplit(':')
            .next()
            .and_then(|s| {
                let port: u16 = s.parse().ok()?;
                Some(port)
            })
            .unwrap_or_else(|| {
                tracing::warn!(
                    bind = %self.inner.bind,
                    "could not parse port from bind address, defaulting to 9090"
                );
                9090
            })
    }

    fn startup_timeout_secs(&self) -> u64 {
        30
    }
}

// ---------------------------------------------------------------------------
// MerchantClientSuiteConfig — thin wrapper around canonical ClientConfig
// ---------------------------------------------------------------------------

/// Test-suite wrapper for `hash_market::config::ClientConfig`.
#[derive(Clone, Debug)]
pub struct MerchantClientSuiteConfig {
    pub inner: ClientConfig,
    pub binary_path: Option<PathBuf>,
}

impl SubprocessConfig for MerchantClientSuiteConfig {
    fn binary_path(&self) -> Result<PathBuf> {
        if let Some(ref p) = self.binary_path {
            if p.exists() {
                return Ok(p.canonicalize()?);
            }
        }
        let resolver = SidecarBinaryResolver::cargo_binary(
            vec![
                "../../target/debug/hash-market-client",
                "../../target/release/hash-market-client",
            ],
            "hash-market-client",
        );
        resolver.resolve()
    }

    fn args(&self, config_path: &Path) -> Vec<String> {
        vec!["-c".to_string(), config_path.to_string_lossy().to_string()]
    }

    fn env(&self) -> Vec<(String, String)> {
        vec![]
    }

    fn write_config(&self, config_dir: &Path) -> Result<PathBuf> {
        let config_path = config_dir.join("client.toml");
        let content = toml::to_string(&self.inner)
            .context("failed to serialize hashmerchant client config")?;
        std::fs::write(&config_path, &content)?;
        Ok(config_path)
    }

    fn health_check(&self) -> Option<(String, String)> {
        None // client has no HTTP endpoint
    }

    fn bind_port(&self) -> u16 {
        0 // no port needed
    }

    fn startup_timeout_secs(&self) -> u64 {
        10
    }
}

// ---------------------------------------------------------------------------
// HashMerchantSuite
// ---------------------------------------------------------------------------

/// Combined hashmerchant suite — server + client.
pub struct HashMerchantSuite {
    id: SidecarId,
    server: SubprocessSidecar<MerchantSuiteConfig>,
    client: Option<SubprocessSidecar<MerchantClientSuiteConfig>>,
    endpoints: EndpointMap,
    started: bool,
}

impl HashMerchantSuite {
    /// Create a hashmerchant suite with a canonical `Config`.
    pub fn new(id: impl Into<String>, config: Config) -> Self {
        Self {
            id: id.into(),
            server: SubprocessSidecar::new("hm-server", MerchantSuiteConfig::new(config)),
            client: None,
            endpoints: EndpointMap::new(),
            started: false,
        }
    }

    /// Create a hashmerchant suite from a pre-built `MerchantSuiteConfig`.
    pub fn with_config(id: impl Into<String>, config: MerchantSuiteConfig) -> Self {
        Self {
            id: id.into(),
            server: SubprocessSidecar::new("hm-server", config),
            client: None,
            endpoints: EndpointMap::new(),
            started: false,
        }
    }

    /// Merkle/BUD host + embedded  (no VE). Local production sidecar.
    pub fn mint_host_onboard(
        chain_id: &str,
        bind: &str,
        dao: impl Into<String>,
        runtime_key: impl Into<String>,
    ) -> Self {
        let dao = dao.into();
        let runtime_key = runtime_key.into();
        let config = Config {
            bind: bind.into(),
            chain_id: chain_id.to_string(),
            signing_key: None,
            data_dir: None,
            ve_enabled: false,
            providers: vec![],
            oracle: None,
            cors_allow_origin: None,
            distribution: None,
            notes_auth: None,
            cashu_mesh: None,
            onboard: Some(hash_market::config::OnboardConfig {
                enabled: true,
                dao: Some(dao),
                runtime_key: Some(runtime_key),
                chain_id: Some(chain_id.to_string()),
                register_limit: None,
                mint_limit: None,
                broadcast: false,
            }),
            privval: None,
        };
        Self::new("hashmerchant-mint", config)
    }

    /// Add a hash-market-client that polls an Ethereum node.
    pub fn with_client(mut self, client_config: ClientConfig) -> Self {
        self.client = Some(SubprocessSidecar::new(
            "hm-client",
            MerchantClientSuiteConfig {
                inner: client_config,
                binary_path: None,
            },
        ));
        self
    }

    /// Build a default config for a local test:
    /// - random ephemeral signing key (64 hex chars)
    /// - single keccak256 provider pointing at localhost:8545
    /// - chain_id supplied by caller
    pub fn with_defaults(chain_id: &str) -> Self {
        // Generate ephemeral secp256k1 signing key
        let secret = k256::SecretKey::random(&mut OsRng);
        let signing_key = hex::encode(secret.to_bytes());

        let config = Config {
            bind: "127.0.0.1:9090".into(),
            chain_id: chain_id.to_string(),
            signing_key: Some(signing_key),
            data_dir: None,
            ve_enabled: true,
            providers: vec![ProviderConfig {
                name: "local-test".into(),
                chain_uid: "terp-test".into(),
                algo: "keccak256".into(),
                mode: "http_poll".into(),
                address: "http://127.0.0.1:8545".into(),
                interval_secs: 12,
                kind: None,
                market_id: None,
                weight: None,
                static_root: None,
                static_height: None,
                static_block_time: None,
            }],
            oracle: None,
            cors_allow_origin: None,
            distribution: None,
            notes_auth: None,
            cashu_mesh: None,
            onboard: None,
            privval: None,
        };
        Self::new("hashmerchant", config)
    }

    /// Same plane methods as in-process tests (`HashMerchantClient`).
    pub fn http_client(&self) -> Result<crate::suite::HashMerchantClient> {
        let url = self
            .server_http_url()
            .context("hashmerchant server not started")?;
        Ok(crate::suite::HashMerchantClient::from_url(url))
    }

    /// The server's HTTP URL (e.g. "http://127.0.0.1:9090")
    pub fn server_http_url(&self) -> Option<String> {
        self.endpoints.get("http").cloned()
    }

    /// Query the /ve/vote-extension endpoint for a given chain UID.
    pub async fn vote_extension(&self, chain_uid: &str) -> Result<String> {
        let base = self
            .server_http_url()
            .context("hashmerchant server not started")?;
        let url = format!(
            "{}/ve/vote-extension?chain_uid={}",
            base.trim_end_matches('/'),
            chain_uid
        );
        let resp = reqwest::get(&url).await?;
        let body = resp.text().await?;
        Ok(body)
    }

    /// Query registered provider runtime statuses via the /ve/providers endpoint.
    pub async fn provider_statuses(&self) -> Result<Vec<ProviderStatus>> {
        let base = self
            .server_http_url()
            .context("hashmerchant server not started")?;
        let url = format!("{}/ve/providers", base.trim_end_matches('/'));
        let resp = reqwest::get(&url).await?;
        let statuses: Vec<ProviderStatus> = resp
            .json()
            .await
            .context("failed to deserialize provider statuses")?;
        Ok(statuses)
    }

    /// Feed a provider's data directly by calling the internal gRPC endpoint.

    /// Enable embedded `/onboard/*` on the sidecar (no second onboard-runtime).
    /// Rebuild `MerchantSuiteConfig` so TOML includes `[onboard]`.
    pub fn with_onboard_roles(
        self,
        dao: impl Into<String>,
        runtime_key: impl Into<String>,
    ) -> Self {
        let mut cfg = self.server_config_clone();
        cfg.inner.onboard = Some(hash_market::config::OnboardConfig {
            enabled: true,
            dao: Some(dao.into()),
            runtime_key: Some(runtime_key.into()),
            chain_id: Some(cfg.inner.chain_id.clone()),
            register_limit: None,
            mint_limit: None,
            broadcast: false,
        });
        let mut s = Self::with_config(self.id.clone(), cfg);
        s.client = self.client;
        s
    }

    fn server_config_clone(&self) -> MerchantSuiteConfig {
        self.server.config().clone()
    }

    /// `GET /onboard/status` — grant plane, not merchant `/health`.
    pub async fn onboard_status(&self) -> Result<serde_json::Value> {
        let base = self
            .server_http_url()
            .context("hashmerchant server not started")?;
        let url = format!("{}/onboard/status", base.trim_end_matches('/'));
        let resp = reqwest::get(&url).await?;
        let v = resp.json().await.context("onboard status json")?;
        Ok(v)
    }

    pub async fn request_register_grant(&self, address: &str) -> Result<serde_json::Value> {
        self.onboard_grant("register-grant", address).await
    }

    pub async fn request_mint_grant(&self, address: &str) -> Result<serde_json::Value> {
        self.onboard_grant("mint-grant", address).await
    }

    async fn onboard_grant(&self, which: &str, address: &str) -> Result<serde_json::Value> {
        let base = self
            .server_http_url()
            .context("hashmerchant server not started")?;
        let url = format!("{}/onboard/v1/{}", base.trim_end_matches('/'), which);
        let resp = reqwest::Client::new()
            .post(&url)
            .json(&serde_json::json!({ "address": address }))
            .send()
            .await?;
        let v = resp.json().await.context("onboard grant json")?;
        Ok(v)
    }

    pub async fn feed_provider(&self, _provider: &str, _root: &[u8], _height: u64) -> Result<()> {
        // In practice this calls the hashmarket internal gRPC endpoint.
        // For now, a no-op — the actual gRPC client lives in terp_rs proto.
        Ok(())
    }
}

#[async_trait]
impl TerpSidecar for HashMerchantSuite {
    fn id(&self) -> &SidecarId {
        &self.id
    }

    async fn start(&mut self, deps: &SidecarRegistry) -> Result<EndpointMap> {
        if self.started {
            return Ok(self.endpoints.clone());
        }

        // 1. Start server first
        let server_endpoints = self.server.start(deps).await?;
        self.endpoints.extend(server_endpoints);

        // 2. Optionally start client (depends on server)
        if let Some(ref mut client) = self.client {
            let _client_endpoints = client.start(deps).await?;
        }

        self.started = true;
        Ok(self.endpoints.clone())
    }

    async fn stop(&mut self) -> Result<()> {
        // Stop client first, then server
        if let Some(ref mut client) = self.client {
            let _ = client.stop().await;
        }
        self.server.stop().await?;
        self.started = false;
        Ok(())
    }

    async fn health(&self) -> Result<HealthStatus> {
        self.server.health().await
    }

    fn endpoint(&self, name: &str) -> Option<String> {
        self.endpoints.get(name).cloned()
    }
}

// ---------------------------------------------------------------------------
// MerkleServerSuite — BLAKE3 merkle proof server for whitelists
// ---------------------------------------------------------------------------
//
// Stores and serves merkle proofs for the cw-whitelist-merkletree contract.
// Config format mirrors `tools/merkle-server/src/main.rs` (JSON).
// This binary lives in a separate crate, so its config types are defined here
// rather than in the hash-market library.
//
// (No reimplementation — this is a different binary.)

// ---------------------------------------------------------------------------
// MerkleServerConfig
// ---------------------------------------------------------------------------

/// Configuration for the merkle-server binary.
///
/// NOTE: If `tools/merkle-server/src/main.rs` changes its config shape, update
/// this struct to match. It is the single source of truth for test JSON config.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MerkleServerConfig {
    /// 66-char hex compressed secp256k1 public key
    pub public_key: String,
    /// Data directory for tree storage
    pub data_dir: PathBuf,
    /// Bind host
    pub host: String,
    /// Bind port
    pub port: u16,
    /// Timestamp tolerance in seconds (default: 300)
    pub timestamp_tolerance_s: u64,
    /// Optional: path to the binary (auto-resolved if None)
    pub binary_path: Option<PathBuf>,
}

impl Default for MerkleServerConfig {
    fn default() -> Self {
        Self {
            public_key: String::new(),
            data_dir: PathBuf::from("./data"),
            host: "127.0.0.1".into(),
            port: 8765,
            timestamp_tolerance_s: 300,
            binary_path: None,
        }
    }
}

impl MerkleServerConfig {
    /// Generate a fresh keypair and return the config + hex private key.
    pub fn with_new_keypair() -> Result<(Self, String)> {
        let secret = k256::SecretKey::random(&mut OsRng);
        let sk_hex = hex::encode(secret.to_bytes());
        let pk = secret.public_key();
        let pk_hex = hex::encode(pk.to_sec1_bytes());

        Ok((
            Self {
                public_key: pk_hex,
                data_dir: PathBuf::from("./data"),
                host: "127.0.0.1".into(),
                port: 8765,
                timestamp_tolerance_s: 300,
                binary_path: None,
            },
            sk_hex,
        ))
    }
}

impl SubprocessConfig for MerkleServerConfig {
    fn binary_path(&self) -> Result<PathBuf> {
        if let Some(ref p) = self.binary_path {
            if p.exists() {
                return Ok(p.canonicalize()?);
            }
        }
        let resolver = SidecarBinaryResolver::cargo_binary(
            vec![
                "../../target/debug/merkle-server",
                "../../target/release/merkle-server",
                "../../tools/merkle-server/target/debug/merkle-server",
            ],
            "merkle-server",
        );
        resolver.resolve()
    }

    fn args(&self, config_path: &Path) -> Vec<String> {
        vec![
            "serve".to_string(),
            "--config".to_string(),
            config_path.to_string_lossy().to_string(),
        ]
    }

    fn env(&self) -> Vec<(String, String)> {
        vec![]
    }

    fn write_config(&self, config_dir: &Path) -> Result<PathBuf> {
        // Serialize MerkleServerConfig directly via derive(Serialize).
        // No separate JsonConfig shim — single source of truth.
        let config_path = config_dir.join("config.json");
        let json = serde_json::json!({
            "public_key": self.public_key,
            "data_dir": config_dir.join("data").to_string_lossy(),
            "host": self.host,
            "port": self.port,
            "timestamp_tolerance_s": self.timestamp_tolerance_s,
        });
        let content = serde_json::to_string_pretty(&json)
            .context("failed to serialize merkle-server config")?;
        std::fs::create_dir_all(config_dir.join("data"))?;
        std::fs::write(&config_path, &content).context("failed to write merkle-server config")?;
        info!(
            path = %config_path.display(),
            "wrote merkle server config"
        );
        Ok(config_path)
    }

    fn health_check(&self) -> Option<(String, String)> {
        Some(("GET".to_string(), "/health".to_string()))
    }

    fn bind_port(&self) -> u16 {
        self.port
    }

    fn startup_timeout_secs(&self) -> u64 {
        15
    }
}

// ---------------------------------------------------------------------------
// MerkleServerSuite
// ---------------------------------------------------------------------------

/// Merkle proof server for whitelists.
pub struct MerkleServerSuite {
    id: SidecarId,
    inner: SubprocessSidecar<MerkleServerConfig>,
    endpoints: EndpointMap,
    started: bool,
    private_key: Option<String>,
}

impl MerkleServerSuite {
    /// Create a new merkle server with the given config.
    pub fn new(id: impl Into<String>, config: MerkleServerConfig) -> Self {
        Self {
            id: id.into(),
            inner: SubprocessSidecar::new("merkle-server", config),
            endpoints: EndpointMap::new(),
            started: false,
            private_key: None,
        }
    }

    /// Create a merkle server with a fresh keypair.
    /// Returns the private key for signing upload/delete requests.
    pub fn with_new_keypair(id: impl Into<String>) -> Result<Self> {
        let (config, sk) = MerkleServerConfig::with_new_keypair()?;
        Ok(Self {
            id: id.into(),
            inner: SubprocessSidecar::new("merkle-server", config),
            endpoints: EndpointMap::new(),
            started: false,
            private_key: Some(sk),
        })
    }

    /// Create with defaults: fresh keypair, port 8765.
    pub fn with_defaults() -> Result<Self> {
        Self::with_new_keypair("merkle-server")
    }

    /// The server's HTTP base URL.
    pub fn http_url(&self) -> Option<String> {
        self.endpoints.get("http").cloned()
    }

    /// Sign a message (canonical message) with the private key.
    fn sign(&self, message: &[u8]) -> Result<String> {
        let sk_hex = self
            .private_key
            .as_ref()
            .context("no private key available — use with_new_keypair()")?;
        let sk_bytes = hex::decode(sk_hex)?;
        let sk = k256::SecretKey::from_slice(&sk_bytes)?;
        use k256::ecdsa::{SigningKey, signature::Signer};
        let signing_key = SigningKey::from(sk);
        let signature: k256::ecdsa::Signature = signing_key.sign(message);
        Ok(hex::encode(signature.to_bytes()))
    }

    /// Upload a tree JSON to the server.
    pub async fn upload_tree(&self, tree_id: &str, tree_json: &serde_json::Value) -> Result<()> {
        let base = self.http_url().context("merkle-server not started")?;
        let url = format!("{}/tree/{}", base.trim_end_matches('/'), tree_id);
        let body = serde_json::to_string(tree_json)?;

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string();

        // Build canonical message: POST /tree/<id> + timestamp + body
        let canonical = format!("POST /tree/{}\n{}\n{}", tree_id, timestamp, body);
        let sig = self.sign(canonical.as_bytes())?;

        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("X-Timestamp", &timestamp)
            .header("X-Signature", &sig)
            .body(body)
            .send()
            .await
            .context("failed to upload tree")?;

        if !resp.status().is_success() {
            let text = resp.text().await?;
            anyhow::bail!("upload failed: {text}");
        }
        Ok(())
    }

    /// Get a merkle proof for an address.
    pub async fn get_proof(&self, tree_id: &str, address: &str) -> Result<serde_json::Value> {
        let base = self.http_url().context("merkle-server not started")?;
        let url = format!(
            "{}/tree/{}/proof/{}",
            base.trim_end_matches('/'),
            tree_id,
            address
        );
        let resp = reqwest::get(&url).await?;
        let body: serde_json::Value = resp.json().await?;
        Ok(body)
    }

    /// Get the root hash for a tree.
    pub async fn get_root(&self, tree_id: &str) -> Result<String> {
        let base = self.http_url().context("merkle-server not started")?;
        let url = format!("{}/tree/{}/root", base.trim_end_matches('/'), tree_id);
        let resp = reqwest::get(&url).await?;
        let body: serde_json::Value = resp.json().await?;
        Ok(body["root"].as_str().unwrap_or("").to_string())
    }
}

#[async_trait]
impl TerpSidecar for MerkleServerSuite {
    fn id(&self) -> &SidecarId {
        &self.id
    }

    async fn start(&mut self, deps: &SidecarRegistry) -> Result<EndpointMap> {
        if self.started {
            return Ok(self.endpoints.clone());
        }
        let ep = self.inner.start(deps).await?;
        self.endpoints = ep.clone();
        self.started = true;
        Ok(ep)
    }

    async fn stop(&mut self) -> Result<()> {
        self.inner.stop().await?;
        self.started = false;
        Ok(())
    }

    async fn health(&self) -> Result<HealthStatus> {
        self.inner.health().await
    }

    fn endpoint(&self, name: &str) -> Option<String> {
        self.endpoints.get(name).cloned()
    }
}

// ── Integration Tests ─────────────────────────────────────────────────────────
//
// In-process tests that build the full server router, bind to a random port,
// and exercise every route via reqwest. No subprocess dependency.
//
// 22 tests covering:
//   - Health endpoint (GET /health — status, blossom.blobs, ve.*)
//   - Blob storage and retrieval (BUD-01/02/06)
//   - Headstash CID prefix storage specification (/f/ prefix on disk)
//   - Nostr SHA256 content-addressing (SHA256 of raw content matches server hash)
//   - CID metadata headers (X-Content-Length, X-Uploaded, X-Content-SHA256
//     on GET response — distinct from upload response descriptor fields)
//   - Merkle tree CRUD (6 tests: save/load, not-found, list, delete,
//     invalid-root validation, bad-proof-hash validation)
//   - Headstash registration CRUD (3 tests: save/get, not-found, overwrite)
//   - Auth middleware (3 tests exercising NoopAuthVerifier path for
//     blob_upload, tree_save, headstash_save)
//   - Full round-trip: upload tree → register headstash referencing tree root
//     → verify tree loads → verify headstash loads
//
// BUD-07 payment verification branches are not exercised in this configuration
// (NoopAuthVerifier skips payment checks). Add a real AuthVerifier to test
// BUD-07 enforcement.

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use hash_market::{
        client::ve::VoteExtensionHandler,
        custody::local::LocalSecp256k1,
        server::{AppState, router as build_router},
    };
    use sha2::{Digest, Sha256};

    /// Start the hash-market server in-process on a random port.
    /// Returns (base_url, data_dir, shutdown_guard).
    /// The shutdown_guard triggers graceful shutdown when dropped.
    async fn start_test_server()
    -> anyhow::Result<(String, tempfile::TempDir, tokio::sync::oneshot::Sender<()>)> {
        // Initialize tracing for diagnostics
        let _ = tracing_subscriber::fmt()
            .with_env_filter("axum=trace,hash_market=debug")
            .with_test_writer()
            .try_init();
        let data_dir = tempfile::TempDir::new()?;
        let custody = LocalSecp256k1::generate();
        let ve_handler = VoteExtensionHandler::new(Box::new(custody));
        let state = Arc::new(AppState::new(
            Some(ve_handler),
            "test-chain".into(),
            data_dir.path().to_path_buf(),
            true,
        )?);
        let app = build_router(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let base_url = format!("http://127.0.0.1:{port}");

        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    rx.await.ok();
                })
                .await
                .ok();
        });

        // Brief wait for server readiness
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        Ok((base_url, data_dir, tx))
    }

    // ── Health ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_health_endpoint() {
        let (base, _dir, _shutdown) = start_test_server().await.unwrap();
        let resp = reqwest::get(format!("{base}/health")).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["status"], "ok");
        assert!(body["blossom"]["blobs"].is_number());
        assert!(body["ve"]["active_providers"].is_number());
    }

    // ── Blob Storage and Retrieval (BUD-01/06) ────────────────────────────────

    /// Upload a blob, verify the response descriptor, then retrieve by hash
    /// and confirm exact byte matching.
    #[tokio::test]
    async fn test_blob_upload_and_retrieve() {
        let (base, _dir, _shutdown) = start_test_server().await.unwrap();
        let client = reqwest::Client::new();
        let content = b"hello blossom world from hash-market!";

        // Upload
        let upload_resp = client
            .post(format!("{base}/blobs"))
            .body(content.to_vec())
            .send()
            .await
            .unwrap();
        assert_eq!(upload_resp.status(), 200, "POST /blobs failed");
        let descriptor: serde_json::Value = upload_resp.json().await.unwrap();
        let sha256 = descriptor["sha256"].as_str().unwrap().to_string();
        assert_eq!(sha256.len(), 64);
        assert!(descriptor["size"].as_u64().unwrap() > 0);
        assert!(descriptor["url"].as_str().is_some());

        // Verify SHA256 is correct
        let expected_hash = hex::encode(Sha256::digest(content));
        assert_eq!(sha256, expected_hash, "content hash mismatch");

        // Retrieve
        let get_resp = client
            .get(format!("{base}/blobs/{sha256}"))
            .send()
            .await
            .unwrap();
        assert_eq!(get_resp.status(), 200);

        // CID metadata headers on GET response — distinct from the upload
        // response descriptor above. Exercises headstash CID prefix specification.
        let x_len = get_resp
            .headers()
            .get("X-Content-Length")
            .and_then(|v| v.to_str().ok())
            .unwrap();
        assert_eq!(x_len, content.len().to_string());
        let x_uploaded = get_resp
            .headers()
            .get("X-Uploaded")
            .and_then(|v| v.to_str().ok())
            .unwrap();
        assert!(!x_uploaded.is_empty(), "X-Uploaded should be a timestamp");

        // Verify X-Content-SHA256 matches
        let x_sha = get_resp
            .headers()
            .get("X-Content-SHA256")
            .and_then(|v| v.to_str().ok())
            .unwrap();
        assert_eq!(x_sha, sha256);
    }

    /// Confirm the /f/ prefix on-disk layout after blob upload.
    /// This exercises the headstash CID prefix storage specification.
    #[tokio::test]
    async fn test_blob_disk_layout_f_prefix() {
        let (base, data_dir, _shutdown) = start_test_server().await.unwrap();
        let client = reqwest::Client::new();
        let content = b"cid-prefix-storage-test";
        let hash = hex::encode(Sha256::digest(content));

        // Upload
        let resp = client
            .post(format!("{base}/blobs"))
            .body(content.to_vec())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        // Verify on-disk: blob should be at data_dir/trees/f/{hash}
        let f_path = data_dir.path().join("trees").join("f").join(&hash);
        assert!(
            f_path.exists(),
            "blob not found at /f/ prefix path: {f_path:?}"
        );
        let on_disk = std::fs::read(&f_path).unwrap();
        assert_eq!(&on_disk[..], content);
    }

    /// Upload via /upload alias — same handler as /blobs.
    #[tokio::test]
    async fn test_blob_upload_via_alias() {
        let (base, _dir, _shutdown) = start_test_server().await.unwrap();
        let client = reqwest::Client::new();
        let content = b"upload-alias-test";

        let resp = client
            .post(format!("{base}/upload"))
            .body(content.to_vec())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
    }

    // ── Blob List (BUD-06) ──────────────────────────────────────────────────

    /// Upload multiple blobs, list them, verify all hashes appear.
    #[tokio::test]
    async fn test_blob_list() {
        let (base, _dir, _shutdown) = start_test_server().await.unwrap();
        let client = reqwest::Client::new();
        let contents = vec![b"blob-a", b"blob-b", b"blob-c"];
        let mut expected_hashes = Vec::new();

        for content in &contents {
            let resp = client
                .post(format!("{base}/blobs"))
                .body(content.to_vec())
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 200);
            let desc: serde_json::Value = resp.json().await.unwrap();
            expected_hashes.push(desc["sha256"].as_str().unwrap().to_string());
        }

        // List
        let list_resp = client.get(format!("{base}/blobs")).send().await.unwrap();
        assert_eq!(list_resp.status(), 200);
        let list: serde_json::Value = list_resp.json().await.unwrap();
        let listed: Vec<String> = list["blobs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["sha256"].as_str().unwrap().to_string())
            .collect();

        for h in &expected_hashes {
            assert!(listed.contains(h), "hash {h} not found in blob list");
        }

        // Verify each listed entry has size + uploaded metadata
        for entry in list["blobs"].as_array().unwrap() {
            assert!(entry["size"].is_number(), "missing size metadata");
            assert!(entry["uploaded"].is_number(), "missing uploaded metadata");
        }
    }

    // ── Blob Delete (BUD-02) ────────────────────────────────────────────────

    /// Upload a blob, delete it, confirm 404 on subsequent GET.
    #[tokio::test]
    async fn test_blob_delete() {
        let (base, _dir, _shutdown) = start_test_server().await.unwrap();
        let client = reqwest::Client::new();
        let content = b"delete-me";
        let hash = hex::encode(Sha256::digest(content));

        // Upload
        client
            .post(format!("{base}/blobs"))
            .body(content.to_vec())
            .send()
            .await
            .unwrap();

        // Delete
        let del_resp = client
            .delete(format!("{base}/blobs/{hash}"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            del_resp.status(),
            204,
            "delete should return 204 No Content"
        );

        // Confirm gone
        let get_resp = client
            .get(format!("{base}/blobs/{hash}"))
            .send()
            .await
            .unwrap();
        assert_eq!(get_resp.status(), 404, "blob should be gone after delete");
    }

    /// Delete a non-existent blob returns 404.
    #[tokio::test]
    async fn test_blob_delete_not_found() {
        let (base, _dir, _shutdown) = start_test_server().await.unwrap();
        let client = reqwest::Client::new();
        let fake_hash = "0000000000000000000000000000000000000000000000000000000000000000";

        let resp = client
            .delete(format!("{base}/blobs/{fake_hash}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
    }

    // ── Nostr Content ID Compatibility ──────────────────────────────────────
    //
    // SHA256 of raw upload content must match the server-assigned blob hash.
    // This is a round-trip content integrity check: the server computes SHA256
    // on upload and returns it; client verifies local SHA256 matches.
    //
    // The content-addressed URL path is compatible with nostr event ID semantics
    // (content-addressed via SHA256), but this test only validates byte-level
    // equivalence — not nostr event serialization rules.
    #[tokio::test]
    async fn test_nostr_content_id_compatibility() {
        let (base, _dir, _shutdown) = start_test_server().await.unwrap();
        let client = reqwest::Client::new();
        // Realistic nostr-like content: a minimal JSON event serialization
        let nostr_like_content = br#"{"content":"hello nostr","created_at":1717000000,"kind":1,"tags":[],"pubkey":"abc"}"#;
        let content_hash = hex::encode(Sha256::digest(nostr_like_content));

        // Upload
        let resp = client
            .post(format!("{base}/blobs"))
            .body(nostr_like_content.to_vec())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let desc: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(
            desc["sha256"].as_str().unwrap(),
            &content_hash,
            "SHA256 from server must match SHA256 of raw content"
        );

        // Retrieve — the hash in the URL path IS the nostr-event-compatible content ID
        let get_resp = client
            .get(format!("{base}/blobs/{content_hash}"))
            .send()
            .await
            .unwrap();
        assert_eq!(get_resp.status(), 200);
        let body = get_resp.bytes().await.unwrap();
        assert_eq!(&body[..], &nostr_like_content[..]);
    }

    /// Verify parse_hash rejects both short and non-hex strings
    /// (exercises the BUD-02 parse_hash validation path).
    #[tokio::test]
    async fn test_blob_get_invalid_hash() {
        let (base, _dir, _shutdown) = start_test_server().await.unwrap();
        let client = reqwest::Client::new();

        // Too short
        let resp = client
            .get(format!("{base}/blobs/short"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);

        // Non-hex chars
        let resp = client
            .get(format!("{base}/blobs/zz{}", "00".repeat(31)))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);
    }

    // ── Merkle Tree CRUD ────────────────────────────────────────────────────

    fn valid_tree_input() -> serde_json::Value {
        serde_json::json!({
            "merkle_root": "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
            "accounts": {
                "terp1testaddressxxxxxxxxxxxxxxxxxxxxxxxxxxxxx": {
                    "tier": 1,
                    "allocation": 1000,
                    "proof_hashes": [
                        "0000000000000000000000000000000000000000000000000000000000000001"
                    ]
                }
            }
        })
    }

    #[tokio::test]
    async fn test_tree_save_and_load() {
        let (base, _dir, _shutdown) = start_test_server().await.unwrap();
        let client = reqwest::Client::new();
        let tree_id = "test-tree-001";

        // Save
        let input = valid_tree_input();
        let resp = client
            .post(format!("{base}/trees/{tree_id}"))
            .json(&input)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 201, "tree creation should return 201");

        // Load
        let get_resp = client
            .get(format!("{base}/trees/{tree_id}"))
            .send()
            .await
            .unwrap();
        assert_eq!(get_resp.status(), 200);
        let tree: serde_json::Value = get_resp.json().await.unwrap();
        assert_eq!(tree["root"], input["merkle_root"]);
        assert!(
            tree["members"]
                .as_object()
                .unwrap()
                .contains_key(&"terp1testaddressxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".to_string())
        );
        assert!(tree["created_at"].as_u64().unwrap() > 0);
        assert!(tree["updated_at"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn test_tree_get_not_found() {
        let (base, _dir, _shutdown) = start_test_server().await.unwrap();
        let resp = reqwest::get(format!("{base}/trees/nonexistent"))
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
    }

    #[tokio::test]
    async fn test_tree_list() {
        let (base, _dir, _shutdown) = start_test_server().await.unwrap();
        let client = reqwest::Client::new();
        let ids = vec!["tree-list-a", "tree-list-b"];

        for id in &ids {
            let resp = client
                .post(format!("{base}/trees/{id}"))
                .json(&valid_tree_input())
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 201);
        }

        let list_resp = client.get(format!("{base}/trees")).send().await.unwrap();
        assert_eq!(list_resp.status(), 200);
        let list: Vec<String> = list_resp.json().await.unwrap();
        for id in &ids {
            assert!(
                list.contains(&id.to_string()),
                "tree {id} not in list {list:?}"
            );
        }
    }

    #[tokio::test]
    async fn test_tree_delete() {
        let (base, _dir, _shutdown) = start_test_server().await.unwrap();
        let client = reqwest::Client::new();
        let tree_id = "tree-to-delete";

        // Save
        client
            .post(format!("{base}/trees/{tree_id}"))
            .json(&valid_tree_input())
            .send()
            .await
            .unwrap();

        // Delete
        let del_resp = client
            .delete(format!("{base}/trees/{tree_id}"))
            .send()
            .await
            .unwrap();
        assert_eq!(del_resp.status(), 204);

        // Confirm gone
        let get_resp = client
            .get(format!("{base}/trees/{tree_id}"))
            .send()
            .await
            .unwrap();
        assert_eq!(get_resp.status(), 404);
    }

    #[tokio::test]
    async fn test_tree_validate_invalid_root() {
        let (base, _dir, _shutdown) = start_test_server().await.unwrap();
        let client = reqwest::Client::new();
        let invalid = serde_json::json!({
            "merkle_root": "bad",
            "accounts": {}
        });

        let resp = client
            .post(format!("{base}/trees/bad-root"))
            .json(&invalid)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400, "invalid merkle_root should be rejected");
    }

    #[tokio::test]
    async fn test_tree_validate_bad_proof_hash() {
        let (base, _dir, _shutdown) = start_test_server().await.unwrap();
        let client = reqwest::Client::new();
        let invalid = serde_json::json!({
            "merkle_root": "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
            "accounts": {
                "terp1addr": {
                    "tier": 1,
                    "allocation": 100,
                    "proof_hashes": ["short"]
                }
            }
        });

        let resp = client
            .post(format!("{base}/trees/bad-proof"))
            .json(&invalid)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400, "invalid proof hash should be rejected");
    }

    // ── Headstash Registration CRUD ─────────────────────────────────────────

    #[tokio::test]
    async fn test_headstash_save_and_get() {
        let (base, _dir, _shutdown) = start_test_server().await.unwrap();
        let client = reqwest::Client::new();
        let hs_id = "test-headstash-001";
        let data = serde_json::json!({
            "root": "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
            "total_addresses": 100,
            "tier_summary": { "1": 50, "2": 50 }
        });

        let resp = client
            .post(format!("{base}/headstash/{hs_id}"))
            .json(&data)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 201);

        let get_resp = client
            .get(format!("{base}/headstash/{hs_id}"))
            .send()
            .await
            .unwrap();
        assert_eq!(get_resp.status(), 200);
        let retrieved: serde_json::Value = get_resp.json().await.unwrap();
        assert_eq!(retrieved["root"], data["root"]);
        assert_eq!(retrieved["total_addresses"], 100);
    }

    #[tokio::test]
    async fn test_headstash_get_not_found() {
        let (base, _dir, _shutdown) = start_test_server().await.unwrap();
        let resp = reqwest::get(format!("{base}/headstash/nonexistent"))
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
    }

    #[tokio::test]
    async fn test_headstash_overwrite() {
        let (base, _dir, _shutdown) = start_test_server().await.unwrap();
        let client = reqwest::Client::new();
        let hs_id = "hs-overwrite";

        // Write once
        client
            .post(format!("{base}/headstash/{hs_id}"))
            .json(&serde_json::json!({"version": 1}))
            .send()
            .await
            .unwrap();

        // Overwrite
        let resp = client
            .post(format!("{base}/headstash/{hs_id}"))
            .json(&serde_json::json!({"version": 2, "note": "overwritten"}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 201);

        let get_resp = client
            .get(format!("{base}/headstash/{hs_id}"))
            .send()
            .await
            .unwrap();
        let retrieved: serde_json::Value = get_resp.json().await.unwrap();
        assert_eq!(retrieved["version"], 2, "should return overwritten data");
    }

    // ── Auth Middleware Exercise ─────────────────────────────────────────────
    //
    // These tests confirm the auth codepath is exercised for each protected route.
    // With the default NoopAuthVerifier, all requests pass through — this confirms
    // that the auth middleware does not crash or reject valid requests.
    //
    // To test real auth enforcement (rejection of invalid tokens), create a
    // BlossomState with a non-trivial AuthVerifier and pass it to AppState.

    #[tokio::test]
    async fn test_auth_blob_upload_path_ok() {
        let (base, _dir, _shutdown) = start_test_server().await.unwrap();
        let client = reqwest::Client::new();
        // The NoopAuthVerifier accepts any request — confirm the code path runs
        let resp = client
            .post(format!("{base}/blobs"))
            .body("auth-exercise")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200, "auth should pass with NoopAuthVerifier");
    }

    #[tokio::test]
    async fn test_auth_tree_save_path_ok() {
        let (base, _dir, _shutdown) = start_test_server().await.unwrap();
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{base}/trees/auth-test"))
            .json(&valid_tree_input())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 201, "auth should pass for tree_save");
    }

    #[tokio::test]
    async fn test_auth_headstash_save_path_ok() {
        let (base, _dir, _shutdown) = start_test_server().await.unwrap();
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{base}/headstash/auth-hs"))
            .json(&serde_json::json!({"test": true}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 201, "auth should pass for headstash_save");
    }

    // ── Full Round-Trip: Headstash + Blob CID Consistency ──────────────────
    //
    // 4-step real workflow:
    //   1. Upload merkle tree via POST /trees/{id}
    //   2. Register headstash referencing the tree root via POST /headstash/{id}
    //   3. Retrieve and verify the merkle tree (GET /trees/{id})
    //   4. Retrieve and verify the headstash (GET /headstash/{id})

    #[tokio::test]
    async fn test_full_round_trip_headstash_with_tree() {
        let (base, _dir, _shutdown) = start_test_server().await.unwrap();
        let client = reqwest::Client::new();
        let hs_id = "integration-roundtrip";
        let tree_id = "integration-tree";

        // 1. Upload merkle tree
        let tree_input = valid_tree_input();
        let tree_resp = client
            .post(format!("{base}/trees/{tree_id}"))
            .json(&tree_input)
            .send()
            .await
            .unwrap();
        assert_eq!(tree_resp.status(), 201);

        // 2. Register headstash referencing the tree root
        let hs_data = serde_json::json!({
            "root": tree_input["merkle_root"],
            "tree_id": tree_id,
            "total_addresses": 1
        });
        let hs_resp = client
            .post(format!("{base}/headstash/{hs_id}"))
            .json(&hs_data)
            .send()
            .await
            .unwrap();
        assert_eq!(hs_resp.status(), 201);

        // 3. Verify tree loads
        let tree_get = client
            .get(format!("{base}/trees/{tree_id}"))
            .send()
            .await
            .unwrap();
        assert_eq!(tree_get.status(), 200);
        let tree: serde_json::Value = tree_get.json().await.unwrap();
        assert_eq!(tree["root"], tree_input["merkle_root"]);

        // 4. Verify headstash loads
        let hs_get = client
            .get(format!("{base}/headstash/{hs_id}"))
            .send()
            .await
            .unwrap();
        assert_eq!(hs_get.status(), 200);
        let hs: serde_json::Value = hs_get.json().await.unwrap();
        assert_eq!(hs["root"], tree_input["merkle_root"]);
        assert_eq!(hs["tree_id"], tree_id);
    }
}
