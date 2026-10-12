//! Canonical config types for hash-market binaries.
//!
//! # Modes
//!
//! **Mint / whitelist host (default):** `ve_enabled = false` — serves BUD blobs +
//! Merkle trees for NFT whitelist proofs. No vote-extension providers required.
//!
//! **Validator sidecar:** `ve_enabled = true` (+ compile with `--features ve`) —
//! ABCI++ ExtendVote feeders + signing.

use serde::{Deserialize, Serialize};

// ── Server config ───────────────────────────────────────────────────────────

/// TOML config for `hash-market-server`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// HTTP bind address. Default `127.0.0.1:9090`.
    /// A non-loopback host (including `0.0.0.0`) refuses to start unless
    /// `allow_public_bind` or `HASH_MARKET_ALLOW_PUBLIC_BIND=1`.
    #[serde(default = "default_bind")]
    pub bind: String,
    /// Explicit override for a non-loopback `bind`. Logged at startup.
    #[serde(default)]
    pub allow_public_bind: bool,
    /// Sole `terp1` allowed to POST/PUT/DELETE `/trees`.
    /// `HASH_MARKET_TREE_OPERATOR` overrides this.
    #[serde(default = "default_tree_operator")]
    pub tree_operator: String,
    /// CometBFT / app chain ID (metadata; used by VE when enabled)
    #[serde(default = "default_chain_id")]
    pub chain_id: String,
    /// Hex-encoded secp256k1 signing key.
    /// Required only when `ve_enabled = true`. `/trees` writes use `tree_operator`
    /// plus an ADR-36 request signature, not this key.
    #[serde(default)]
    pub signing_key: Option<String>,
    /// Optional data directory for tree/blob/headstash storage (default: "data")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<String>,
    /// Enable ABCI++ vote-extension routes and provider feeders.
    /// Default **false** — mint frontends only need `/trees/*` + `/blobs/*`.
    #[serde(default)]
    pub ve_enabled: bool,
    /// Provider definitions (ignored when `ve_enabled = false`)
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    /// Optional multi-source price-bound aggregation (Connect-style).
    /// Ignored unless at least one provider has `kind = "price_bound"`.
    #[serde(default)]
    pub oracle: Option<OracleConfig>,
    /// Optional CORS allow-origin for browser mint pages (e.g. "*")
    #[serde(default)]
    pub cors_allow_origin: Option<String>,
    /// Optional oline/IPFS + BUD dual-index distribution (default off).
    #[serde(default)]
    pub distribution: Option<crate::content::DistributionConfig>,
    /// Auth for blossom + private `/notes/*` (default noop for local/dev).
    #[serde(default)]
    pub notes_auth: Option<NotesAuthConfig>,
    /// Cashu off-chain mesh: mint discovery cache + encrypted wallet stash (`/cashu/*`).
    /// See `docs/plans/cashu/CANONICAL-MINT-REGISTRY.md` (canonical registry, not “official”).
    #[serde(default)]
    pub cashu_mesh: Option<CashuMeshConfig>,
    /// Embed onboard-authz under `/onboard/*` (no second process).
    #[serde(default)]
    pub onboard: Option<OnboardConfig>,
    /// FROST + Noise privval sidecar (same process as hash-market).
    #[serde(default)]
    pub privval: Option<PrivvalSidecarConfig>,
    /// Opt-in public-leaf PIR ingest. Honored only with `--features pir`.
    /// Does not replace blossom/NIP-BUD or Nostr and does not broadcast.
    #[serde(default)]
    pub pir: Option<PirIngestConfig>,
}

/// `[pir]` — genesis `notes.json` → public leaf blobs + merkle paths.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PirIngestConfig {
    /// When the section is present, ingest on startup (default true).
    #[serde(default = "default_pir_enabled")]
    pub enabled: bool,
    /// Headstash id used in `/notes/{hs_id}/pir` (default `community-notes`).
    #[serde(default = "default_pir_hs_id")]
    pub hs_id: String,
    /// Path to `crates/headstash/artifacts/community-notes/notes.json`.
    #[serde(default)]
    pub notes_json: String,
    /// Path to `merkle_output.json` (poseidon-v1 root).
    #[serde(default)]
    pub merkle_json: String,
}

fn default_pir_enabled() -> bool {
    true
}

fn default_pir_hs_id() -> String {
    "community-notes".into()
}

