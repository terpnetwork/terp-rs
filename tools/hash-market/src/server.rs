//! Unified hash-market server.
//!
//! Composes:
//! - Vote Extension server (VE routes for ABCI ExtendVote/VerifyVote)
//! - Blossom server (BUD protocol for blob/merkle tree serving)
//!
//! The unified `AppState` holds both VE handler and blob storage, allowing
//! both protocol handlers to share the same runtime.
//!
//! # Architecture
//!
//! Blossom blob storage delegates to `TreeStore` (file-based with `/f/` prefix)
//! via the `BlobStore` trait — no independent in-memory store exists. Content
//! lookups are O(1) through the QMD-style content-ID index layered on top.
//! This matches the BUD-02/BUD-06 spec while keeping a single canonical store.

use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, Method, Request, Response, StatusCode};
use axum::{
    extract::{Path, State},
    middleware::{self, Next},
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use cw721_nips::buds::{self, BlossomState};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

#[cfg(feature = "ve")]
use crate::client::ve::VoteExtensionHandler;
use crate::oracle::{AggregationPolicy, OracleAttributeStore, OracleBoundsResponse};
use crate::store::{CashuMeshStore, HeadstashStore, MemberData, TreeInput, TreeStore};
#[cfg(feature = "transport")]
use crate::transport::TransportMode;
#[cfg(feature = "ve")]
use crate::ve::server as ve_server;
#[cfg(feature = "ve")]
use crate::{ProviderKey, ProviderStatus};

// ── Unified AppState ─────────────────────────────────────────────────────────

/// Unified server state — Blossom/BUD + Merkle trees always;
/// vote-extension fields only when compiled with `ve` and enabled at runtime.
///
/// All blob storage (BUD, blossom) delegates to `tree_store` through
/// `blossom.store`, which is `Arc<dyn BlobStore>` backed by `TreeStore`.
pub struct AppState {
    /// Vote extension signing handler (None when VE disabled)
    #[cfg(feature = "ve")]
    pub ve_handler: Option<VoteExtensionHandler>,
    /// Runtime VE flag (false for mint/whitelist host)
    pub ve_enabled: bool,
    /// CometBFT chain ID
    pub chain_id: String,
    /// Provider data for VE (chain_uid → data)
    #[cfg(feature = "ve")]
    pub provider_data: RwLock<HashMap<ProviderKey, ve_server::ProviderData>>,
    /// Provider metadata for status
    #[cfg(feature = "ve")]
    pub provider_status: RwLock<Vec<ProviderStatus>>,
    /// Current block height
    pub current_height: RwLock<u64>,
    /// Blossom blob storage — delegates to TreeStore via BlobStore trait
    pub blossom: Arc<BlossomState>,
    /// Merkle tree storage (NFT whitelist / headstash trees)
    pub tree_store: Arc<TreeStore>,
    /// Headstash metadata storage
    pub headstash_store: Arc<HeadstashStore>,
    /// Cashu mesh: mint discovery cache + encrypted wallet stash (`data/cashu/…`).
    pub cashu_mesh: Arc<CashuMeshStore>,
    /// When true, `GET /cashu/mints*` does not require auth (canonical discovery preference).
    pub cashu_public_mint_list: bool,
    /// When false, `/cashu/*` routes are not mounted.
    pub cashu_mesh_enabled: bool,
    /// Connect-style multi-source price bounds (None when disabled).
    pub oracle_store: Option<Arc<OracleAttributeStore>>,
    /// Aggregation policy for price bounds.
    pub oracle_policy: AggregationPolicy,
    /// Optional BUD+IPFS dual-index distribution (None when disabled).
    pub distribution: Option<Arc<crate::content::DistributionRuntime>>,
    /// Soulbound trusted-builder claims (manifesto signatures).
    pub trusted_builder: RwLock<HashMap<String, crate::trusted_builder::TrustedBuilderClaim>>,
    /// Embedded onboard feegrant (None = do not nest `/onboard`).
    #[cfg(feature = "onboard")]
    pub onboard: Option<Arc<crate::onboard_authz::OnboardRuntime>>,
    /// Sole terp1 allowed to write `/trees`. Reads stay public.
    pub tree_operator: String,
}

impl AppState {
    /// Create blossom/merkle host state (VE optional).
    ///
    /// `BlossomState` is initialized with `TreeStore` as its `BlobStore`
    /// backend — no separate in-memory store is created.
    pub fn new(
        #[cfg(feature = "ve")] ve_handler: Option<VoteExtensionHandler>,
        chain_id: String,
        data_dir: PathBuf,
        ve_enabled: bool,
    ) -> anyhow::Result<Self> {
        Self::new_with_oracle(
            #[cfg(feature = "ve")]
            ve_handler,
            chain_id,
            data_dir,
            ve_enabled,
            false,
            AggregationPolicy::default(),
        )
    }

    /// Create host state with optional Connect-style oracle bounds store.
    pub fn new_with_oracle(
        #[cfg(feature = "ve")] ve_handler: Option<VoteExtensionHandler>,
        chain_id: String,
        data_dir: PathBuf,
        ve_enabled: bool,
        oracle_bounds_enabled: bool,
        oracle_policy: AggregationPolicy,
    ) -> anyhow::Result<Self> {
        Self::new_full(
            #[cfg(feature = "ve")]
            ve_handler,
            chain_id,
            data_dir,
            ve_enabled,
            oracle_bounds_enabled,
            oracle_policy,
            None,
        )
    }

    /// Full host state including optional content distribution (BUD + IPFS labels).
    pub fn new_full(
        #[cfg(feature = "ve")] ve_handler: Option<VoteExtensionHandler>,
        chain_id: String,
        data_dir: PathBuf,
        ve_enabled: bool,
        oracle_bounds_enabled: bool,
        oracle_policy: AggregationPolicy,
        distribution: Option<Arc<crate::content::DistributionRuntime>>,
    ) -> anyhow::Result<Self> {
        let tree_store = Arc::new(TreeStore::open(data_dir.join("trees"))?);
        let headstash_store = Arc::new(HeadstashStore::new(&data_dir)?);
        let cashu_mesh = Arc::new(CashuMeshStore::new(&data_dir)?);
        let blossom = Arc::new(BlossomState::new(tree_store.clone()));

        Ok(Self {
            #[cfg(feature = "ve")]
            ve_handler,
            ve_enabled,
            chain_id,
            #[cfg(feature = "ve")]
            provider_data: RwLock::new(HashMap::new()),
            #[cfg(feature = "ve")]
            provider_status: RwLock::new(Vec::new()),
            current_height: RwLock::new(0),
            blossom,
            tree_store,
            headstash_store,
            cashu_mesh,
            // Lab-friendly defaults; server binary overrides from `[cashu_mesh]`.
            cashu_public_mint_list: true,
            cashu_mesh_enabled: true,
            oracle_store: if oracle_bounds_enabled {
                Some(Arc::new(OracleAttributeStore::new()))
            } else {
                None
            },
            oracle_policy,
            distribution,
            trusted_builder: RwLock::new(HashMap::new()),
            #[cfg(feature = "onboard")]
            onboard: None,
            tree_operator: crate::config::DEFAULT_TREE_OPERATOR.into(),
        })
    }

    pub fn set_tree_operator(&mut self, operator: impl Into<String>) {
        self.tree_operator = operator.into();
    }

    /// Apply `[cashu_mesh]` config (call before wrapping in `Arc` for the router).
    pub fn set_cashu_mesh_policy(&mut self, enabled: bool, public_mint_list: bool) {
        self.cashu_mesh_enabled = enabled;
        self.cashu_public_mint_list = public_mint_list;
    }

    /// Embed onboard feegrant (call before `Arc` + `router`).
    #[cfg(feature = "onboard")]
    pub fn set_onboard(&mut self, rt: Arc<crate::onboard_authz::OnboardRuntime>) {
        self.onboard = Some(rt);
    }

    /// Embed grant plane (roles must be live terp1). Used by the orch suite.
    #[cfg(feature = "onboard")]
    pub fn enable_onboard_roles(
        &mut self,
        dao: impl Into<String>,
        runtime_key: impl Into<String>,
        chain_id: impl Into<String>,
    ) {
        let mut rt = crate::onboard_authz::OnboardRuntime::from_env();
        rt.service_name = "hash-market-onboard".into();
        rt.roles.runtime = dao.into();
        rt.roles.hashmerchant = runtime_key.into();
        rt.roles.chain_id = chain_id.into();
        rt.broadcast = false;
        self.onboard = Some(Arc::new(rt));
    }

    /// Replace blossom/notes [`AuthVerifier`] (call before wrapping in `Arc` for the router).
    pub fn set_auth_verifier(&mut self, auth: Arc<dyn cw721_nips::buds::AuthVerifier>) {
        if let Some(b) = Arc::get_mut(&mut self.blossom) {
            b.auth = auth;
            return;
        }
        let store = self.blossom.store.clone();
        let mut blossom = BlossomState::new(store).with_auth(auth);
        if let Some(pay) = self.blossom.payment.clone() {
            blossom = blossom.with_payment(pay);
        }
        self.blossom = Arc::new(blossom);
    }

    /// Convenience: merkle/BUD-only host (no VE).
    pub fn new_merkle_host(chain_id: String, data_dir: PathBuf) -> anyhow::Result<Self> {
        Self::new(
            #[cfg(feature = "ve")]
            None,
            chain_id,
            data_dir,
            false,
        )
    }

    /// Add a provider to track (VE mode only).
    #[cfg(feature = "ve")]
    pub async fn add_provider(&self, status: ProviderStatus) {
        let mut statuses = self.provider_status.write().await;
        statuses.push(status);
    }
}
// ── Unified Router ───────────────────────────────────────────────────────────

/// Build the unified router combining Blossom/Merkle (+ optional VE) routes.
///
/// Route layout:
/// ```text
/// /health                      — unified health (mode + blossom + trees)
/// /ve/health                   — alias when VE off (mint pages / e2e)
/// /blobs/{hash}  GET, DELETE   — BUD-02 blob retrieval/deletion
/// /blobs         GET, POST     — BUD-06 list/upload
/// /upload        POST          — alias for /blobs POST
/// /trees/{id}    GET — public tree. POST, PUT, DELETE — one ADR-36 operator
/// /trees/{id}/members/{addr} GET — public proof for one address (mint page)
/// /trees         GET           — list tree IDs
/// /headstash/{id} GET, POST    — headstash registration
/// /notes/{hs_id}/{addr} GET, PUT, POST — private encrypted notes (auth; not public /content)
/// /notes/{hs_id} GET           — list encrypted-note keys (lab GET-by-addr setup)
/// /notes/{hs_id}/pir-index GET — public-leaf PIR key order (feature `pir`; else 404)
/// /notes/{hs_id}/pir POST      — XOR PIR over public leaf blobs (feature `pir`; else 404)
/// /cashu/mints/{id} GET, PUT, POST, DELETE — canonical mint discovery cache
/// /cashu/mints GET             — list mints (?status=; public when cashu_public_mint_list)
/// /cashu/wallets/{wid}/{iid}   — encrypted Cashu wallet backup (auth; never public /content)
/// /cashu/wallets/{wid} GET     — list wallet item keys (auth)
/// /ve/*                        — VE routes only when ve_enabled + feature ve
/// /vote-extension              — root alias of /ve/vote-extension (terpd + L3)
/// /onboard/status              — embedded feegrant (not root /status)
/// /onboard/v1/register-grant   — POST {address}
/// /onboard/v1/mint-grant       — POST {address}
/// ```
/// Shared blossom/tree/oracle routes. When `ve_health_alias`, register `/ve/health`
/// before `with_state` (mint host). When VE is on, nest owns `/ve/*` instead.
fn blossom_routes(state: Arc<AppState>, ve_health_alias: bool) -> Router {
    let cashu_on = state.cashu_mesh_enabled;
    let mut r = Router::new()
        .route("/health", get(unified_health))
        // Connect-style price bounds (opt-in; 503 when store disabled)
        .route("/oracle/bounds", get(oracle_bounds_get))
        .route("/oracle/ticks", get(oracle_ticks_get))
        .route("/blobs/:hash", get(blob_get).delete(blob_delete))
        .route("/blobs", get(blob_list).post(blob_upload))
        .route("/upload", post(blob_upload))
        .route(
            "/trees/:id",
            get(tree_get)
                .post(tree_save)
                .put(tree_save)
                .delete(tree_delete),
        )
        .route("/trees/:id/members/:addr", get(tree_member_proof))
        .route("/trees", get(tree_list))
        .route("/headstash/:id", get(headstash_get).post(headstash_save))
        // Private notes: HeadstashStore only (auth-gated). Not dual-index /content.
        // Register /pir and /pir-index before /:addr so those segments are not addresses.
        // PIR payload is feature-gated; blossom/NIP-BUD routes stay either way.
        .route("/notes/:hs_id/pir", post(note_pir))
        .route("/notes/:hs_id/pir-index", get(note_pir_index))
        .route("/notes/:hs_id", get(note_list))
        .route(
            "/notes/:hs_id/:addr",
            get(note_get).put(note_put).post(note_put),
        )
        // Content distribution (BUD label + optional IPFS) — handlers no-op/404 when disabled
        .route("/content/:sha256", get(content_get))
        .route("/content/:sha256/meta", get(content_meta))
        .route("/content/by-ipfs/:cid", get(content_get_by_ipfs))
        .route("/content", get(content_list))
        .route("/webhooks/s3", post(webhook_s3_ingest))
        .route("/content/register", post(content_register));
    if cashu_on {
        // Cashu mesh: canonical mint discovery cache + encrypted wallet stash.
        // Wallet proofs never dual-indexed as public /content SSOT.
        r = r
            .route(
                "/cashu/mints/:mint_id",
                get(cashu_mint_get)
                    .put(cashu_mint_put)
                    .post(cashu_mint_put)
                    .delete(cashu_mint_delete),
            )
            .route("/cashu/mints", get(cashu_mint_list))
            .route(
                "/cashu/wallets/:wallet_id/:item_id",
                get(cashu_wallet_get)
                    .put(cashu_wallet_put)
                    .post(cashu_wallet_put)
                    .delete(cashu_wallet_delete),
            )
            .route("/cashu/wallets/:wallet_id", get(cashu_wallet_list));
    }
    if ve_health_alias {
        r = r.route("/ve/health", get(unified_health));
    }
    r.with_state(state)
}

pub fn router(state: Arc<AppState>) -> Router {
    #[cfg(feature = "ve")]
    if state.ve_enabled {
        // Nested under /ve/* (historical) + root aliases for terpd / mock parity:
        // GET {sidecar_url}/vote-extension with sidecar_url = http://host:9090
        // Do not also register /ve/health on the parent — nest owns that path.
        let mut r = blossom_routes(state.clone(), false)
            .nest("/ve", ve_server::router(state.clone()))
            .nest(
                "/trusted-builder",
                crate::trusted_builder::router(state.clone()),
            )
            .merge(ve_server::root_aliases(state.clone()));
        #[cfg(feature = "onboard")]
        if let Some(rt) = state.onboard.clone() {
            r = r.nest("/onboard", crate::onboard::router(rt));
        }
        #[cfg(feature = "re-zerve")]
        {
            r = r.nest("/re-zerve", re_zerve_router());
        }
        return with_cors(r);
    }

    // Mint host (or ve feature off): /ve/health aliases unified health for operators/e2e
    let mut r = blossom_routes(state.clone(), true).nest(
        "/trusted-builder",
        crate::trusted_builder::router(state.clone()),
    );
    #[cfg(feature = "onboard")]
    if let Some(rt) = state.onboard.clone() {
        r = r.nest("/onboard", crate::onboard::router(rt));
    }
    #[cfg(feature = "re-zerve")]
    {
        r = r.nest("/re-zerve", re_zerve_router());
    }
    with_cors(r)
}

#[cfg(feature = "re-zerve")]
fn re_zerve_router() -> Router {
    Router::new().route("/v1/deposit", post(re_zerve_deposit))
}

#[cfg(feature = "re-zerve")]
async fn re_zerve_deposit(
    Json(att): Json<crate::re_zerve::DepositAttestation>,
) -> (StatusCode, Json<serde_json::Value>) {
    match att.verify(&att.frost_group_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "seam": "re-zerve"})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok": false, "error": e.to_string()})),
        ),
    }
}

