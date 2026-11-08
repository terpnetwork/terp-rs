//! DAO text-record / storage-item registration for NIP + mesh discovery.
//!
//! # Why
//!
//! DAOs register **relays, query RPCs, and marketplace contracts** via core
//! `set_item` or billboard **text records** — same JSON key when possible
//! (`mesh.v1`). NIP message structures (NIP-15, NIP-52, …) should produce
//! that JSON through **one trait method** so tooling and dao-dao actions share
//! a single registration path.
//!
//! # Traits
//!
//! - [`DaoRecordAction`] — required: record key + to/from JSON  
//! - [`NipDaoPlane`] — progressive Nostr filter heuristics for [`crate::NipMetadata`]  
//! - [`DaoRecordWasmHint`] — optional CosmWasm execute msg shapes (chain finality)
//!
//! Design: websites/dao-dao-ui/docs/superpowers/specs/2026-07-20-dao-record-nip-action-trait.md

use crate::error::{NipError, NipResult};
use crate::{NipKind, NipMetadata};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// Canonical DAO core storage + billboard text-record key for mesh discovery.
pub const DAO_MESH_RECORD_KEY: &str = "mesh.v1";

/// Types that register through DAO core storage or billboard text records.
///
/// Implement this once per struct; callers only need:
/// `value = T::to_dao_record_json(&t)` then `set_item { key: T::record_key(), value }`.
pub trait DaoRecordAction: Sized {
    /// Key for `set_item` / text record (e.g. [`DAO_MESH_RECORD_KEY`]).
    fn record_key() -> &'static str;

    /// Deterministic heuristic JSON for the DAO record value (not a signed Nostr event).
    fn to_dao_record_json(&self) -> NipResult<String>;

    /// Parse DAO record value back into `Self`.
    fn from_dao_record_json(raw: &str) -> NipResult<Self>;
}

/// Progressive-plane hints for types that already implement [`NipMetadata`].
pub trait NipDaoPlane: NipMetadata {
    /// Suggested REQ filters (kinds + tags) for discovery on mesh relays.
    fn nostr_filter_heuristic(&self, dao: Option<&str>) -> NostrFilterHeuristic;

    /// Optional patch for `mesh.v1.marketplace` when the NIP body implies plane data.
    fn mesh_marketplace_patch(&self) -> Option<MeshMarketplacePatch> {
        None
    }
}

/// Optional bridge to CosmWasm execute shapes (dao-dao action library / ICT).
pub trait DaoRecordWasmHint {
    fn wasm_execute_hint(&self) -> Option<WasmExecuteHint>;
}

// ── Mesh.v1 (first citizen record) ─────────────────────────────────────────

/// mesh.v1.marketplace block — keep fields aligned with TS `MeshMarketplaceV1`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeshMarketplacePatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_price: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_base: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted_denom: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_buffer_blocks: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modes: Option<Vec<String>>,
}

/// Versioned mesh descriptor stored at [`DAO_MESH_RECORD_KEY`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct MeshDescriptorV1 {
    pub v: u32,
    pub relays: Vec<String>,
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendar: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marketplace: Option<MeshMarketplacePatch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl MeshDescriptorV1 {
    pub fn new(query: impl Into<String>, relays: Vec<String>) -> Self {
        Self {
            v: 1,
            relays,
            query: query.into(),
            calendar: None,
            marketplace: None,
            notes: None,
        }
    }

    pub fn with_marketplace(mut self, m: MeshMarketplacePatch) -> Self {
        self.marketplace = Some(m);
        self
    }
}