/// `[privval]` — coordinator + session crypto in this runtime, not a second kms ELF.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrivvalSidecarConfig {
    #[serde(default)]
    pub enabled: bool,
    pub addr: String,
    /// `noise` (default) or `cleartext` (lab)
    #[serde(default = "default_privval_crypto")]
    pub crypto: String,
    #[serde(default)]
    pub frost_t: u16,
    #[serde(default)]
    pub frost_n: u16,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub shares: Vec<String>,
    /// `json` (e2e) or `proto` (sign-bytes protobuf)
    #[serde(default = "default_privval_codec")]
    pub codec: String,
    #[serde(default)]
    pub state_file: Option<String>,
}

fn default_privval_codec() -> String {
    "json".into()
}

fn default_privval_crypto() -> String {
    "noise".into()
}

/// `[onboard]` — DAO → runtime key → scoped feegrant. Served at `/onboard/*`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OnboardConfig {
    /// Mount grant routes (default **true** when the section is present).
    #[serde(default = "default_onboard_enabled")]
    pub enabled: bool,
    /// DAO granter (`terp1…`). Env `ONBOARD_DAO` overrides.
    #[serde(default)]
    pub dao: Option<String>,
    /// Runtime key that holds authz (`terp1…`). Env `ONBOARD_RUNTIME` overrides.
    #[serde(default)]
    pub runtime_key: Option<String>,
    /// Override chain (else `Config.chain_id`). Env `ONBOARD_CHAIN` overrides.
    #[serde(default)]
    pub chain_id: Option<String>,
    #[serde(default)]
    pub register_limit: Option<String>,
    #[serde(default)]
    pub mint_limit: Option<String>,
    /// Never enable in the merchant image without `terpd` + TEST keyring.
    #[serde(default)]
    pub broadcast: bool,
}

fn default_onboard_enabled() -> bool {
    true
}

impl Default for OnboardConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            dao: None,
            runtime_key: None,
            chain_id: None,
            register_limit: None,
            mint_limit: None,
            broadcast: false,
        }
    }
}

/// Cashu mesh host settings (`[cashu_mesh]` in TOML).
///
/// Store always lives under `{data_dir}/cashu/…`. Routes are registered when
/// `enabled` (default **true** when the section is present; default config
/// without section also enables mesh routes for lab — set `enabled = false` to hide).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CashuMeshConfig {
    /// Mount `/cashu/mints/*` and `/cashu/wallets/*` (default true).
    #[serde(default = "default_cashu_mesh_enabled")]
    pub enabled: bool,
    /// If true, `GET /cashu/mints` and `GET /cashu/mints/{id}` skip auth (discovery).
    /// Writes and all wallet routes always require notes_auth. Default **true**.
    #[serde(default = "default_cashu_public_mint_list")]
    pub public_mint_list: bool,
    /// Optional on-chain canonical registry contract address (indexer later).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_contract: Option<String>,
    /// Optional chain RPC for registry poll (indexer later).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_rpc: Option<String>,
}

fn default_cashu_mesh_enabled() -> bool {
    true
}
fn default_cashu_public_mint_list() -> bool {
    true
}

impl Default for CashuMeshConfig {
    fn default() -> Self {
        Self {
            enabled: default_cashu_mesh_enabled(),
            public_mint_list: default_cashu_public_mint_list(),
            registry_contract: None,
            chain_rpc: None,
        }
    }
}

/// Auth settings for private notes / blossom (`[notes_auth]` in TOML).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotesAuthConfig {
    /// `noop` | `bearer` | `secp` | `bearer_or_secp`
    #[serde(default = "default_notes_auth_mode")]
    pub mode: String,
    /// Shared bearer token (or env `NOTES_BEARER_TOKEN` / `JWT_SECRET`).
    #[serde(default)]
    pub bearer_token: Option<String>,
    /// Allowed compressed secp pubkeys (hex).
    #[serde(default)]
    pub allowed_pubkeys: Vec<String>,
    /// When true and allowed_pubkeys empty, accept any valid secp signature.
    #[serde(default)]
    pub allow_any_secp: bool,
    #[serde(default = "default_notes_auth_tolerance")]
    pub timestamp_tolerance_secs: u64,
}

fn default_notes_auth_mode() -> String {
    "noop".into()
}
fn default_notes_auth_tolerance() -> u64 {
    300
}

impl Default for NotesAuthConfig {
    fn default() -> Self {
        Self {
            mode: default_notes_auth_mode(),
            bearer_token: None,
            allowed_pubkeys: Vec::new(),
            allow_any_secp: false,
            timestamp_tolerance_secs: default_notes_auth_tolerance(),
        }
    }
}