fn with_cors(r: Router) -> Router {
    r.layer(middleware::from_fn(cors_mw))
}

async fn cors_mw(req: Request<Body>, next: Next) -> Response<Body> {
    let origin = "*".to_string();
    if req.method() == Method::OPTIONS {
        let mut res = Response::new(axum::body::Body::empty());
        *res.status_mut() = StatusCode::NO_CONTENT;
        put_cors(res.headers_mut(), &origin);
        return res;
    }
    let mut res = next.run(req).await;
    put_cors(res.headers_mut(), &origin);
    res
}

fn put_cors(h: &mut axum::http::HeaderMap, origin: &str) {
    if let Ok(v) = HeaderValue::from_str(origin) {
        h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, v);
    }
    h.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("content-type"),
    );
}

// ── Health ───────────────────────────────────────────────────────────────────

async fn unified_health(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    #[cfg(feature = "ve")]
    let ve_status = if state.ve_enabled {
        let statuses = state.provider_status.read().await;
        let active = statuses.iter().filter(|s| s.running).count();
        serde_json::json!({
            "enabled": true,
            "active_providers": active,
            "total_providers": statuses.len(),
        })
    } else {
        serde_json::json!({ "enabled": false })
    };
    #[cfg(not(feature = "ve"))]
    let ve_status = serde_json::json!({ "enabled": false, "compiled": false });

    let trees = state.tree_store.list();
    let blossom_status = serde_json::json!({
        "blobs": state.blossom.store.list().len(),
        "trees": trees.len(),
        "store": "TreeStore (/f/ prefix + QMD content-ID index)",
    });

    let distribution_status = if let Some(d) = &state.distribution {
        serde_json::json!({
            "enabled": true,
            "ipfs_api": d.config.ipfs_api.is_some(),
            "pin_on_upload": d.config.pin_on_upload,
            "webhooks": d.config.webhooks.len(),
            "registry_entries": d.registry.list().map(|l| l.len()).unwrap_or(0),
        })
    } else {
        serde_json::json!({ "enabled": false })
    };

    let oracle_status = if let Some(store) = &state.oracle_store {
        let bounds = store.list_bounds();
        serde_json::json!({
            "bounds_enabled": true,
            "method": state.oracle_policy.method.as_str(),
            "min_sources": state.oracle_policy.min_sources,
            "markets": bounds.iter().map(|b| &b.market_id).collect::<Vec<_>>(),
            "role": "bound_only",
        })
    } else {
        serde_json::json!({ "bounds_enabled": false })
    };

    #[cfg(feature = "onboard")]
    let onboard_status = if state.onboard.is_some() {
        serde_json::json!({ "enabled": true, "prefix": "/onboard" })
    } else {
        serde_json::json!({ "enabled": false })
    };
    #[cfg(not(feature = "onboard"))]
    let onboard_status = serde_json::json!({ "enabled": false });

    Json(serde_json::json!({
        "status": "ok",
        "mode": if state.ve_enabled { "validator-sidecar" } else { "merkle-bud-host" },
        "ve": ve_status,
        "oracle": oracle_status,
        "blossom": blossom_status,
        "distribution": distribution_status,
        "onboard": onboard_status,
        "trees": trees,
    }))
}

