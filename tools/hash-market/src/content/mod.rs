//! Content distribution plane: BUD sha256 identity + optional IPFS CID + webhooks.
//!
//! Design (see `docs/bootstrap` / oline minio-ipfs + hash-market BUD synthesis):
//! - **Primary ecosystem label:** BUD SHA-256 of raw bytes
//! - **Secondary transport:** IPFS CID when Kubo pin succeeds
//! - **Events:** `content.available` fan-out (oline-compatible webhooks)
//! - **Non-disruptive:** disabled unless `[distribution] enabled = true`
//!
//! Does not replace TreeStore/BUD; it **labels and announces** public content.

mod auth;
pub mod calendar;
mod cid;
mod ipfs;
mod registry;
mod webhook;

pub use auth::{
    check_ingest_auth, resolve_service_jwt_secret, sign_service_jwt, verify_service_jwt,
    IngestAuthMode, ServiceJwtClaims, DEFAULT_AUD as DIST_JWT_AUD, ROLE_CONTENT_INGEST,
};
#[cfg(any(feature = "server", feature = "client"))]
pub use calendar::resolve_http;
pub use calendar::{
    bind_listing_body, bind_offchain_body, bind_offchain_event, is_nip52_event_kind, is_nip52_kind,
    resolve_local, resolve_url, resolve_url_parsed, OffchainBind, OffchainCalendarMeta,
    KIND_CALENDAR, KIND_DATE_BASED, KIND_TIME_BASED,
};
pub use cid::{content_path, encode_for_chain, parse_cid, CidKind, ParsedCid};
pub use ipfs::IpfsClient;
pub use registry::{ContentOrigin, ContentRecord, ContentRegistry};
pub use webhook::{ContentAvailableEvent, WebhookFanout};

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

/// TOML `[distribution]` — off by default so mint/VE support is unchanged.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct DistributionConfig {
    /// Master switch (default false).
    #[serde(default)]
    pub enabled: bool,
    /// Kubo HTTP API base, e.g. `http://127.0.0.1:5001` (optional).
    #[serde(default)]
    pub ipfs_api: Option<String>,
    /// Public gateway prefix for urls.ipfs, e.g. `https://ipfs.example/ipfs/` or `/ipfs/`.
    #[serde(default = "default_ipfs_gateway")]
    pub ipfs_gateway_public: String,
    /// Pin blob bytes to IPFS on BUD upload (requires ipfs_api).
    #[serde(default)]
    pub pin_on_upload: bool,
    /// Fail BUD upload if pin fails (default false — pin is best-effort).
    #[serde(default)]
    pub pin_required: bool,
    /// Webhook URLs to POST `content.available` JSON.
    #[serde(default)]
    pub webhooks: Vec<String>,
    /// Optional Bearer token for outgoing webhooks.
    #[serde(default)]
    pub webhook_bearer: Option<String>,
    /// Shared secret for POST /webhooks/s3 and POST /content/register (local/dev).
    #[serde(default)]
    pub ingest_bearer: Option<String>,
    /// Ingest auth: `bearer` | `jwt-plane` | `both` (default bearer).
    #[serde(default)]
    pub auth_mode: Option<String>,
    /// HMAC secret for plane service JWTs (else OLINE_SERVICE_JWT_SECRET env).
    #[serde(default)]
    pub jwt_secret: Option<String>,
    /// Expected JWT aud (default `hash-market-distribution`).
    #[serde(default)]
    pub jwt_audience: Option<String>,
    /// Require this role on JWT (default `content.ingest` when jwt-plane/both).
    #[serde(default)]
    pub jwt_require_role: Option<String>,
    /// Default labels applied on upload (e.g. ["public"]).
    #[serde(default)]
    pub default_labels: Vec<String>,
    /// When true, GET /content/{sha256} serves bytes without blossom auth if in registry.
    #[serde(default = "default_true")]
    pub public_get: bool,
    /// Prefer HTTP redirect to IPFS gateway when local miss but cid known.
    #[serde(default)]
    pub redirect_ipfs_on_miss: bool,
}

fn default_ipfs_gateway() -> String {
    "/ipfs/".into()
}
fn default_true() -> bool {
    true
}

impl DistributionConfig {
    pub fn active(&self) -> bool {
        self.enabled
    }

    pub fn ingest_auth_mode(&self) -> IngestAuthMode {
        self.auth_mode
            .as_deref()
            .map(IngestAuthMode::parse)
            .unwrap_or(IngestAuthMode::Bearer)
    }

