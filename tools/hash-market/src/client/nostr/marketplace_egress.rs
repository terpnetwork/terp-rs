//! Chain → Nostr NIP-15 egress for marketplace-mirror inventory.
//!
//! # Phase B — thin pure helpers (no Docker)
//!
//! ```text
//! ChainEventWatcher (wasm-*)
//!   → filter list_product / purchase / settle / cancel attrs
//!   → optional content-plane GET /content/{cid}
//!   → publish NIP-15 EVENT (kind 30017 stall | 30018 product | confirmation tags)
//! ```
//!
//! `ListProduct.content_cid` should be the bare sha256 from
//! [`crate::content::bind_listing_body`] / [`crate::content::bind_offchain_body`]
//! (BUD primary). Same TreeStore plane as calendar — no second blob store.
//!
//! Reverse direction (order parse → suggested purchase intent, no auto-execute):
//! [`super::marketplace_ingress`].
//!
//! Signature is **not** cryptographic — test / unsigned egress. Production should
//! re-sign with operator key before publish. Mirrors `calendar_egress.rs`.

use crate::client::nostr::calendar_egress::Nip01Event;
use crate::content::{content_path, parse_cid, CidKind};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

/// NIP-15 kinds used by marketplace-mirror egress.
pub const KIND_SET_STALL: u16 = 30017;
pub const KIND_SET_PRODUCT: u16 = 30018;
/// Informal purchase-confirmation kind (not standardized; tag-heavy 1-kind note).
pub const KIND_PURCHASE_NOTE: u16 = 1;

/// View of on-chain product listing needed for egress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductMetaView {
    pub sku: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_cid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stall_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stock: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seller: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<String>,
}

/// Parsed wasm event attributes from marketplace-mirror execute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketplaceChainAction {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sku: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qty: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_cid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buyer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settle_after: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<String>,
    pub height: u64,
}

/// Filter marketplace-mirror lifecycle actions from generic wasm attributes.
pub fn parse_marketplace_action(
    attrs: &[(String, String)],
    height: u64,
    contract: Option<&str>,
) -> Option<MarketplaceChainAction> {
    let mut action = None;
    let mut sku = None;
    let mut order_id = None;
    let mut qty = None;
    let mut status = None;
    let mut content_cid = None;
    let mut d_tag = None;
    let mut buyer = None;
    let mut settle_after = None;
    let mut contract_attr = contract.map(|s| s.to_string());

    for (k, v) in attrs {
        match k.as_str() {
            "action" => action = Some(v.clone()),
            "sku" => sku = Some(v.clone()),
            "order_id" => order_id = Some(v.clone()),
            "qty" => qty = Some(v.clone()),
            "status" => status = Some(v.clone()),
            "content_cid" => content_cid = Some(v.clone()),
            "d_tag" => d_tag = Some(v.clone()),
            "buyer" => buyer = Some(v.clone()),
            "settle_after" => settle_after = Some(v.clone()),
            "_contract_address" => contract_attr = Some(v.clone()),
            _ => {}
        }
    }

    let action = action?;
    match action.as_str() {
        "list_product" | "update_stock" | "purchase" | "settle" | "cancel" => {}
        _ => return None,
    }

    Some(MarketplaceChainAction {
        action,
        sku,
        order_id,
        qty,
        status,
        content_cid,
        d_tag,
        buyer,
        settle_after,
        contract: contract_attr,
        height,
    })
}

