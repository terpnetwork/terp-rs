//! Calendar off-chain binding to the dual-index content plane.
//!
//! # Phase B — B1 content binding
//!
//! | Mode | Chain holds | Body lives |
//! |------|-------------|------------|
//! | On-chain | full NIP-52 in `MetadataExt.e` | chain |
//! | Off-chain | `cid` + kind + d_tag | BUD/S3/IPFS via this plane |
//!
//! Off-chain events **must** use [`encode_for_chain`] bare sha256 (preferred) so
//! resolvers hit `GET /content/{sha256}` without inventing a second blob store.
//!
//! TreeStore remains the sole `BlobStore` for bytes.

use super::cid::{content_path, encode_for_chain, parse_cid, CidKind, ParsedCid};
use crate::store::TreeStore;
use cw721_nips::buds::BlobStore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// NIP-52 kind: date-based calendar event.
pub const KIND_DATE_BASED: u16 = 31922;
/// NIP-52 kind: time-based calendar event.
pub const KIND_TIME_BASED: u16 = 31923;
/// NIP-52 kind: calendar collection.
pub const KIND_CALENDAR: u16 = 31924;

/// Result of binding raw off-chain event JSON into TreeStore + chain cid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OffchainBind {
    /// Lowercase 64-hex BUD sha256.
    pub sha256: String,
    /// Canonical string for `MetadataExt.cid` (bare sha256).
    pub chain_cid: String,
    pub size: u64,
    /// Relative content-plane path: `/content/{sha256}`.
    pub content_path: String,
}

/// Chain-facing off-chain metadata view (mirrors dao-calendar `MetadataExt` off-chain mode).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OffchainCalendarMeta {
    pub on_chain: bool,
    pub cid: String,
    pub kind: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar_d: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_pubkey: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nostr_e_d: Option<String>,
}

impl OffchainCalendarMeta {
    pub fn new(chain_cid: impl Into<String>, kind: u16) -> Self {
        Self {
            on_chain: false,
            cid: chain_cid.into(),
            kind,
            d_tag: None,
            calendar_d: None,
            author_pubkey: None,
            nostr_e_d: None,
        }
    }

    pub fn with_d_tag(mut self, d: impl Into<String>) -> Self {
        self.d_tag = Some(d.into());
        self
    }

    pub fn with_calendar_d(mut self, d: impl Into<String>) -> Self {
        self.calendar_d = Some(d.into());
        self
    }

    pub fn with_author(mut self, pk: impl Into<String>) -> Self {
        self.author_pubkey = Some(pk.into());
        self
    }
}

/// Put event body bytes into TreeStore and return chain-ready cid binding.
///
/// Does **not** require HTTP or IPFS — pure local content plane. Callers that
/// also need dual-index registry / pin should use [`super::DistributionRuntime::pin_and_announce`].
///
/// Marketplace listings use the same path: see [`bind_listing_body`]. The returned
/// `chain_cid` (bare sha256) is the preferred `ListProduct.content_cid`.
pub fn bind_offchain_body(store: &TreeStore, body: &[u8]) -> Result<OffchainBind, String> {
    if body.is_empty() {
        return Err("empty off-chain body".into());
    }
    let desc = store
        .put(body.to_vec())
        .map_err(|e| format!("TreeStore put: {e}"))?;
    let sha = hex::encode(desc.sha256);
    debug_assert_eq!(Sha256::digest(body).as_slice(), desc.sha256.as_slice());
    let chain_cid = encode_for_chain(CidKind::BudSha256, &sha)?;
    let parsed = parse_cid(&chain_cid)?;
    Ok(OffchainBind {
        content_path: content_path(&parsed),
        sha256: sha,
        chain_cid,
        size: desc.size,
    })
}

/// Bind marketplace product / listing JSON into the content plane (no second store).
///
/// Alias of [`bind_offchain_body`]. Use `bind.chain_cid` as marketplace-mirror
/// `ListProduct.content_cid` (bare BUD sha256).
pub fn bind_listing_body(store: &TreeStore, body: &[u8]) -> Result<OffchainBind, String> {
    bind_offchain_body(store, body)
}

/// Build off-chain metadata for `CreateEvent` after binding body bytes.
pub fn bind_offchain_event(
    store: &TreeStore,
    body: &[u8],
    kind: u16,
) -> Result<(OffchainBind, OffchainCalendarMeta), String> {
    if !is_nip52_event_kind(kind) {
        return Err(format!(
            "kind {kind} is not a NIP-52 event (want {KIND_DATE_BASED} or {KIND_TIME_BASED})"
        ));
    }
    let bind = bind_offchain_body(store, body)?;
    let meta = OffchainCalendarMeta::new(&bind.chain_cid, kind);
    Ok((bind, meta))
}