    pub fn jwt_audience(&self) -> &str {
        self.jwt_audience
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(DIST_JWT_AUD)
    }

    pub fn jwt_required_role(&self) -> Option<&str> {
        match self.ingest_auth_mode() {
            IngestAuthMode::Bearer => None,
            IngestAuthMode::JwtPlane | IngestAuthMode::Both => Some(
                self.jwt_require_role
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(ROLE_CONTENT_INGEST),
            ),
        }
    }
}

/// Runtime handle held on AppState when distribution is enabled.
pub struct DistributionRuntime {
    pub config: DistributionConfig,
    pub registry: Arc<ContentRegistry>,
    pub ipfs: Option<IpfsClient>,
    pub webhooks: WebhookFanout,
    pub public_bud_base: String,
}

impl DistributionRuntime {
    pub fn open(
        data_dir: &Path,
        config: DistributionConfig,
        public_bud_base: String,
    ) -> anyhow::Result<Self> {
        let registry = Arc::new(ContentRegistry::open(data_dir.join("content-registry"))?);
        let ipfs = config
            .ipfs_api
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .map(|api| IpfsClient::new(api.clone(), config.ipfs_gateway_public.clone()));
        let webhooks = WebhookFanout::new(config.webhooks.clone(), config.webhook_bearer.clone());
        Ok(Self {
            config,
            registry,
            ipfs,
            webhooks,
            public_bud_base,
        })
    }

    /// After local BUD put: dual-index, optional IPFS pin, webhook fan-out.
    /// Never fails the caller for pin/webhook errors unless `pin_required`.
    pub async fn on_blob_stored(
        &self,
        sha256: [u8; 32],
        size: u64,
        content_type: Option<String>,
        origin: ContentOrigin,
        extra_labels: &[String],
    ) -> anyhow::Result<ContentRecord> {
        let mut labels = self.config.default_labels.clone();
        for l in extra_labels {
            if !labels.contains(l) {
                labels.push(l.clone());
            }
        }

        self.registry.upsert(ContentRecord {
            sha256: hex::encode(sha256),
            size,
            content_type: content_type.clone(),
            ipfs_cid: None,
            origins: vec![origin.clone()],
            labels: labels.clone(),
            updated_at: now_rfc3339(),
        })
    }

    /// Pin raw bytes to IPFS and merge cid into registry; fire webhooks.
    pub async fn pin_and_announce(
        &self,
        sha256: [u8; 32],
        bytes: &[u8],
        size: u64,
        content_type: Option<String>,
        origin: ContentOrigin,
        extra_labels: &[String],
    ) -> anyhow::Result<ContentRecord> {
        let sha_hex = hex::encode(sha256);
        let mut labels = self.config.default_labels.clone();
        for l in extra_labels {
            if !labels.contains(l) {
                labels.push(l.clone());
            }
        }

        let mut ipfs_cid = None;
        if self.config.pin_on_upload {
            if let Some(ipfs) = &self.ipfs {
                match ipfs.add_bytes(bytes).await {
                    Ok(cid) => ipfs_cid = Some(cid),
                    Err(e) => {
                        tracing::warn!(error = %e, "ipfs pin failed");
                        if self.config.pin_required {
                            return Err(e);
                        }
                    }
                }
            } else if self.config.pin_required {
                anyhow::bail!("pin_on_upload/pin_required but ipfs_api not configured");
            }
        }

        let rec = self.registry.upsert(ContentRecord {
            sha256: sha_hex.clone(),
            size,
            content_type: content_type.clone(),
            ipfs_cid: ipfs_cid.clone(),
            origins: vec![origin.clone()],
            labels: labels.clone(),
            updated_at: now_rfc3339(),
        })?;

        let event = ContentAvailableEvent {
            r#type: "content.available".into(),
            sha256: sha_hex,
            ipfs_cid: rec.ipfs_cid.clone(),
            size,
            content_type,
            origin: origin.kind.clone(),
            bucket: origin.bucket.clone(),
            key: origin.key.clone(),
            labels: rec.labels.clone(),
            urls: rec.urls(&self.public_bud_base, &self.config.ipfs_gateway_public),
        };
        self.webhooks.fanout(&event).await;
        Ok(rec)
    }

    /// Validate Authorization / X-Minio-Webhook-Auth per auth_mode.
    pub fn check_ingest_auth(
        &self,
        auth_header: Option<&str>,
        x_minio: Option<&str>,
    ) -> Result<(), String> {
        let mode = self.config.ingest_auth_mode();
        let secret = resolve_service_jwt_secret(self.config.jwt_secret.as_deref());
        let secret_ref = secret.as_deref();
        check_ingest_auth(
            auth_header,
            x_minio,
            &mode,
            self.config.ingest_bearer.as_deref(),
            secret_ref,
            self.config.jwt_audience(),
            self.config.jwt_required_role(),
        )
    }
}