/// Build a NIP-15 kind:30018 SetProduct event from product meta + optional body.
///
/// Prefer bare sha256 `content_cid` (BUD primary). Body may be product JSON
/// fetched from the content plane.
pub fn product_to_nip15(
    meta: &ProductMetaView,
    body: Option<&[u8]>,
    pubkey: &str,
) -> Result<Nip01Event, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let content = if let Some(raw) = body {
        String::from_utf8_lossy(raw).into_owned()
    } else {
        serde_json::json!({
            "id": meta.d_tag.as_ref().unwrap_or(&meta.sku),
            "stall_id": meta.stall_id.as_deref().unwrap_or("default"),
            "name": meta.name.as_deref().unwrap_or(&meta.sku),
            "currency": meta.currency.as_deref().unwrap_or("uterp"),
            "price": meta.unit_price.as_deref().unwrap_or("0"),
            "quantity": meta.stock,
            "sku": meta.sku,
            "content_cid": meta.content_cid,
        })
        .to_string()
    };

    let mut tags = vec![
        vec!["t".into(), "marketplace".into()],
        vec!["t".into(), "hashmerchant".into()],
        vec![
            "d".into(),
            meta.d_tag.clone().unwrap_or_else(|| meta.sku.clone()),
        ],
        vec!["sku".into(), meta.sku.clone()],
    ];
    if let Some(cid) = &meta.content_cid {
        if !cid.is_empty() {
            tags.push(vec!["cid".into(), cid.clone()]);
            if let Ok(parsed) = parse_cid(cid) {
                if parsed.kind == CidKind::BudSha256 {
                    tags.push(vec!["bud".into(), parsed.value]);
                }
            }
        }
    }
    if let Some(seller) = &meta.seller {
        tags.push(vec!["p".into(), seller.clone()]);
    }
    if let Some(c) = &meta.contract {
        tags.push(vec!["contract".into(), c.clone()]);
    }
    if let Some(stall) = &meta.stall_id {
        tags.push(vec!["stall".into(), stall.clone()]);
    }

    let mut event = Nip01Event {
        id: String::new(),
        pubkey: pubkey.to_string(),
        created_at: now,
        kind: KIND_SET_PRODUCT as u32,
        tags,
        content,
        sig: "00".repeat(32),
    };
    event.id = deterministic_id(&event);
    Ok(event)
}

/// Build a confirmation / status note from a settle or purchase chain action.
///
/// Uses kind:1 with marketplace tags so relays can filter without requiring a
/// dedicated NIP-15 kind for order confirmations.
pub fn purchase_confirmation_to_nostr(action: &MarketplaceChainAction, pubkey: &str) -> Nip01Event {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let content = serde_json::json!({
        "action": action.action,
        "sku": action.sku,
        "order_id": action.order_id,
        "qty": action.qty,
        "status": action.status,
        "buyer": action.buyer,
        "settle_after": action.settle_after,
        "height": action.height,
        "contract": action.contract,
    })
    .to_string();

    let mut tags = vec![
        vec!["t".into(), "marketplace".into()],
        vec!["t".into(), "purchase".into()],
        vec!["action".into(), action.action.clone()],
        vec!["h".into(), action.height.to_string()],
    ];
    if let Some(sku) = &action.sku {
        tags.push(vec!["sku".into(), sku.clone()]);
    }
    if let Some(oid) = &action.order_id {
        tags.push(vec!["order_id".into(), oid.clone()]);
        tags.push(vec!["d".into(), format!("order-{}", oid)]);
    }
    if let Some(st) = &action.status {
        tags.push(vec!["status".into(), st.clone()]);
    }
    if let Some(c) = &action.contract {
        tags.push(vec!["contract".into(), c.clone()]);
    }
    if let Some(cid) = &action.content_cid {
        if !cid.is_empty() {
            tags.push(vec!["cid".into(), cid.clone()]);
        }
    }

    let mut event = Nip01Event {
        id: String::new(),
        pubkey: pubkey.to_string(),
        created_at: now,
        kind: KIND_PURCHASE_NOTE as u32,
        tags,
        content,
        sig: "00".repeat(32),
    };
    event.id = deterministic_id(&event);
    event
}