// ── Oracle bounds (Connect-style multi-source mid) ───────────────────────────

#[derive(serde::Deserialize, Default)]
struct OracleBoundsQuery {
    market_id: Option<String>,
}

/// GET /oracle/bounds?market_id=ETH/USD
///
/// Returns the aggregated mid for a market (or all markets when omitted).
/// Role is always `bound_only` — never a mint instruction.
async fn oracle_bounds_get(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<OracleBoundsQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    let store = state
        .oracle_store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    if let Some(mid) = q.market_id.as_deref().filter(|s| !s.is_empty()) {
        let bound = store.get_bound(mid).ok_or(StatusCode::NOT_FOUND)?;
        return Ok(Json(serde_json::json!(OracleBoundsResponse::from(&bound))));
    }

    let all: Vec<OracleBoundsResponse> = store
        .list_bounds()
        .iter()
        .map(OracleBoundsResponse::from)
        .collect();
    Ok(Json(
        serde_json::json!({ "bounds": all, "role": "bound_only" }),
    ))
}

/// GET /oracle/ticks?market_id=ETH/USD — raw attributed sources (debug / operators).
async fn oracle_ticks_get(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<OracleBoundsQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    let store = state
        .oracle_store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let ticks = store.list_ticks(q.market_id.as_deref().filter(|s| !s.is_empty()));
    Ok(Json(serde_json::json!({ "ticks": ticks })))
}
// ── Blossom/BUD Handlers ─────────────────────────────────────────────────────
//
// These are thin wrappers that delegate to BlossomState (which backs to
// TreeStore via the BlobStore trait). Helper functions (parse_hash,
// check_scope, extract_payment_proof) are re-exported from cw721-nips::buds
// to avoid duplication with the bud06.rs canonical implementation.