impl DaoRecordAction for MeshDescriptorV1 {
    fn record_key() -> &'static str {
        DAO_MESH_RECORD_KEY
    }

    fn to_dao_record_json(&self) -> NipResult<String> {
        if self.v != 1 {
            return Err(NipError::Validation(format!(
                "unsupported mesh descriptor version {}",
                self.v
            )));
        }
        if self.query.is_empty() {
            return Err(NipError::Validation(
                "mesh.v1.query is required (HTTPS query plane)".into(),
            ));
        }
        serde_json::to_string(self).map_err(NipError::Serialization)
    }

    fn from_dao_record_json(raw: &str) -> NipResult<Self> {
        let m: Self = serde_json::from_str(raw).map_err(NipError::Serialization)?;
        if m.v != 1 {
            return Err(NipError::Validation(format!(
                "unsupported mesh descriptor version {}",
                m.v
            )));
        }
        if m.query.is_empty() {
            return Err(NipError::Validation("mesh.v1.query is required".into()));
        }
        Ok(m)
    }
}

// ── Progressive filter heuristic ───────────────────────────────────────────

/// JSON-friendly Nostr REQ filter fragment (TS `MarketplaceNostrFilter` compatible).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct NostrFilterHeuristic {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<u16>>,
    #[serde(rename = "#t", skip_serializing_if = "Option::is_none")]
    pub t_tags: Option<Vec<String>>,
    #[serde(rename = "#d", skip_serializing_if = "Option::is_none")]
    pub d_tags: Option<Vec<String>>,
    #[serde(rename = "#e", skip_serializing_if = "Option::is_none")]
    pub e_tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

impl NostrFilterHeuristic {
    pub fn marketplace_base(dao: Option<&str>, kinds: &[u16], mode_tag: Option<&str>) -> Self {
        let mut t = vec!["marketplace".to_string(), "dao-marketplace".to_string()];
        if let Some(d) = dao {
            t.push(d.to_string());
        }
        if let Some(m) = mode_tag {
            t.push(m.to_string());
        }
        Self {
            kinds: Some(kinds.to_vec()),
            t_tags: Some(t),
            d_tags: None,
            e_tags: None,
            limit: Some(50),
        }
    }
}

// ── Wasm execute hint ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct WasmExecuteHint {
    /// Snake_case CosmWasm execute body, e.g. `{"place_bid":{"auction_id":"1"}}`.
    pub msg: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub funds_denom: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub funds_amount: Option<String>,
}

// ── Blanket: any NipMetadata can dump kind-tagged registration stub ────────

/// Wrapper that stores a NIP payload under a namespaced record key.
/// Prefer embedding marketplace data in `mesh.v1`; use this for advanced splits.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct NipNamespacedRecord<T> {
    pub nip_kind: u16,
    pub payload: T,
}

impl<T> DaoRecordAction for NipNamespacedRecord<T>
where
    T: NipMetadata + Serialize + DeserializeOwned,
{
    fn record_key() -> &'static str {
        // Generic key; concrete NIPs should override via newtype for stable keys.
        "nip.record"
    }

    fn to_dao_record_json(&self) -> NipResult<String> {
        self.payload.validate()?;
        serde_json::to_string(self).map_err(NipError::Serialization)
    }

    fn from_dao_record_json(raw: &str) -> NipResult<Self> {
        let r: Self = serde_json::from_str(raw).map_err(NipError::Serialization)?;
        r.payload.validate()?;
        if r.nip_kind != r.payload.kind().kind_value() {
            return Err(NipError::Validation(format!(
                "nip_kind {} != payload kind {}",
                r.nip_kind,
                r.payload.kind().kind_value()
            )));
        }
        Ok(r)
    }
}

// ── NIP-15 plane helpers (kinds locked to nips::nip15) ─────────────────────

