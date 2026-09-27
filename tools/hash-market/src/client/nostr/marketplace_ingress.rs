//! Nostr → chain **ingress stub** for marketplace-mirror.
//!
//! Parses NIP-15 / order-shaped Nostr event JSON into a **suggested** purchase
//! intent compatible with marketplace-mirror `ExecuteMsg::Purchase` fields.
//!
//! # Honest scope
//!
//! - **Does not** auto-execute chain messages
//! - **Does not** hold funds or talk to wasmd
//! - Returns [`PurchaseIntent`] for a demo relayer / operator to submit
//!   `Purchase { sku, qty }` + payment separately
//!
//! Pairs with [`super::marketplace_egress`] (chain → notes only).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Suggested on-chain purchase fields derived from a Nostr order-shaped event.
///
/// Maps to marketplace-mirror `ExecuteMsg::Purchase { sku, qty }` plus identity
/// metadata for the relayer. **Not** an execute message itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurchaseIntent {
    pub sku: String,
    pub qty: u64,
    /// Nostr pubkey of the order author (buyer identity off-chain).
    pub buyer_pubkey: String,
    /// Nostr event id (`id` field), when present.
    pub nostr_event_id: String,
}

/// Relayer policy for turning a [`PurchaseIntent`] into an on-chain `Purchase`.
///
/// Default is [`IngressExecutePolicy::Off`]. `On` still does **not** submit
/// a tx from this crate — it only authorizes a caller to proceed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum IngressExecutePolicy {
    #[default]
    Off,
    On,
}

impl IngressExecutePolicy {
    /// From marketplace-mirror `Config.auto_execute_ingress`.
    pub fn from_contract_flag(auto_execute_ingress: bool) -> Self {
        if auto_execute_ingress {
            Self::On
        } else {
            Self::Off
        }
    }
}

/// Errors from order-shaped parse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IngressError {
    #[error("invalid json: {0}")]
    InvalidJson(String),
    #[error("not a nostr event object")]
    NotEvent,
    #[error("missing or empty buyer pubkey")]
    MissingPubkey,
    #[error("missing sku (tag or content)")]
    MissingSku,
    #[error("missing or invalid qty")]
    MissingQty,
    #[error("qty must be > 0")]
    ZeroQty,
    #[error("garbage or unsupported order shape: {0}")]
    Reject(String),
    #[error("auto-execute ingress is off (governance / relayer policy)")]
    AutoExecuteOff,
}

/// Minimal NIP-01 event fields we care about for ingress.
#[derive(Debug, Clone, Deserialize)]
struct RawNostrEvent {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    pubkey: Option<String>,
    #[serde(default)]
    kind: Option<u64>,
    #[serde(default)]
    content: Option<Value>,
    #[serde(default)]
    tags: Option<Vec<Vec<String>>>,
}

/// Gate auto-execute: parse the event, then require [`IngressExecutePolicy::On`].
///
/// Returns [`IngressError::AutoExecuteOff`] when policy is off (the default).
/// Does not talk to wasmd.
pub fn authorize_auto_execute(
    event_json: &str,
    policy: IngressExecutePolicy,
) -> Result<PurchaseIntent, IngressError> {
    let intent = parse_purchase_intent(event_json)?;
    match policy {
        IngressExecutePolicy::On => Ok(intent),
        IngressExecutePolicy::Off => Err(IngressError::AutoExecuteOff),
    }
}

/// Parse Nostr event JSON (string or object) into a [`PurchaseIntent`].
///
/// Accepted shapes (first match wins for sku/qty):
/// 1. Tags: `["sku", …]`, `["qty"|"quantity", …]`, optional `["product_id"|"d", …]` as sku
/// 2. Content JSON object: `sku` / `product_id` / `id`, `qty` / `quantity`
/// 3. Content JSON NIP-15-style: `items: [{ "product_id"|"sku", "quantity"|"qty" }]`
///    (first item only — multi-item split is out of scope for this stub)
///
/// Rejects non-objects, empty required fields, and zero qty.
pub fn parse_purchase_intent(event_json: &str) -> Result<PurchaseIntent, IngressError> {
    let raw: Value = serde_json::from_str(event_json.trim())
        .map_err(|e| IngressError::InvalidJson(e.to_string()))?;
    parse_purchase_intent_value(&raw)
}