/// GET /blobs/{hash} — BUD-01 style blob retrieval.
async fn blob_get(
    State(state): State<Arc<AppState>>,
    Path(hash_hex): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    // Verify auth via blossom handler
    let claims = state
        .blossom
        .auth
        .verify(&headers)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    // Parse hash (uses bud06::parse_hash — canonical impl)
    let hash = buds::parse_hash(&hash_hex)?;

    // Check scope
    buds::check_scope(&claims, &hash)?;

    // Check payment if configured
    if let Some(pv) = &state.blossom.payment {
        let proof = buds::extract_payment_proof(&headers)?;
        pv.verify_download(&proof, &hash)
            .map_err(|_| StatusCode::PAYMENT_REQUIRED)?;
    }

    // Fetch blob — delegated to TreeStore via BlossomState::store
    let blob = state
        .blossom
        .store
        .get(&hash)
        .ok_or(StatusCode::NOT_FOUND)?;

    let mut resp = Response::new(Body::from(blob));
    if let Ok(hv) = HeaderValue::from_str(&hash_hex) {
        resp.headers_mut().insert("X-Content-SHA256", hv);
    }
    // Include metadata from the content-ID index
    if let Some(entry) = state.tree_store.blob_meta(&hash) {
        if let Ok(hv) = HeaderValue::from_str(&entry.size.to_string()) {
            resp.headers_mut().insert("X-Content-Length", hv);
        }
        if let Ok(hv) = HeaderValue::from_str(&entry.uploaded.to_string()) {
            resp.headers_mut().insert("X-Uploaded", hv);
        }
    }
    resp.headers_mut().insert(
        "Content-Type",
        HeaderValue::from_static("application/octet-stream"),
    );
    Ok(resp)
}

/// DELETE /blobs/{hash} — BUD-02 style blob deletion.
async fn blob_delete(
    State(state): State<Arc<AppState>>,
    Path(hash_hex): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    let claims = state
        .blossom
        .auth
        .verify(&headers)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    buds::check_action(&claims, "delete")?;
    let hash = buds::parse_hash(&hash_hex)?;

    // Delegates to TreeStore via BlossomState
    state.blossom.store.delete(&hash).map_err(|e| match e {
        cw721_nips::buds::BlobStoreError::NotFound => StatusCode::NOT_FOUND,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    })?;

    Ok(StatusCode::NO_CONTENT)
}

/// POST /blobs or /upload — BUD-07 upload with optional payment.
async fn blob_upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Body,
) -> Result<impl IntoResponse, StatusCode> {
    let claims = state
        .blossom
        .auth
        .verify(&headers)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    buds::check_action(&claims, "upload")?;

    let bytes = axum::body::to_bytes(body, 10 * 1024 * 1024)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    if let Some(pv) = &state.blossom.payment {
        let proof = buds::extract_payment_proof(&headers)?;
        pv.verify_upload(&proof, bytes.len() as u64)
            .map_err(|_| StatusCode::PAYMENT_REQUIRED)?;
    }

    // Delegates to TreeStore via BlossomState
    let raw = bytes.to_vec();
    let descriptor = state.blossom.store.put(raw.clone()).map_err(|e| match e {
        cw721_nips::buds::BlobStoreError::AlreadyExists => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    })?;

    let sha_hex = hex::encode(&descriptor.sha256);
    let mut content_meta = serde_json::json!({});
    if let Some(dist) = &state.distribution {
        let origin = crate::content::ContentOrigin {
            kind: "bud".into(),
            bucket: None,
            key: None,
        };
        match dist
            .pin_and_announce(
                descriptor.sha256,
                &raw,
                descriptor.size,
                descriptor.mime_type.clone(),
                origin,
                &[],
            )
            .await
        {
            Ok(rec) => {
                content_meta = serde_json::json!({
                    "ipfs_cid": rec.ipfs_cid,
                    "labels": rec.labels,
                    "urls": rec.urls(&dist.public_bud_base, &dist.config.ipfs_gateway_public),
                });
            }
            Err(e) => {
                tracing::warn!(error = %e, "distribution pin_and_announce failed");
            }
        }
    }

    Ok(Json(serde_json::json!({
        "url": descriptor.url,
        "sha256": sha_hex,
        "size": descriptor.size,
        "type": descriptor.mime_type,
        "uploaded": descriptor.uploaded,
        "content": content_meta,
    })))
}

/// GET /blobs — list all blob hashes with metadata.
async fn blob_list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    let claims = state
        .blossom
        .auth
        .verify(&headers)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    buds::check_action(&claims, "list")?;

    // Lists via QMD content-ID index (O(1) lookups, no filesystem scan)
    let hashes = state.blossom.store.list();
    // Single lock acquisition for all lookups
    let metas = state.tree_store.blob_metas(&hashes);
    let blob_entries: Vec<serde_json::Value> = hashes
        .iter()
        .map(|h| {
            let meta = metas.get(h);
            serde_json::json!({
                "sha256": hex::encode(h),
                "size": meta.map(|m| m.size),
                "uploaded": meta.map(|m| m.uploaded),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "blobs": blob_entries })))
}