/// Elevated sophistication: multi-source mid aggregation for **bounds only**.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct OracleConfig {
    /// Enable bounds aggregation path (default false — state_root VE only).
    #[serde(default)]
    pub bounds_enabled: bool,
    /// median | mean | weighted_mean | trimmed_mean | min | max
    #[serde(default = "default_agg_method")]
    pub aggregation: String,
    #[serde(default = "default_min_sources")]
    pub min_sources: usize,
    #[serde(default = "default_max_age_secs")]
    pub max_age_secs: u64,
}

fn default_agg_method() -> String {
    "median".into()
}
fn default_min_sources() -> usize {
    1
}
fn default_max_age_secs() -> u64 {
    120
}

impl OracleConfig {
    pub fn to_policy(&self) -> crate::oracle::AggregationPolicy {
        use crate::oracle::{AggregationMethod, AggregationPolicy};
        let method = match self.aggregation.to_lowercase().as_str() {
            "mean" => AggregationMethod::Mean,
            "weighted_mean" | "weighted" => AggregationMethod::WeightedMean,
            "trimmed_mean" | "trimmed" => AggregationMethod::TrimmedMean,
            "min" => AggregationMethod::Min,
            "max" => AggregationMethod::Max,
            _ => AggregationMethod::Median,
        };
        AggregationPolicy {
            method,
            min_sources: self.min_sources.max(1),
            max_age_secs: self.max_age_secs,
        }
    }
}

fn default_chain_id() -> String {
    "morocco-1".into()
}

fn default_bind() -> String {
    "127.0.0.1:9090".into()
}

/// Deployed whitelist creator. Public address, not a key.
pub const DEFAULT_TREE_OPERATOR: &str = "terp1fxccvvhhy43tvet2ah7jqwq4cwl9k3dxmww052";

fn default_tree_operator() -> String {
    DEFAULT_TREE_OPERATOR.into()
}

impl Config {
    /// True when vote extensions should run (runtime flag; also needs `ve` feature at compile).
    pub fn wants_ve(&self) -> bool {
        self.ve_enabled
    }

    /// Cashu mesh routes enabled (default true when unset — lab-friendly; set false to hide).
    pub fn wants_cashu_mesh(&self) -> bool {
        self.cashu_mesh.as_ref().map(|c| c.enabled).unwrap_or(true)
    }

    /// Public unauthenticated mint discovery GETs (`GET /cashu/mints*`).
    pub fn cashu_public_mint_list(&self) -> bool {
        self.cashu_mesh
            .as_ref()
            .map(|c| c.public_mint_list)
            .unwrap_or(true)
    }

    /// oline/IPFS + BUD dual-index distribution plane.
    pub fn wants_distribution(&self) -> bool {
        self.distribution
            .as_ref()
            .map(|d| d.enabled)
            .unwrap_or(false)
    }

    /// Public base URL advertised in content.available bud urls (no trailing slash).
    pub fn public_bud_base(&self) -> String {
        let bind = self.bind.replace("0.0.0.0", "127.0.0.1");
        if bind.starts_with("http") {
            bind.trim_end_matches('/').to_string()
        } else {
            format!("http://{bind}")
        }
    }

    /// Connect-style multi-source price bounds (opt-in elevated path).
    pub fn wants_oracle_bounds(&self) -> bool {
        self.oracle
            .as_ref()
            .map(|o| o.bounds_enabled)
            .unwrap_or(false)
    }

    /// Price-bound provider entries (`kind = "price_bound"`).
    pub fn price_bound_providers(&self) -> impl Iterator<Item = &ProviderConfig> {
        self.providers
            .iter()
            .filter(|p| p.provider_kind() == crate::oracle::ProviderKind::PriceBound)
    }

    /// State-root VE provider entries (legacy default).
    pub fn state_root_providers(&self) -> impl Iterator<Item = &ProviderConfig> {
        self.providers
            .iter()
            .filter(|p| p.provider_kind() == crate::oracle::ProviderKind::StateRoot)
    }

    /// Validate config for the selected mode.
    pub fn wants_onboard(&self) -> bool {
        self.onboard.as_ref().map(|o| o.enabled).unwrap_or(false)
    }