/// Same as [`parse_purchase_intent`] from an already-parsed JSON value.
pub fn parse_purchase_intent_value(raw: &Value) -> Result<PurchaseIntent, IngressError> {
    if !raw.is_object() {
        return Err(IngressError::NotEvent);
    }

    let ev: RawNostrEvent = serde_json::from_value(raw.clone())
        .map_err(|e| IngressError::InvalidJson(e.to_string()))?;

    let buyer_pubkey = ev
        .pubkey
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(IngressError::MissingPubkey)?
        .to_string();

    let nostr_event_id = ev
        .id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("")
        .to_string();

    // Kind is not decisive for this stub (NIP-15 orders often kind 4; demos use 1 / 30018).
    let _ = ev.kind;

    let tags = ev.tags.as_deref().unwrap_or(&[]);
    let mut sku = tag_value(tags, "sku")
        .or_else(|| tag_value(tags, "product_id"))
        .or_else(|| tag_value(tags, "d"));
    let mut qty = tag_u64(tags, "qty").or_else(|| tag_u64(tags, "quantity"));

    // Content may be a JSON object or a stringified object.
    let content_obj = match &ev.content {
        Some(Value::Object(_)) => ev.content.clone(),
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                serde_json::from_str::<Value>(t).ok()
            }
        }
        _ => None,
    };

    if let Some(obj) = content_obj.as_ref().filter(|v| v.is_object()) {
        // Prefer explicit product fields; `id` alone is ambiguous (order id vs product).
        if sku.is_none() {
            sku = str_field(obj, "sku").or_else(|| str_field(obj, "product_id"));
        }
        if qty.is_none() {
            qty = u64_field(obj, "qty").or_else(|| u64_field(obj, "quantity"));
        }
        // NIP-15 customer order: items[] — product identity lives here, not outer `id`.
        if let Some(items) = obj.get("items").and_then(|i| i.as_array()) {
            if items.is_empty() {
                return Err(IngressError::Reject("empty items[]".into()));
            }
            let first = &items[0];
            if !first.is_object() {
                return Err(IngressError::Reject("items[0] not object".into()));
            }
            if sku.is_none() {
                sku = str_field(first, "sku")
                    .or_else(|| str_field(first, "product_id"))
                    .or_else(|| str_field(first, "id"));
            }
            if qty.is_none() {
                qty = u64_field(first, "qty").or_else(|| u64_field(first, "quantity"));
            }
        } else if sku.is_none() {
            // Product-shaped content (no items[]): allow top-level id as sku.
            sku = str_field(obj, "id");
        }
    }

    let sku = sku
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or(IngressError::MissingSku)?;
    let qty = qty.ok_or(IngressError::MissingQty)?;
    if qty == 0 {
        return Err(IngressError::ZeroQty);
    }

    if nostr_event_id.is_empty() {
        return Err(IngressError::Reject("missing event id".into()));
    }

    Ok(PurchaseIntent {
        sku,
        qty,
        buyer_pubkey,
        nostr_event_id,
    })
}

fn tag_value(tags: &[Vec<String>], name: &str) -> Option<String> {
    tags.iter().find_map(|t| {
        if t.len() >= 2 && t[0] == name {
            let v = t[1].trim();
            if v.is_empty() {
                None
            } else {
                Some(v.to_string())
            }
        } else {
            None
        }
    })
}

fn tag_u64(tags: &[Vec<String>], name: &str) -> Option<u64> {
    tag_value(tags, name)?.parse().ok()
}