// ── Content distribution (BUD label + optional IPFS) ─────────────────────────

/// GET /content/{sha256} — public resolve: local BUD store, else IPFS if registered.
async fn content_get(
    State(state): State<Arc<AppState>>,
    Path(sha_hex): Path<String>,
) -> Result<Response<Body>, StatusCode> {
    let dist = state.distribution.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    if !dist.config.public_get {
        return Err(StatusCode::FORBIDDEN);
    }
    let sha_hex = sha_hex.to_lowercase();
    let hash = buds::parse_hash(&sha_hex)?;

    // 1) Local BUD /f/ store
    if let Some(blob) = state.blossom.store.get(&hash) {
        let mut resp = Response::new(Body::from(blob));
        if let Ok(hv) = HeaderValue::from_str(&sha_hex) {
            resp.headers_mut().insert("X-Content-SHA256", hv);
        }
        resp.headers_mut().insert(
            "Content-Type",
            HeaderValue::from_static("application/octet-stream"),
        );
        resp.headers_mut()
            .insert("X-Content-Origin", HeaderValue::from_static("bud-local"));
        return Ok(resp);
    }

    // 2) Registry → IPFS
    let rec = dist
        .registry
        .get(&sha_hex)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if let Some(cid) = &rec.ipfs_cid {
        if dist.config.redirect_ipfs_on_miss {
            let gw = dist.config.ipfs_gateway_public.trim();
            let loc = if gw.starts_with("http") {
                format!("{}/{cid}", gw.trim_end_matches('/'))
            } else {
                format!("{gw}{cid}")
            };
            return Response::builder()
                .status(StatusCode::TEMPORARY_REDIRECT)
                .header("Location", loc)
                .header("X-Content-SHA256", sha_hex)
                .header("X-IPFS-CID", cid.as_str())
                .body(Body::empty())
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
        }
        if let Some(ipfs) = &dist.ipfs {
            match ipfs.cat_gateway(cid).await {
                Ok(bytes) => {
                    let mut resp = Response::new(Body::from(bytes));
                    resp.headers_mut().insert(
                        "X-Content-SHA256",
                        HeaderValue::from_str(&sha_hex).unwrap_or(HeaderValue::from_static("")),
                    );
                    resp.headers_mut().insert(
                        "X-IPFS-CID",
                        HeaderValue::from_str(cid).unwrap_or(HeaderValue::from_static("")),
                    );
                    resp.headers_mut()
                        .insert("X-Content-Origin", HeaderValue::from_static("ipfs"));
                    resp.headers_mut().insert(
                        "Content-Type",
                        HeaderValue::from_static("application/octet-stream"),
                    );
                    return Ok(resp);
                }
                Err(e) => {
                    tracing::warn!(error = %e, %cid, "ipfs resolve failed");
                }
            }
        }
    }
    Err(StatusCode::NOT_FOUND)
}

/// GET /content/by-ipfs/{cid} — resolve via dual-index cid → sha256 → local/IPFS.
async fn content_get_by_ipfs(
    State(state): State<Arc<AppState>>,
    Path(cid): Path<String>,
) -> Result<Response<Body>, StatusCode> {
    let dist = state.distribution.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    if !dist.config.public_get {
        return Err(StatusCode::FORBIDDEN);
    }
    let rec = dist
        .registry
        .get_by_cid(&cid)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    // Reuse sha256 path
    content_get(State(state), Path(rec.sha256)).await
}

/// GET /content/{sha256}/meta — dual-index record (sha256 + ipfs_cid + urls).
async fn content_meta(
    State(state): State<Arc<AppState>>,
    Path(sha_hex): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    let dist = state.distribution.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    let rec = dist
        .registry
        .get(&sha_hex.to_lowercase())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let urls = rec.urls(&dist.public_bud_base, &dist.config.ipfs_gateway_public);
    Ok(Json(serde_json::json!({
        "sha256": rec.sha256,
        "ipfs_cid": rec.ipfs_cid,
        "size": rec.size,
        "content_type": rec.content_type,
        "labels": rec.labels,
        "origins": rec.origins,
        "urls": urls,
        "updated_at": rec.updated_at,
    })))
}

/// GET /content — list registered content labels.
async fn content_list(State(state): State<Arc<AppState>>) -> Result<impl IntoResponse, StatusCode> {
    let dist = state.distribution.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    let list = dist
        .registry
        .list()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({ "content": list })))
}

/// POST /content/register — manual dual-index entry (bearer if configured).
async fn content_register(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, StatusCode> {
    let dist = state.distribution.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    let auth = headers.get("Authorization").and_then(|v| v.to_str().ok());
    let x_minio = headers
        .get("x-minio-webhook-auth")
        .and_then(|v| v.to_str().ok());
    if let Err(e) = dist.check_ingest_auth(auth, x_minio) {
        tracing::debug!(error = %e, "content register auth failed");
        return Err(StatusCode::UNAUTHORIZED);
    }
    let sha256 = body
        .get("sha256")
        .and_then(|x| x.as_str())
        .ok_or(StatusCode::BAD_REQUEST)?
        .to_lowercase();
    let rec = dist
        .registry
        .upsert(crate::content::ContentRecord {
            sha256,
            size: body.get("size").and_then(|x| x.as_u64()).unwrap_or(0),
            content_type: body
                .get("content_type")
                .and_then(|x| x.as_str())
                .map(str::to_string),
            ipfs_cid: body
                .get("ipfs_cid")
                .and_then(|x| x.as_str())
                .map(str::to_string),
            origins: vec![crate::content::ContentOrigin {
                kind: body
                    .get("origin")
                    .and_then(|x| x.as_str())
                    .unwrap_or("ingest")
                    .into(),
                bucket: body
                    .get("bucket")
                    .and_then(|x| x.as_str())
                    .map(str::to_string),
                key: body.get("key").and_then(|x| x.as_str()).map(str::to_string),
            }],
            labels: body
                .get("labels")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            updated_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs().to_string())
                .unwrap_or_default(),
        })
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(rec))
}