/// Resolve off-chain body from TreeStore using chain `cid` string.
pub fn resolve_local(store: &TreeStore, cid: &str) -> Result<Vec<u8>, String> {
    let parsed = parse_cid(cid)?;
    match parsed.kind {
        CidKind::BudSha256 => {
            let bytes = hex::decode(&parsed.value).map_err(|e| format!("hex: {e}"))?;
            if bytes.len() != 32 {
                return Err(format!("sha256 length {}", bytes.len()));
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            store
                .get(&arr)
                .ok_or_else(|| format!("blob not found for {}", parsed.value))
        }
        CidKind::Ipfs => Err(
            "local resolve of bare IPFS cid requires dual-index registry; use resolve_http or by-ipfs"
                .into(),
        ),
    }
}

/// Absolute URL to fetch off-chain body from a content-plane base (no trailing slash).
pub fn resolve_url(base: &str, cid: &str) -> Result<String, String> {
    let parsed = parse_cid(cid)?;
    let path = content_path(&parsed);
    Ok(format!("{}{}", base.trim_end_matches('/'), path))
}

/// Same as [`resolve_url`] but from a pre-parsed cid.
pub fn resolve_url_parsed(base: &str, parsed: &ParsedCid) -> String {
    format!("{}{}", base.trim_end_matches('/'), content_path(parsed))
}

pub fn is_nip52_event_kind(kind: u16) -> bool {
    kind == KIND_DATE_BASED || kind == KIND_TIME_BASED
}

pub fn is_nip52_kind(kind: u16) -> bool {
    is_nip52_event_kind(kind) || kind == KIND_CALENDAR
}

/// HTTP GET resolve against a content plane base URL.
///
/// Available when `reqwest` is linked (`server` / `client` features).
#[cfg(any(feature = "server", feature = "client"))]
pub async fn resolve_http(base: &str, cid: &str) -> Result<Vec<u8>, String> {
    let url = resolve_url(base, cid)?;
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("GET {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GET {url} → HTTP {}", resp.status()));
    }
    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("body: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{
        encode_for_chain, parse_cid, CidKind, ContentOrigin, DistributionConfig,
        DistributionRuntime,
    };

    #[test]
    fn bind_and_resolve_local_roundtrip() {
        let dir = std::env::temp_dir().join(format!("hm-cal-bind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = TreeStore::open(dir.join("trees")).unwrap();

        let body = br#"{"title":"meetup","start":1700000000,"kind":31923}"#;
        let (bind, meta) = bind_offchain_event(&store, body, KIND_TIME_BASED).unwrap();
        assert_eq!(bind.chain_cid.len(), 64);
        assert!(!meta.on_chain);
        assert_eq!(meta.cid, bind.chain_cid);
        assert_eq!(meta.kind, KIND_TIME_BASED);
        assert_eq!(bind.content_path, format!("/content/{}", bind.sha256));

        let got = resolve_local(&store, &meta.cid).unwrap();
        assert_eq!(got, body);

        let url = resolve_url("http://127.0.0.1:9090", &meta.cid).unwrap();
        assert_eq!(
            url,
            format!("http://127.0.0.1:9090/content/{}", bind.sha256)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bind_listing_body_marketplace_product_json() {
        let dir = std::env::temp_dir().join(format!("hm-mkt-bind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = TreeStore::open(dir.join("trees")).unwrap();

        // NIP-15-ish product JSON labeled for marketplace-mirror ListProduct
        let body = br#"{"id":"product-tee","stall_id":"default","name":"Terp Tee","currency":"uterp","price":100,"quantity":10,"sku":"tee","t":"marketplace"}"#;
        let bind = bind_listing_body(&store, body).unwrap();
        assert_eq!(bind.chain_cid.len(), 64);
        assert_eq!(bind.chain_cid, bind.sha256);
        assert_eq!(parse_cid(&bind.chain_cid).unwrap().kind, CidKind::BudSha256);
        // Same bytes as bind_offchain_body — single content plane
        let via_alias = bind_offchain_body(&store, body).unwrap();
        assert_eq!(via_alias.chain_cid, bind.chain_cid);
        let got = resolve_local(&store, &bind.chain_cid).unwrap();
        assert_eq!(got, body);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reject_non_nip52_kind() {
        let dir = std::env::temp_dir().join(format!("hm-cal-kind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = TreeStore::open(dir.join("trees")).unwrap();
        assert!(bind_offchain_event(&store, b"{}", 1).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn dual_index_calendar_label_smoke() {
        let dir = std::env::temp_dir().join(format!("hm-cal-dist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = TreeStore::open(dir.join("trees")).unwrap();
        let body = br#"{"kind":31922,"title":"date-event"}"#;
        let (bind, meta) = bind_offchain_event(&store, body, KIND_DATE_BASED).unwrap();

        let mut dcfg = DistributionConfig {
            enabled: true,
            pin_on_upload: false,
            public_get: true,
            ..Default::default()
        };
        dcfg.default_labels = vec!["public".into(), "calendar".into()];
        let dist = DistributionRuntime::open(&dir, dcfg, "http://bud.example".into()).unwrap();

        let mut sha = [0u8; 32];
        hex::decode_to_slice(&bind.sha256, &mut sha).unwrap();
        let rec = dist
            .pin_and_announce(
                sha,
                body,
                bind.size,
                Some("application/json".into()),
                ContentOrigin {
                    kind: "bud".into(),
                    bucket: None,
                    key: None,
                },
                &["dao-calendar".into()],
            )
            .await
            .unwrap();
        assert!(rec.labels.contains(&"calendar".to_string()));
        assert!(rec.labels.contains(&"dao-calendar".to_string()));

        // Chain cid is bare sha256 — dual-index row keyed the same way
        assert_eq!(parse_cid(&meta.cid).unwrap().kind, CidKind::BudSha256);
        assert_eq!(
            encode_for_chain(CidKind::BudSha256, &bind.sha256).unwrap(),
            meta.cid
        );
        let meta_row = dist.registry.get(&bind.sha256).unwrap().unwrap();
        assert_eq!(meta_row.sha256, bind.sha256);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
