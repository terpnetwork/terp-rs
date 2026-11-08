# Terp Sidecar Suite — Runtime Design

## Core Insight

Sidecars have THREE runtime modes, not one. A single abstraction must handle all:

1. **Docker container** — full process isolation (relayer, minio-ipfs, Postgres)
2. **Subprocess** — local binary spawned from the suite (hash-market-server, merkle-server)
3. **In-process tokio task** — lightweight embedded server (mock sidecar, nostr manager's health endpoint)

The existing `SidecarProcess` in ict-rs covers (1). The `NostrRelayerManager` in `ict-rs/src/nostr/relayer_manager.rs` partially covers (2) with its `RuntimeBackend` abstraction. The `start_mock_sidecar()` function in `loyalty_rewards.rs` covers (3). None of them are unified.

## Component Registry (from codebase survey)

| Sidecar | Runtime Mode | Config Source | Health Check | Ports | Dependencies |
|---------|-------------|---------------|-------------|-------|-------------|
| hash-market-server | Subprocess | config.toml (bind, chain_id, signing_key, [[providers]]) | GET /health | HTTP:9090, gRPC:9091 | Terp chain running |
| hash-market-client | Subprocess | client.toml (eth_rpc, sidecar_url) | None | None (outbound only) | hash-market-server |
| merkle-server | Subprocess | config.json (public_key, data_dir, host, port) | GET /health | HTTP:8765 | None |
| argus-indexer | Docker or subprocess | config.json (rpc, bech32, db, redis, meili) | Server health | API:3420, Debug:9227 | Postgres, Redis, MeiliSearch |
| relayer (Hermes/CosmosRly) | Docker container | TOML per chain config | Via docker | depends | Both chains funded |
| minio-ipfs | Docker container | env vars (root_user/pass, webhook, buckets) | /minio/health/live | 9000,9002,8081,9100,9443 | None |
| nostr-relay | Docker container | None (default mattn image) | None (WS immediate) | 7777 | None |

Argus is uniquely complex: it's a Node.js app with 7+ subprocesses (tracer, listener, workers, workers-bg, account-webhooks, server_accounts, server) plus 3 infrastructure containers (Postgres x2, Redis, MeiliSearch). A single "Argus indexer" suite element must manage an entire microservice fleet.

## Trait Design

```
┌─────────────────────────────────────────────┐
│             TerpSidecar<T: Runtime>         │
│  Generic over runtime strategy              │
├─────────────────────────────────────────────┤
│ + name() -> &str                            │
│ + start(chain, ctx) -> Result<EndpointMap>  │
│ + stop() -> Result<()>                      │
│ + health() -> Result<HealthStatus>          │
│ + endpoint(name: &str) -> Option<String>    │
└─────────────────────────────────────────────┘
                      ∧
                      │ implements
        ┌─────────────┼─────────────┐
        │             │             │
   DockerSidecar  SubprocessSidecar  InProcessSidecar
   (wraps          (spawns binary,  (tokio::spawn
    SidecarProcess) manages stdin)   task handle)
```

## Concrete Suite Types

```rust
/// The top-level TerpArtifactStack — typed access to every sidecar.
pub struct TerpArtifactStack<Chain: CwEnv> {
    // Chain environment (inherits Environment<Daemon>)
    daemon: Chain,

    // Core sidecars
    pub hashmerchant: HashMerchantSuite,
    pub indexer: IndexerSuite,
    pub relayer: RelayerSuite,
    pub merkle_server: MerkleServerSuite,
    pub minio_ipfs: MinioIpfsSuite,
    pub nostr_relay: NostrRelaySuite,

    // Internal runtime for subprocess/Docker management
    runtime: Arc<dyn RuntimeBackend>,
    network_id: String,
    test_name: String,
}

impl<Chain: CwEnv> Environment<Chain> for TerpArtifactStack<Chain> {
    fn environment(&self) -> &Chain { &self.daemon }
}
```

Each suite element implements `TerpSidecar` and surfaces its own typed API:

### HashMerchantSuite

```rust
pub struct HashMerchantSuite {
    server: SubprocessSidecar<MerchantSuiteConfig>,
    client: SubprocessSidecar<MerchantClientSuiteConfig>,
}

/// Uses canonical `hash_market::config::Config` wrapped with `binary_path`.
/// See `tools/hash-market/src/config.rs` for the canonical struct.

pub struct MerchantSuiteConfig {
    pub inner: Config,              // hash_market::config::Config
    pub binary_path: Option<PathBuf>,
}

pub struct MerchantClientSuiteConfig {
    pub inner: ClientConfig,        // hash_market::config::ClientConfig
    pub binary_path: Option<PathBuf>,
}

/// Wrapper over ProviderConfig for BUD compatibility
/// See `tools/hash-market/src/config.rs`

pub struct ProviderConfig {
    pub name: String,
    pub chain_uid: String,
    pub algo: String,              // "keccak256" | "sha256"
    pub mode: String,              // "grpc" | "http_poll" | "websocket"
    pub address: String,
    pub interval_secs: u64,
}

impl HashMerchantSuite {
    /// Build from the example config pattern in hashmerchant.rs
    pub fn with_defaults(chain_id: &str) -> Self;

    /// Feed a single provider's data via gRPC transport
    pub async fn feed_provider(&self, provider: &str, root: &[u8], height: u64) -> Result<()>;

    /// Query the /vote-extension endpoint directly
    pub async fn vote_extension(&self, chain_uid: &str) -> Result<VoteExtensionData>;

    /// Get the /health endpoint URL
    pub fn server_http_url(&self) -> String;
}
```

Key reference: `tools/hash-market/src/bin/server.rs` (Config struct), `examples/hashmerchant.rs` (config.toml generation pattern with bind, chain_id, signing_key, [[providers]])

### MerkleServerSuite

```rust
pub struct MerkleServerSuite {
    inner: SubprocessSidecar<MerkleServerConfig>,
}

pub struct MerkleServerConfig {
    pub public_key: String,       // 66-char hex compressed secp256k1
    pub host: String,             // "127.0.0.1"
    pub port: u16,                // 8765
    pub data_dir: PathBuf,        // "./data"
    pub timestamp_tolerance_s: u64, // 300
}

impl MerkleServerSuite {
    pub fn with_default_keypair() -> Self;

    /// Build a tree from addresses and upload
    pub async fn build_and_upload(
        &self, tree_id: &str, entries: &[(String, u32)]
    ) -> Result<String>;

    /// Query a proof for an address
    pub async fn get_proof(&self, tree_id: &str, address: &str) -> Result<MerkleProof>;

    /// Query the root for a tree
    pub async fn get_root(&self, tree_id: &str) -> Result<String>;
}
```

Key reference: `tools/merkle-server/src/main.rs` (Config struct, REST API endpoints)

### IndexerSuite

```rust
pub struct IndexerSuite {
    // Argus is complex: it's a Node.js fleet + infrastructure
    server: DockerSidecar,           // The main server container
    listener: DockerSidecar,         // The event listener
    workers: DockerSidecar,          // Background workers
    // Infrastructure managed as companion sidecars
    postgres_data: DockerSidecar,    // TimescaleDB
    postgres_accounts: DockerSidecar, // Postgres
    redis: DockerSidecar,            // Redis
    meilisearch: DockerSidecar,      // MeiliSearch
}

pub struct IndexerConfig {
    pub local_rpc: String,
    pub bech32_prefix: String,
    pub db_password: String,
    pub meili_master_key: String,
}

impl IndexerSuite {
    /// Create from compose.dev.yml pattern — all 7 containers
    pub fn from_compose(chain_rpc: &str, bech32: &str) -> Self;

    /// Health check: all containers healthy and API responding
    pub async fn wait_ready(&self) -> Result<()>;

    /// The API URL for query chain data
    pub fn api_url(&self) -> String;

    /// Start only a subset (for quick tests that don't need full indexing)
    pub fn with_minimal() -> Self;  // just server + postgres_data
}
```

Key reference: `tools/argus/compose.dev.yml` (7 services), `tools/argus/src/types/config.ts` (Config type)

### RelayerSuite

```rust
pub struct RelayerSuite {
    inner: DockerSidecar, // wraps DockerRelayer
}

pub struct RelayerConfig {
    pub relayer_type: RelayerType,  // Hermes | CosmosRly
    pub chain_a: ChainEndpoint,
    pub chain_b: ChainEndpoint,
    pub mnemonic: String,
}

impl RelayerSuite {
    pub fn new_hermes() -> Self;
    pub fn new_cosmos_rly() -> Self;

    /// Create IBC path + start relaying
    pub async fn link_and_start(
        &self, src: &str, dst: &str, channel: &ChannelOptions
    ) -> Result<()>;
}
```

Key reference: `ict-rs/src/relayer/` (DockerRelayer, HermesRelayer, CosmosRlyCommander)

### MinioIpfsSuite

```rust
pub struct MinioIpfsSuite {
    inner: DockerSidecar,
}

pub struct MinioIpfsConfig {
    pub root_user: String,
    pub root_password: String,
    pub webhook_auth_token: String,
    pub autopin_buckets: String,
}

impl MinioIpfsSuite {
    pub fn with_default_creds() -> Self;

    /// S3 endpoint URL
    pub fn s3_url(&self) -> String;

    /// IPFS gateway URL
    pub fn ipfs_gateway_url(&self) -> String;

    /// Bootstrap from a remote MinIO instance
    pub async fn bootstrap_remote(
        &self, mode: BootstrapMode, remote: &RemoteConfig
    ) -> Result<()>;
}
```

Key reference: `ict-rs/src/cosmos/docker_sidecar.rs` (minio_ipfs_config), `plays/instant-replay/`

### NostrRelaySuite

```rust
pub struct NostrRelaySuite {
    inner: DockerSidecar,
}

impl NostrRelaySuite {
    pub fn new() -> Self;

    pub fn ws_url(&self) -> String;
    pub async fn connect_client(&self) -> Result<NostrClient>;
}
```

Already exists as `NostrRelayerManager` — wrap in the unified interface.

## Generic Subprocess Sidecar

The key new abstraction. A subprocess sidecar manages a local binary:

```rust
pub struct SubprocessSidecar<C: SubprocessConfig> {
    config: C,
    child: Option<tokio::process::Child>,
    config_file: Option<tempfile::TempDir>,
    health_check_url: Option<String>,
}

pub trait SubprocessConfig: Clone + Send + Sync + 'static {
    /// The binary to run (resolved via cargo manifest or PATH)
    fn binary_path(&self) -> Result<PathBuf>;

    /// Args passed to the binary
    fn args(&self) -> Vec<String>;

    /// Env vars to set
    fn env(&self) -> Vec<(String, String)>;

    /// Write config to disk at `config_dir` and return the path
    fn write_config(&self, config_dir: &Path) -> Result<PathBuf>;

    /// Optional health check endpoint (method, url)
    fn health_check(&self) -> Option<(String, String)>;
}
```

For `MerchantSuiteConfig` (wraps canonical `hash_market::config::Config`):
- binary: `find_server_binary()` (cargo build pattern from hashmerchant.rs:221-236)
- args: `["-c", config_path]`
- config: writes TOML matching `Config` struct in `server.rs`
- health: `("GET", "/health")` on bind address

For `MerkleServerConfig`:
- binary: `merkle-server` (compiled from tools/merkle-server/)
- args: `["serve", "--config", config_path]`
- config: writes JSON matching `Config` struct in `main.rs`
- health: `("GET", "/health")`

## Integration with ict-rs ChainConfig

The existing `ChainConfig.sidecar_configs: Vec<SidecarConfig>` only handles Docker containers. The suite extends this at a higher level:

```rust
impl TerpArtifactStack {
    /// Build suite from an existing ict-rs chain
    pub async fn attach_to_chain(
        chain: &CosmosChain,
        runtime: Arc<dyn RuntimeBackend>,
        mnemonic: &str,
    ) -> Result<Self> {
        // Extract host endpoints from chain
        // Initialize each sidecar according to its runtime mode
        // Return fully configured suite
    }
}
```

This bridges the gap: `TerpArtifactStack` owns the sidecar lifecycle but delegates block production and chain-state to the existing `CosmosChain` + `Daemon`.

## Defaults Principle

Each suite element has a `with_defaults()` constructor that produces a working configuration using sensible defaults:

- **HashMerchant**: ephemeral signing key, test chain ID, single keccak256 provider
- **MerkleServer**: fresh keypair auto-generated, port 8765, /tmp data dir
- **Indexer**: "terp" bech32, "dev" db credentials, local RPC:26657
- **Relayer**: Hermes type, auto-fund from deployer wallet
- **MinioIpfs**: "minioadmin/minioadmin" creds, single "test" bucket

## File Layout in the tests crate

```
crates/terp-rs/tests/src/
├── suite/
│   ├── mod.rs                    — TerpArtifactStack struct definition
│   ├── sidecar.rs               — TerpSidecar trait + SubprocessSidecar generic
│   ├── hashmerchant.rs          — HashMerchantSuite + MerchantSuiteConfig
│   ├── merkle.rs                — MerkleServerSuite + MerkleServerConfig
│   ├── indexer.rs               — IndexerSuite + IndexerConfig
│   ├── relayer.rs               — RelayerSuite + RelayerConfig
│   ├── minio_ipfs.rs            — MinioIpfsSuite + MinioIpfsConfig
│   └── nostr.rs                 — NostrRelaySuite (wraps NostrRelayerManager)
├── nostr_env.rs                  — Existing (may wrap into suite)
├── quickspawn_env.rs             — Existing
└── suite.rs                      — Existing (skeleton, now extended)
```

## Implementation Order

1. **Foundation**: `TerpSidecar<T: Runtime>` trait + `SubprocessSidecar<C>` generic in `suite/sidecar.rs`
   - This is the hardest part — get the generic right for all 3 runtime modes
   - Must handle: binary resolution, config file generation, health polling, graceful shutdown, signal forwarding

2. **HashMerchantSuite** — simplest concrete impl (single binary, single config file, well-known health check)
   - Validates the SubprocessSidecar abstraction
   - Reuses the `find_server_binary()` and config.toml pattern from `hashmerchant.rs`

3. **MerkleServerSuite** — validates the generic handles a different binary/config format (JSON not TOML)

4. **RelayerSuite** — wraps existing `DockerRelayer` into the unified trait, extends with `link_and_start()`

5. **NostrRelaySuite** — wraps existing `NostrRelayerManager` — proves Docker path works through the trait

6. **MinioIpfsSuite** — wraps existing `minio_ipfs_config()` SidecarConfig constructor

7. **IndexerSuite** — most complex, validates the suite handles multi-container sidecaps
   - Argus is a fleet, not a single container

8. **Integration**: `TerpArtifactStack::attach_to_chain()` + `Environment<Daemon>` impl
   - Test with a full e2e: spawn Terp chain → attach suite → deploy hashmerchant → verify VE

## Key Design Decisions

- **`SubprocessSidecar<C: SubprocessConfig>` is NOT a trait object** — each config type is concrete. The suite struct has typed fields so you call `suite.hashmerchant.vote_extension()` directly, not through a `dyn Any` downcast.

- **Config files written to temp dirs** — `tempfile::tempdir()` created per sidecar, destroyed on `stop()`. No filesystem pollution.

- **Binary resolution via cargo manifest** — use the `find_server_binary()` pattern from `hashmerchant.rs:221-236` as a resolver trait method with a default implementation that checks `CARGO_MANIFEST_DIR` relative paths + `PATH`.

- **Health polling is OPTIONAL** — `SubprocessConfig::health_check()` returns `None` for sidecars without health endpoints (nostr relay). The suite start waits for `Some` endpoints to pass, or sleeps a default timeout.

- **Argus complexity handled via `with_minimal()`** — the IndexerSuite knows its fleet. `with_minimal()` starts only the API server + data backend. `from_compose()` starts everything. Tests choose their fidelity level.

- **Drop impl** — every suite struct implements `Drop` to kill subprocesses / stop containers. Uses `tokio::runtime::Handle::try_current()` pattern from SpawnedChainSet (avoids panic if no tokio runtime in drop context).