/// POST /webhooks/s3 — MinIO-style notify / oline pin bridge ingest.
/// Accepts either a `content.available` event or S3 `Records[]` with optional
/// precomputed `sha256` / `ipfs_cid` in user metadata extension fields.
async fn webhook_s3_ingest(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, StatusCode> {
    let dist = state.distribution.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    let auth = headers.get("Authorization").and_then(|v| v.to_str().ok());
    let x_minio = headers
        .get("x-minio-webhook-auth")
        .or_else(|| headers.get("X-Minio-Webhook-Auth"))
        .and_then(|v| v.to_str().ok());
    if let Err(e) = dist.check_ingest_auth(auth, x_minio) {
        tracing::debug!(error = %e, "webhook s3 auth failed");
        return Err(StatusCode::UNAUTHORIZED);
    }

    // Direct content.available passthrough (from peer or collector)
    if body.get("type").and_then(|x| x.as_str()) == Some("content.available") {
        let sha256 = body
            .get("sha256")
            .and_then(|x| x.as_str())
            .ok_or(StatusCode::BAD_REQUEST)?
            .to_lowercase();
        let rec = dist
            .registry
            .upsert(crate::content::ContentRecord {
                sha256: sha256.clone(),
                size: body.get("size").and_then(|x| x.as_u64()).unwrap_or(0),
                content_type: body
                    .get("content_type")
                    .and_then(|x| x.as_str())
                    .map(str::to_string),
                ipfs_cid: body
                    .get("ipfs_cid")
                    .and_then(|x| x.as_str())
                    .map(str::to_string),
                origins: vec![crate::content::ContentOrigin {
                    kind: body
                        .get("origin")
                        .and_then(|x| x.as_str())
                        .unwrap_or("s3")
                        .into(),
                    bucket: body
                        .get("bucket")
                        .and_then(|x| x.as_str())
                        .map(str::to_string),
                    key: body.get("key").and_then(|x| x.as_str()).map(str::to_string),
                }],
                labels: body
                    .get("labels")
                    .and_then(|x| x.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_else(|| dist.config.default_labels.clone()),
                updated_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs().to_string())
                    .unwrap_or_default(),
            })
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let event = crate::content::ContentAvailableEvent {
            r#type: "content.available".into(),
            sha256: rec.sha256.clone(),
            ipfs_cid: rec.ipfs_cid.clone(),
            size: rec.size,
            content_type: rec.content_type.clone(),
            origin: "s3".into(),
            bucket: rec.origins.first().and_then(|o| o.bucket.clone()),
            key: rec.origins.first().and_then(|o| o.key.clone()),
            labels: rec.labels.clone(),
            urls: rec.urls(&dist.public_bud_base, &dist.config.ipfs_gateway_public),
        };
        dist.webhooks.fanout(&event).await;
        return Ok(Json(serde_json::json!({ "ok": true, "record": rec })));
    }

    // Minimal S3 Records parse — register key without pin (collector already pinned)
    if let Some(records) = body.get("Records").and_then(|x| x.as_array()) {
        let mut registered = 0u32;
        for rec in records {
            let bucket = rec
                .pointer("/s3/bucket/name")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let key = rec
                .pointer("/s3/object/key")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let size = rec
                .pointer("/s3/object/size")
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            // Optional dual fields if collector enriched the event
            let sha256 = rec
                .get("sha256")
                .or_else(|| rec.pointer("/s3/object/userMetadata/sha256"))
                .and_then(|x| x.as_str())
                .map(|s| s.to_lowercase());
            let ipfs_cid = rec
                .get("ipfs_cid")
                .or_else(|| rec.pointer("/s3/object/userMetadata/ipfs_cid"))
                .and_then(|x| x.as_str())
                .map(str::to_string);
            if let Some(sha) = sha256 {
                let _ = dist.registry.upsert(crate::content::ContentRecord {
                    sha256: sha,
                    size,
                    content_type: None,
                    ipfs_cid,
                    origins: vec![crate::content::ContentOrigin {
                        kind: "s3".into(),
                        bucket: Some(bucket.into()),
                        key: Some(key.into()),
                    }],
                    labels: dist.config.default_labels.clone(),
                    updated_at: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs().to_string())
                        .unwrap_or_default(),
                });
                registered += 1;
            }
        }
        return Ok(Json(
            serde_json::json!({ "ok": true, "registered": registered }),
        ));
    }

    Err(StatusCode::BAD_REQUEST)
}

// ── Tree (Merkle) Handlers ────────────────────────────────────────────────────

/// GET /trees/{id} — get merkle tree by ID (includes all members + proofs).
/// Prefer `/trees/{id}/members/{addr}` from browsers to avoid large payloads.
async fn tree_get(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    let tree = state.tree_store.load(&id).ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(tree))
}

/// GET /trees/{id}/members/{addr} — public NFT whitelist proof for one address.
///
/// Response (eligible):
/// ```json
/// {
///   "eligible": true,
///   "tree_id": "...",
///   "merkle_root": "...",
///   "address": "terp1...",
///   "allocation": 1,
///   "tier": 0,
///   "proof_hashes": ["...", "..."]
/// }
/// ```
///
/// Mint pages pass `proof_hashes` to whitelist-merkletree `has_member` / mint msgs
/// so eligible addresses can bypass mint fees.
async fn tree_member_proof(
    State(state): State<Arc<AppState>>,
    Path((id, addr)): Path<(String, String)>,
) -> Result<impl IntoResponse, StatusCode> {
    let tree = state.tree_store.load(&id).ok_or(StatusCode::NOT_FOUND)?;
    // Normalize bech32 case for lookup (keys stored as uploaded)
    let member = tree
        .members
        .get(&addr)
        .or_else(|| tree.members.get(&addr.to_lowercase()))
        .cloned();

    match member {
        Some(MemberData {
            allocation,
            tier,
            proof_hashes,
        }) => Ok(Json(serde_json::json!({
            "eligible": true,
            "tree_id": id,
            "merkle_root": tree.root,
            "address": addr,
            "allocation": allocation,
            "tier": tier,
            "proof_hashes": proof_hashes,
        }))),
        None => Ok(Json(serde_json::json!({
            "eligible": false,
            "tree_id": id,
            "merkle_root": tree.root,
            "address": addr,
            "proof_hashes": [],
        }))),
    }
}