/// Build filter heuristic for NIP-15 marketplace modes without holding full metadata.
pub fn nip15_filter_heuristic(mode: Nip15PlaneMode, dao: Option<&str>) -> NostrFilterHeuristic {
    use crate::nips::nip15::Nip15Kind;
    match mode {
        Nip15PlaneMode::Fixed => NostrFilterHeuristic::marketplace_base(
            dao,
            &[Nip15Kind::SetProduct.kind_value()],
            Some("fixed"),
        ),
        Nip15PlaneMode::Auction => NostrFilterHeuristic::marketplace_base(
            dao,
            &[Nip15Kind::AuctionProduct.kind_value()],
            Some("auction"),
        ),
        Nip15PlaneMode::BuyNow => NostrFilterHeuristic::marketplace_base(
            dao,
            &[Nip15Kind::AuctionProduct.kind_value()],
            Some("buy_now"),
        ),
        Nip15PlaneMode::All => NostrFilterHeuristic::marketplace_base(
            dao,
            &[
                Nip15Kind::SetStall.kind_value(),
                Nip15Kind::SetProduct.kind_value(),
                Nip15Kind::AuctionProduct.kind_value(),
            ],
            None,
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nip15PlaneMode {
    Fixed,
    Auction,
    BuyNow,
    All,
}

/// Chain place_bid hint (TerpAuction) — not a NIP-15 signed event.
pub fn place_bid_wasm_hint(
    auction_id: &str,
    funds_denom: &str,
    funds_amount: &str,
) -> WasmExecuteHint {
    WasmExecuteHint {
        msg: serde_json::json!({
            "place_bid": {
                "auction_id": auction_id
            }
        }),
        funds_denom: Some(funds_denom.to_string()),
        funds_amount: Some(funds_amount.to_string()),
    }
}

pub fn purchase_wasm_hint(
    sku: &str,
    qty: u64,
    funds_denom: &str,
    funds_amount: &str,
) -> WasmExecuteHint {
    WasmExecuteHint {
        msg: serde_json::json!({
            "purchase": {
                "sku": sku,
                "qty": qty
            }
        }),
        funds_denom: Some(funds_denom.to_string()),
        funds_amount: Some(funds_amount.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_round_trip_dao_record() {
        let mesh = MeshDescriptorV1::new(
            "https://headstash.terp.network",
            vec!["wss://headstash.terp.network".into()],
        )
        .with_marketplace(MeshMarketplacePatch {
            fixed_price: Some("terp1fp".into()),
            auction: Some("terp1auc".into()),
            content_base: Some("https://hash.terp.network".into()),
            accepted_denom: Some("uterp".into()),
            min_buffer_blocks: Some(7),
            modes: Some(vec!["fixed".into(), "auction".into(), "buy_now".into()]),
        });

        assert_eq!(MeshDescriptorV1::record_key(), "mesh.v1");
        let raw = mesh.to_dao_record_json().unwrap();
        let back = MeshDescriptorV1::from_dao_record_json(&raw).unwrap();
        assert_eq!(back.query, "https://headstash.terp.network");
        assert_eq!(
            back.marketplace.as_ref().unwrap().min_buffer_blocks,
            Some(7)
        );
        assert_eq!(
            back.marketplace.as_ref().unwrap().fixed_price.as_deref(),
            Some("terp1fp")
        );
    }

    #[test]
    fn mesh_rejects_empty_query() {
        let mesh = MeshDescriptorV1::new("", vec![]);
        assert!(mesh.to_dao_record_json().is_err());
    }

    #[test]
    fn nip15_filter_kinds_match_spec() {
        let f = nip15_filter_heuristic(Nip15PlaneMode::Fixed, Some("terp1dao"));
        assert_eq!(f.kinds, Some(vec![30018]));
        assert!(f.t_tags.as_ref().unwrap().contains(&"fixed".to_string()));
        assert!(f.t_tags.as_ref().unwrap().contains(&"terp1dao".to_string()));

        let a = nip15_filter_heuristic(Nip15PlaneMode::Auction, None);
        assert_eq!(a.kinds, Some(vec![30020]));
    }

    #[test]
    fn place_bid_hint_shape() {
        let h = place_bid_wasm_hint("9", "uterp", "1000000");
        assert_eq!(h.msg["place_bid"]["auction_id"], "9");
        assert_eq!(h.funds_amount.as_deref(), Some("1000000"));
    }
}