    /// Config asks to ingest public-leaf PIR. The route still needs `--features pir`.
    pub fn wants_pir_ingest(&self) -> bool {
        self.pir.as_ref().map(|p| p.enabled).unwrap_or(false)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.ve_enabled {
            if self
                .signing_key
                .as_ref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
            {
                return Err(
                    "signing_key is required when ve_enabled = true (validator sidecar mode)"
                        .into(),
                );
            }
            if self.providers.is_empty() {
                return Err(
                    "at least one [[providers]] entry is required when ve_enabled = true".into(),
                );
            }
        }
        if self.wants_pir_ingest() {
            let p = self.pir.as_ref().expect("wants_pir_ingest");
            if p.hs_id.trim().is_empty() {
                return Err("[pir] hs_id must not be empty when enabled".into());
            }
            if p.notes_json.trim().is_empty() || p.merkle_json.trim().is_empty() {
                return Err(
                    "[pir] notes_json and merkle_json are required when enabled (no broadcast)"
                        .into(),
                );
            }
        }
        if self.wants_oracle_bounds() {
            let n = self.price_bound_providers().count();
            if n == 0 {
                return Err(
                    "oracle.bounds_enabled=true requires at least one [[providers]] with kind = \"price_bound\""
                        .into(),
                );
            }
            for p in self.price_bound_providers() {
                if p.market_id
                    .as_ref()
                    .map(|m| m.trim().is_empty())
                    .unwrap_or(true)
                {
                    return Err(format!(
                        "price_bound provider '{}' requires market_id (e.g. \"ETH/USD\")",
                        p.name
                    ));
                }
            }
        }
        Ok(())
    }
}

/// A single provider definition in the server TOML config.
///
/// Default **kind** is state-root VE (legacy). Optional multi-source **price
/// bounds** use `kind = "price_bound"` + `market_id` (Tacit: bounds only, never mint).
/// See `docs/oracle-connect-bounds.md` and Skip Connect provider patterns.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Human-readable name
    pub name: String,
    /// Chain UID (e.g. "ethereum-mainnet", "terp-test-1")
    /// For price_bound feeders this may be a synthetic uid (e.g. "price-feeds").
    #[serde(default)]
    pub chain_uid: String,
    /// Hash algorithm (default: "keccak256"); price bounds may use `price_median_v1`
    #[serde(default = "default_algo")]
    pub algo: String,
    /// Transport mode: "grpc", "http_poll", "websocket", or **"static"** (lab/e2e only)
    pub mode: String,
    /// Transport address (gRPC listen, HTTP URL, or WS URL).  
    /// For `mode = "static"`, use `"lab"` or leave as empty with serde default.
    #[serde(default)]
    pub address: String,
    /// Polling interval in seconds (default: 12); static lab re-emit interval
    #[serde(default = "default_interval")]
    pub interval_secs: u64,
    /// `state_root` (default) | `price_bound` — omit for legacy root feeders
    #[serde(default)]
    pub kind: Option<String>,
    /// Required for price_bound (e.g. "ETH/USD")
    #[serde(default)]
    pub market_id: Option<String>,
    /// Optional weight for weighted aggregation (default 1.0)
    #[serde(default)]
    pub weight: Option<f64>,
    /// Lab static mode: hex root (no 0x), 32+ hex chars recommended
    #[serde(default)]
    pub static_root: Option<String>,
    /// Lab static mode: foreign height
    #[serde(default)]
    pub static_height: Option<u64>,
    /// Lab static mode: foreign block time (unix); default = now if unset
    #[serde(default)]
    pub static_block_time: Option<i64>,
}

impl ProviderConfig {
    /// Resolved provider kind (default state_root when unset/empty).
    pub fn provider_kind(&self) -> crate::oracle::ProviderKind {
        self.kind
            .as_deref()
            .and_then(crate::oracle::ProviderKind::parse)
            .unwrap_or(crate::oracle::ProviderKind::StateRoot)
    }
}

fn default_algo() -> String {
    "keccak256".to_string()
}

fn default_interval() -> u64 {
    12
}

// ── Client config ───────────────────────────────────────────────────────────

/// TOML config for `hash-market-client`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClientConfig {
    /// Ethereum JSON-RPC URL
    pub eth_rpc: String,
    /// hash-market-server URL (e.g. "http://localhost:9090")
    pub sidecar_url: String,
    /// Runtime identifier
    pub runtime_id: String,
    /// Chain UID (e.g. "ethereum-mainnet")
    pub chain_uid: String,
    /// Polling interval in seconds (default: 12)
    #[serde(default = "default_interval")]
    pub interval_secs: u64,
    /// Ethereum account to prove state for
    pub account_address: String,
    /// Storage keys to include in the proof
    #[serde(default)]
    pub storage_keys: Vec<String>,
}