/// POST or PUT `/trees/{id}` — upload a gen_merkle JSON tree.
///
/// One operator. Headers `X-Hash-Market-Pubkey` and `X-Hash-Market-Signature`
/// are an ADR-36 signature over `tree_write_challenge(method, path, raw body)`.
/// Missing or bad signatures are 401. A valid signature for another address is 403.
/// Mint pages do not call this route.
async fn tree_save(
    State(state): State<Arc<AppState>>,
    method: Method,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Result<impl IntoResponse, StatusCode> {
    authorize_tree(&state, &method, &format!("/trees/{id}"), &body, &headers)?;
    let input: TreeInput = serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    input.validate().map_err(|e| {
        tracing::warn!(id = %id, error = %e, "invalid tree input");
        StatusCode::BAD_REQUEST
    })?;

    state.tree_store.save(&id, input).map_err(|e| {
        tracing::error!(id = %id, error = %e, "failed to save tree");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(StatusCode::CREATED)
}

/// DELETE `/trees/{id}` — same operator signature as upload, over an empty body.
async fn tree_delete(
    State(state): State<Arc<AppState>>,
    method: Method,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    authorize_tree(&state, &method, &format!("/trees/{id}"), &[], &headers)?;

    state.tree_store.delete(&id).map_err(|e| {
        tracing::error!(id = %id, error = %e, "failed to delete tree");
        StatusCode::NOT_FOUND
    })?;
    Ok(StatusCode::NO_CONTENT)
}

fn authorize_tree(
    state: &AppState,
    method: &Method,
    path: &str,
    body: &[u8],
    headers: &HeaderMap,
) -> Result<(), StatusCode> {
    let pubkey = header_str(headers, "x-hash-market-pubkey");
    let signature = header_str(headers, "x-hash-market-signature");
    match crate::tree_operator::authorize_tree_write(
        &state.tree_operator,
        method.as_str(),
        path,
        body,
        pubkey,
        signature,
    ) {
        Ok(()) => Ok(()),
        Err(crate::tree_operator::TreeWriteDeny::Forbidden) => Err(StatusCode::FORBIDDEN),
        Err(crate::tree_operator::TreeWriteDeny::Unauthorized) => Err(StatusCode::UNAUTHORIZED),
    }
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

/// GET /trees — list all tree IDs.
async fn tree_list(State(state): State<Arc<AppState>>) -> Json<Vec<String>> {
    Json(state.tree_store.list())
}

// ── Headstash Handlers ─────────────────────────────────────────────────────────

/// GET /headstash/{id} — get headstash registration.
async fn headstash_get(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    let data = state
        .headstash_store
        .get_headstash(&id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(data))
}

/// POST /headstash/{id} — save headstash registration (authenticated).
async fn headstash_save(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(data): Json<serde_json::Value>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .blossom
        .auth
        .verify(&headers)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    // See tree_save for design rationale — AuthClaims dropped intentionally:
    // single admin-key model, no per-action scoping.

    state
        .headstash_store
        .set_headstash(&id, &data)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::CREATED)
}

// ── Private note handlers (HeadstashStore; not public /content dual-index) ───

/// GET /notes/{hs_id}/{addr} — fetch encrypted note envelope (authenticated).
async fn note_get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((hs_id, addr)): Path<(String, String)>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .blossom
        .auth
        .verify(&headers)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    let data = state
        .headstash_store
        .get_note(&hs_id, &addr)
        .map_err(|e| {
            tracing::warn!(%hs_id, %addr, error = %e, "get_note failed");
            StatusCode::BAD_REQUEST
        })?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(data))
}

/// PUT|POST /notes/{hs_id}/{addr} — store encrypted note envelope (authenticated).
///
/// Body: `{ "ciphertext", "nonce", "scheme", ... }` — SEAM cleartext is never
/// parsed server-side. Caller is the mint/bridge client that holds plaintext.
///
/// When `[distribution]` is enabled and the envelope includes optional `sha256`
/// of the **ciphertext**, pin raw ciphertext bytes into TreeStore (BUD path only).
/// Primary fetch remains this auth-gated route — never public `/content` as SSOT.
async fn note_put(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((hs_id, addr)): Path<(String, String)>,
    Json(data): Json<serde_json::Value>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .blossom
        .auth
        .verify(&headers)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    // Optional distribution pin of ciphertext-only when client attached sha256.
    if state.distribution.is_some() {
        if let (Some(ct_hex), Some(sha_hex)) = (
            data.get("ciphertext").and_then(|v| v.as_str()),
            data.get("sha256").and_then(|v| v.as_str()),
        ) {
            if let Ok(ct) = hex::decode(ct_hex) {
                use sha2::{Digest, Sha256};
                let digest = hex::encode(Sha256::digest(&ct));
                if digest == sha_hex.to_lowercase() {
                    // TreeStore implements BlobStore via blossom.store
                    if let Err(e) = state.blossom.store.put(ct) {
                        tracing::warn!(%hs_id, %addr, error = %e, "ciphertext bud pin skipped");
                    }
                } else {
                    tracing::warn!(%hs_id, %addr, "envelope sha256 mismatch; skip bud pin");
                }
            }
        }
    }

    state
        .headstash_store
        .set_note(&hs_id, &addr, &data)
        .map_err(|e| {
            tracing::warn!(%hs_id, %addr, error = %e, "set_note rejected");
            StatusCode::BAD_REQUEST
        })?;
    Ok(StatusCode::CREATED)
}

/// GET /notes/{hs_id} — list note address keys (authenticated; for PIR setup).
async fn note_list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(hs_id): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .blossom
        .auth
        .verify(&headers)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    let keys = state.headstash_store.list_note_keys(&hs_id).map_err(|e| {
        tracing::warn!(%hs_id, error = %e, "list_note_keys failed");
        StatusCode::BAD_REQUEST
    })?;
    Ok(Json(serde_json::json!({ "keys": keys })))
}