fn str_field(obj: &Value, key: &str) -> Option<String> {
    obj.get(key).and_then(|v| match v {
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

fn u64_field(obj: &Value, key: &str) -> Option<u64> {
    obj.get(key).and_then(|v| match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_order_from_tags_and_pubkey() {
        let json = r#"{
            "id": "evt-order-1",
            "pubkey": "npub_buyer_abc",
            "kind": 1,
            "created_at": 1700000000,
            "tags": [
                ["sku", "tee"],
                ["qty", "3"],
                ["t", "marketplace"]
            ],
            "content": "",
            "sig": "00"
        }"#;
        let intent = parse_purchase_intent(json).unwrap();
        assert_eq!(
            intent,
            PurchaseIntent {
                sku: "tee".into(),
                qty: 3,
                buyer_pubkey: "npub_buyer_abc".into(),
                nostr_event_id: "evt-order-1".into(),
            }
        );
    }

    #[test]
    fn parse_nip15_items_content() {
        let json = r#"{
            "id": "deadbeef",
            "pubkey": "buyerpk",
            "kind": 4,
            "tags": [["p", "merchant"]],
            "content": "{\"id\":\"ord-1\",\"type\":1,\"items\":[{\"product_id\":\"hoodie\",\"quantity\":2}]}"
        }"#;
        let intent = parse_purchase_intent(json).unwrap();
        assert_eq!(intent.sku, "hoodie");
        assert_eq!(intent.qty, 2);
        assert_eq!(intent.buyer_pubkey, "buyerpk");
        assert_eq!(intent.nostr_event_id, "deadbeef");
    }

    #[test]
    fn parse_content_object_sku_qty() {
        let json = serde_json::json!({
            "id": "abc123",
            "pubkey": "pk1",
            "kind": 30018,
            "content": { "sku": "mug", "qty": 1 },
            "tags": []
        });
        let intent = parse_purchase_intent_value(&json).unwrap();
        assert_eq!(intent.sku, "mug");
        assert_eq!(intent.qty, 1);
    }

    #[test]
    fn reject_garbage() {
        assert!(matches!(
            parse_purchase_intent("not-json"),
            Err(IngressError::InvalidJson(_))
        ));
        assert!(matches!(
            parse_purchase_intent("[]"),
            Err(IngressError::NotEvent)
        ));
        assert!(matches!(
            parse_purchase_intent(r#"{"id":"x","content":"hi"}"#),
            Err(IngressError::MissingPubkey)
        ));
        assert!(matches!(
            parse_purchase_intent(r#"{"id":"x","pubkey":"pk","tags":[],"content":{}}"#),
            Err(IngressError::MissingSku)
        ));
        assert!(matches!(
            parse_purchase_intent(
                r#"{"id":"x","pubkey":"pk","tags":[["sku","tee"],["qty","0"]],"content":""}"#
            ),
            Err(IngressError::ZeroQty)
        ));
        assert!(matches!(
            parse_purchase_intent(
                r#"{"pubkey":"pk","tags":[["sku","tee"],["qty","1"]],"content":""}"#
            ),
            Err(IngressError::Reject(_))
        ));
    }

    #[test]
    fn product_id_tag_fallback() {
        let json = r#"{
            "id": "e1",
            "pubkey": "buyer",
            "tags": [["product_id", "sku-z"], ["quantity", "5"]],
            "content": null
        }"#;
        let intent = parse_purchase_intent(json).unwrap();
        assert_eq!(intent.sku, "sku-z");
        assert_eq!(intent.qty, 5);
    }

    #[test]
    fn auto_execute_defaults_off() {
        let json = r#"{
            "id": "e1",
            "pubkey": "buyer",
            "tags": [["sku", "tee"], ["qty", "1"]],
            "content": ""
        }"#;
        assert!(matches!(
            authorize_auto_execute(json, IngressExecutePolicy::Off),
            Err(IngressError::AutoExecuteOff)
        ));
        assert!(matches!(
            authorize_auto_execute(json, IngressExecutePolicy::from_contract_flag(false)),
            Err(IngressError::AutoExecuteOff)
        ));
        let intent = authorize_auto_execute(json, IngressExecutePolicy::On).unwrap();
        assert_eq!(intent.sku, "tee");
    }
}