fn now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Minimal RFC3339-ish UTC without chrono dep
    format!("{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::TreeStore;
    use cw721_nips::buds::BlobStore;
    use sha2::{Digest, Sha256};

    #[test]
    fn registry_dual_index_roundtrip() {
        let dir = std::env::temp_dir().join(format!("hm-content-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let reg = ContentRegistry::open(&dir).unwrap();
        let rec = reg
            .upsert(ContentRecord {
                sha256: "ab".repeat(32),
                size: 10,
                content_type: Some("text/plain".into()),
                ipfs_cid: Some("bafytest".into()),
                origins: vec![ContentOrigin {
                    kind: "bud".into(),
                    bucket: None,
                    key: None,
                }],
                labels: vec!["public".into()],
                updated_at: "1".into(),
            })
            .unwrap();
        assert_eq!(rec.ipfs_cid.as_deref(), Some("bafytest"));
        let got = reg.get(&rec.sha256).unwrap().unwrap();
        assert_eq!(got.size, 10);
        let urls = got.urls("https://bud.example", "https://ipfs.example/ipfs/");
        assert!(urls.bud.contains(&rec.sha256));
        assert!(urls.ipfs.unwrap().contains("bafytest"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L0/L1 dual-index smoke: TreeStore put → registry announce → local resolve.
    /// (No HTTP server / blossom auth — pure content plane.)
    #[tokio::test]
    async fn dual_index_upload_meta_get_smoke() {
        let dir = std::env::temp_dir().join(format!("hm-dual-smoke-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let store = TreeStore::open(dir.join("trees")).unwrap();
        let body = br#"{"kind":31923,"title":"calendar-smoke"}"#;
        let desc = store.put(body.to_vec()).unwrap();
        let sha = hex::encode(desc.sha256);
        assert_eq!(sha.len(), 64);
        assert_eq!(Sha256::digest(body).as_slice(), desc.sha256.as_slice());

        // cid convention: bare sha256 for chain MetadataExt.cid
        let chain_cid = encode_for_chain(CidKind::BudSha256, &sha).unwrap();
        assert_eq!(chain_cid, sha);
        let parsed = parse_cid(&chain_cid).unwrap();
        assert_eq!(parsed.kind, CidKind::BudSha256);

        let mut dcfg = DistributionConfig {
            enabled: true,
            pin_on_upload: false, // no Kubo in unit smoke
            public_get: true,
            ..Default::default()
        };
        dcfg.default_labels = vec!["public".into(), "calendar".into()];
        let dist = DistributionRuntime::open(&dir, dcfg, "http://127.0.0.1:9090".into()).unwrap();

        let rec = dist
            .pin_and_announce(
                desc.sha256,
                body,
                desc.size,
                Some("application/json".into()),
                ContentOrigin {
                    kind: "bud".into(),
                    bucket: None,
                    key: None,
                },
                &[],
            )
            .await
            .unwrap();

        assert_eq!(rec.sha256, sha);
        assert!(rec.labels.contains(&"public".to_string()));
        let meta = dist.registry.get(&sha).unwrap().unwrap();
        assert_eq!(meta.size, body.len() as u64);

        // Resolve like GET /content/{sha256}: local TreeStore hit
        let got = store.get(&desc.sha256).unwrap();
        assert_eq!(got, body);

        // S3-shaped ingest path (webhook without bytes)
        let rec2 = dist
            .registry
            .upsert(ContentRecord {
                sha256: sha.clone(),
                size: body.len() as u64,
                content_type: Some("application/json".into()),
                ipfs_cid: Some("bafybeigdyrzt5example".into()),
                origins: vec![ContentOrigin {
                    kind: "s3".into(),
                    bucket: Some("public".into()),
                    key: Some("events/smoke.json".into()),
                }],
                labels: vec!["public".into()],
                updated_at: "2".into(),
            })
            .unwrap();
        assert_eq!(rec2.ipfs_cid.as_deref(), Some("bafybeigdyrzt5example"));
        assert!(rec2.origins.iter().any(|o| o.kind == "s3"));
        assert!(rec2.origins.iter().any(|o| o.kind == "bud"));

        let path = content_path(&parsed);
        assert_eq!(path, format!("/content/{sha}"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