/// Route a chain action to the appropriate NIP-15 / note egress.
pub fn chain_action_to_nostr(
    action: &MarketplaceChainAction,
    product: Option<&ProductMetaView>,
    body: Option<&[u8]>,
    pubkey: &str,
) -> Result<Nip01Event, String> {
    match action.action.as_str() {
        "list_product" | "update_stock" => {
            let meta = if let Some(p) = product {
                p.clone()
            } else {
                ProductMetaView {
                    sku: action.sku.clone().unwrap_or_default(),
                    d_tag: action.d_tag.clone(),
                    content_cid: action.content_cid.clone(),
                    stall_id: None,
                    name: None,
                    unit_price: None,
                    stock: None,
                    currency: None,
                    seller: None,
                    contract: action.contract.clone(),
                }
            };
            product_to_nip15(&meta, body, pubkey)
        }
        "purchase" | "settle" | "cancel" => Ok(purchase_confirmation_to_nostr(action, pubkey)),
        other => Err(format!("unsupported marketplace action: {other}")),
    }
}

/// Content-plane path hint for a product cid (BUD / dual-index).
pub fn product_content_path(cid: &str) -> Option<String> {
    parse_cid(cid).ok().map(|p| content_path(&p))
}

fn deterministic_id(ev: &Nip01Event) -> String {
    let mut hasher = DefaultHasher::new();
    ev.pubkey.hash(&mut hasher);
    ev.created_at.hash(&mut hasher);
    ev.kind.hash(&mut hasher);
    ev.content.hash(&mut hasher);
    for t in &ev.tags {
        for s in t {
            s.hash(&mut hasher);
        }
    }
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_list_product_attrs() {
        let attrs = vec![
            ("action".into(), "list_product".into()),
            ("sku".into(), "tee".into()),
            ("content_cid".into(), "ab".repeat(32)),
            ("d_tag".into(), "product-tee".into()),
        ];
        let a = parse_marketplace_action(&attrs, 100, Some("contract1")).unwrap();
        assert_eq!(a.action, "list_product");
        assert_eq!(a.sku.as_deref(), Some("tee"));
        assert_eq!(a.height, 100);
    }

    #[test]
    fn parse_ignores_unknown_action() {
        let attrs = vec![("action".into(), "claim_rewards".into())];
        assert!(parse_marketplace_action(&attrs, 1, None).is_none());
    }

    #[test]
    fn product_event_kind_30018() {
        let meta = ProductMetaView {
            sku: "tee".into(),
            d_tag: Some("product-tee".into()),
            content_cid: Some("aa".repeat(32)),
            stall_id: Some("stall-1".into()),
            name: Some("Terp Tee".into()),
            unit_price: Some("100".into()),
            stock: Some(10),
            currency: Some("uterp".into()),
            seller: Some("npub1seller".into()),
            contract: Some("terp1mm".into()),
        };
        let ev = product_to_nip15(&meta, None, "pubkey1").unwrap();
        assert_eq!(ev.kind, KIND_SET_PRODUCT as u32);
        assert!(ev.tags.iter().any(|t| t.as_slice() == ["d", "product-tee"]));
        assert!(ev
            .tags
            .iter()
            .any(|t| t.first().map(|s| s.as_str()) == Some("cid")));
        assert!(!ev.id.is_empty());
    }

    #[test]
    fn settle_confirmation_note() {
        let action = MarketplaceChainAction {
            action: "settle".into(),
            sku: Some("tee".into()),
            order_id: Some("1".into()),
            qty: Some("2".into()),
            status: Some("settled".into()),
            content_cid: None,
            d_tag: None,
            buyer: Some("buyer1".into()),
            settle_after: None,
            contract: Some("terp1mm".into()),
            height: 12352,
        };
        let ev = purchase_confirmation_to_nostr(&action, "pubkey1");
        assert_eq!(ev.kind, KIND_PURCHASE_NOTE as u32);
        assert!(ev
            .tags
            .iter()
            .any(|t| t.as_slice() == ["status", "settled"]));
    }
}