/// GET /notes/{hs_id}/pir-index — public leaf order for the XOR selector.
///
/// 404 unless the binary is built with `--features pir` and notes were ingested.
/// This is not GET /notes/{hs_id}/{addr}.
async fn note_pir_index(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(hs_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    state
        .blossom
        .auth
        .verify(&headers)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    #[cfg(not(feature = "pir"))]
    {
        let _ = (state, hs_id);
        return Err(StatusCode::NOT_FOUND);
    }

    #[cfg(feature = "pir")]
    {
        let index = crate::pir::load_pir_index(state.headstash_store.as_ref(), &hs_id)
            .map_err(|e| {
                tracing::warn!(%hs_id, error = %e, "load_pir_index failed");
                StatusCode::BAD_REQUEST
            })?
            .ok_or(StatusCode::NOT_FOUND)?;
        Ok(Json(serde_json::to_value(index).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
    }
}

/// POST /notes/{hs_id}/pir — XOR over ingested public leaf blobs (authenticated).
///
/// Body: `{ "selector": [0, 1, 0, ...] }` with length == public index count.
/// Decoded blob is `{leaf, path, fdi, nd, v, recp, epk_x, epk_y, ...}`.
/// Does not call GET-by-addr and does not read `notes/{hs_id}/{addr}.json`.
/// Without feature `pir`, returns 404 and leaves NIP-BUD / encrypted notes alone.
async fn note_pir(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(hs_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    state
        .blossom
        .auth
        .verify(&headers)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    #[cfg(not(feature = "pir"))]
    {
        let _ = (state, hs_id, body);
        return Err(StatusCode::NOT_FOUND);
    }

    #[cfg(feature = "pir")]
    {
        let selector_vals = body
            .get("selector")
            .and_then(|v| v.as_array())
            .ok_or(StatusCode::BAD_REQUEST)?;
        let selector: Vec<u8> = selector_vals
            .iter()
            .map(|v| {
                if v.as_u64() == Some(1) || v.as_bool() == Some(true) {
                    1u8
                } else {
                    0u8
                }
            })
            .collect();

        let (_keys, blobs) = crate::pir::load_pir_blobs(state.headstash_store.as_ref(), &hs_id)
            .map_err(|e| {
                tracing::warn!(%hs_id, error = %e, "load_pir_blobs failed");
                StatusCode::BAD_REQUEST
            })?
            .ok_or(StatusCode::NOT_FOUND)?;

        let blob_refs: Vec<&[u8]> = blobs.iter().map(|b| b.as_slice()).collect();
        let result = crate::pir::xor_pir(&blob_refs, &selector).map_err(|e| {
            tracing::warn!(%hs_id, error = %e, "note pir failed");
            StatusCode::BAD_REQUEST
        })?;

        Ok(Json(serde_json::json!({
            "result": hex::encode(result)
        })))
    }
}

// ── Cashu mesh handlers (canonical mint cache + encrypted wallet stash) ─────
//
// Schema: docs/plans/cashu/CANONICAL-MINT-REGISTRY.md
// Private wallet envelopes: never dual-index on public /content.

/// GET /cashu/mints/{mint_id} — mint descriptor (public when `cashu_public_mint_list`).
async fn cashu_mint_get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(mint_id): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    if !state.cashu_public_mint_list {
        state
            .blossom
            .auth
            .verify(&headers)
            .map_err(|_| StatusCode::UNAUTHORIZED)?;
    }
    let data = state
        .cashu_mesh
        .get_mint(&mint_id)
        .map_err(|e| {
            tracing::warn!(%mint_id, error = %e, "get_mint failed");
            StatusCode::BAD_REQUEST
        })?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(data))
}

/// PUT|POST /cashu/mints/{mint_id} — upsert mint descriptor cache (authenticated).
async fn cashu_mint_put(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(mint_id): Path<String>,
    Json(data): Json<serde_json::Value>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .blossom
        .auth
        .verify(&headers)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    state
        .cashu_mesh
        .set_mint_json(&mint_id, &data)
        .map_err(|e| {
            tracing::warn!(%mint_id, error = %e, "set_mint rejected");
            StatusCode::BAD_REQUEST
        })?;
    Ok(StatusCode::CREATED)
}

/// DELETE /cashu/mints/{mint_id} — remove mesh cache row (authenticated).
async fn cashu_mint_delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(mint_id): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .blossom
        .auth
        .verify(&headers)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    let removed = state.cashu_mesh.delete_mint(&mint_id).map_err(|e| {
        tracing::warn!(%mint_id, error = %e, "delete_mint failed");
        StatusCode::BAD_REQUEST
    })?;
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

/// GET /cashu/mints — list mint descriptors.
///
/// Query: `?status=active|paused|revoked` (optional). Without filter, returns
/// **active-only** by default (canonical ListMints posture). Pass `?status=all`
/// for every row. Public when `cashu_public_mint_list` (default true).
async fn cashu_mint_list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<impl IntoResponse, StatusCode> {
    if !state.cashu_public_mint_list {
        state
            .blossom
            .auth
            .verify(&headers)
            .map_err(|_| StatusCode::UNAUTHORIZED)?;
    }

    let status = q.get("status").map(|s| s.as_str());
    let (filter, default_active_only) = match status {
        Some("all") | Some("*") => (None, false),
        Some(s) => (Some(s), false),
        None => (None, true),
    };

    let mints = state
        .cashu_mesh
        .list_mints(filter, default_active_only)
        .map_err(|e| {
            tracing::warn!(error = %e, "list_mints failed");
            StatusCode::BAD_REQUEST
        })?;
    let keys: Vec<String> = mints.iter().map(|m| m.mint_id.clone()).collect();
    Ok(Json(serde_json::json!({
        "keys": keys,
        "mints": mints,
    })))
}

/// GET /cashu/wallets/{wallet_id}/{item_id} — encrypted wallet backup (auth).
async fn cashu_wallet_get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((wallet_id, item_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .blossom
        .auth
        .verify(&headers)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    let data = state
        .cashu_mesh
        .get_wallet_item(&wallet_id, &item_id)
        .map_err(|e| {
            tracing::warn!(%wallet_id, %item_id, error = %e, "get_wallet_item failed");
            StatusCode::BAD_REQUEST
        })?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(data))
}

/// PUT|POST /cashu/wallets/{wallet_id}/{item_id} — store opaque encrypted envelope (auth).
async fn cashu_wallet_put(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((wallet_id, item_id)): Path<(String, String)>,
    Json(data): Json<serde_json::Value>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .blossom
        .auth
        .verify(&headers)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    state
        .cashu_mesh
        .set_wallet_item(&wallet_id, &item_id, &data)
        .map_err(|e| {
            tracing::warn!(%wallet_id, %item_id, error = %e, "set_wallet_item rejected");
            StatusCode::BAD_REQUEST
        })?;
    Ok(StatusCode::CREATED)
}

/// DELETE /cashu/wallets/{wallet_id}/{item_id} — remove wallet item (auth).
async fn cashu_wallet_delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((wallet_id, item_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .blossom
        .auth
        .verify(&headers)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    let removed = state
        .cashu_mesh
        .delete_wallet_item(&wallet_id, &item_id)
        .map_err(|e| {
            tracing::warn!(%wallet_id, %item_id, error = %e, "delete_wallet_item failed");
            StatusCode::BAD_REQUEST
        })?;
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

/// GET /cashu/wallets/{wallet_id} — list wallet item keys (auth).
async fn cashu_wallet_list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(wallet_id): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .blossom
        .auth
        .verify(&headers)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    let keys = state
        .cashu_mesh
        .list_wallet_item_keys(&wallet_id)
        .map_err(|e| {
            tracing::warn!(%wallet_id, error = %e, "list_wallet_item_keys failed");
            StatusCode::BAD_REQUEST
        })?;
    Ok(Json(serde_json::json!({ "keys": keys })))
}

// ── Provider Feeder Delegate ──────────────────────────────────────────────────

#[cfg(feature = "ve")]
/// Start a provider feeder (delegates to ve_server).
pub async fn run_provider_feeder(
    state: Arc<AppState>,
    provider_idx: usize,
    name: String,
    mode: TransportMode,
) {
    ve_server::run_provider_feeder(state, provider_idx, name, mode).await
}

// ── Binary entry point ─────────────────────────────────────────────────────────
//
// The actual binary lives in `src/bin/server.rs` which uses these library
// functions. This module provides the shared router, handlers, and AppState.
