//! Chain → Nostr NIP-52 egress for dao-calendar events.
//!
//! # Phase B — B2 production-shaped bridge
//!
//! ```text
//! ChainEventWatcher (wasm-*)
//!   → filter create_event / update_event / cancel_event attrs
//!   → load body (on-chain e JSON **or** content-plane GET /content/{cid})
//!   → publish NIP-52 EVENT (kind 31922|31923) to mesh relays
//! ```
//!
//! Pure conversion lives here (no Docker). Relay publish uses a caller-supplied
//! sink so tests can use mock or `ict_rs::nostr::NostrClient`.

use crate::content::calendar::{is_nip52_event_kind, KIND_DATE_BASED, KIND_TIME_BASED};
use crate::content::{content_path, parse_cid, CidKind};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

/// Wire-shape Nostr event (NIP-01) independent of ict-rs / harness crates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nip01Event {
    pub id: String,
    pub pubkey: String,
    pub created_at: i64,
    pub kind: u32,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub sig: String,
}

/// View of dao-calendar metadata needed for egress (on- or off-chain).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarMetaView {
    pub on_chain: bool,
    /// Full NIP-52 event JSON when on-chain (UTF-8 of `MetadataExt.e` when JSON-encoded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub e_json: Option<String>,
    /// Content-plane cid when off-chain (prefer bare sha256).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cid: Option<String>,
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

/// Parsed wasm event attributes from dao-calendar execute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarChainAction {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub e_d: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<String>,
    pub height: u64,
}

/// Filter dao-calendar lifecycle actions from generic wasm attributes.
pub fn parse_calendar_action(
    attrs: &[(String, String)],
    height: u64,
    contract: Option<&str>,
) -> Option<CalendarChainAction> {
    let mut action = None;
    let mut e_d = None;
    let mut d = None;
    let mut contract_attr = contract.map(|s| s.to_string());

    for (k, v) in attrs {
        match k.as_str() {
            "action" => action = Some(v.clone()),
            "e_d" => e_d = Some(v.clone()),
            "d" => d = Some(v.clone()),
            "_contract_address" => contract_attr = Some(v.clone()),
            _ => {}
        }
    }

    let action = action?;
    match action.as_str() {
        "create_event" | "update_event" | "cancel_event" | "create_calendar" => {}
        _ => return None,
    }

    Some(CalendarChainAction {
        action,
        e_d,
        d,
        contract: contract_attr,
        height,
    })
}

/// Build a NIP-52 EVENT from resolved calendar body + meta.
///
/// * `body` — for on-chain: JSON of event fields; for off-chain: bytes fetched
///   from content plane (`GET /content/{cid}`). If body is itself a full Nostr
///   event JSON with `kind`/`tags`/`content`, those fields are preferred.
/// * `pubkey` — publisher pubkey (usually mesh/operator or event author).
///
/// Signature is **not** cryptographic — test / unsigned egress. Production
/// should re-sign with operator key before publish.
pub fn metadata_to_nip52_event(
    meta: &CalendarMetaView,
    body: Option<&[u8]>,
    action: Option<&CalendarChainAction>,
    pubkey: &str,
) -> Result<Nip01Event, String> {
    if !is_nip52_event_kind(meta.kind) && meta.kind != 0 {
        // allow kind 0 only when body embeds kind
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let pubkey = meta
        .author_pubkey
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(pubkey);

    // Prefer full Nostr event JSON if body looks like one.
    if let Some(raw) = body {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(raw) {
            if v.get("kind").is_some() && v.get("tags").is_some() && v.get("content").is_some() {
                return nip01_from_json_value(&v, pubkey, now, meta, action);
            }
        }
    }

    // On-chain e_json may be CalendarEventMetadata or full event.
    if meta.on_chain {
        if let Some(ej) = meta.e_json.as_deref() {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(ej) {
                if v.get("kind").is_some() && v.get("tags").is_some() {
                    return nip01_from_json_value(&v, pubkey, now, meta, action);
                }
                // CalendarEventMetadata-shaped: synthesize tags from known fields
                return Ok(synthesize_from_metadata_json(
                    &v,
                    meta.kind,
                    pubkey,
                    now,
                    meta,
                    action,
                    Some(ej),
                ));
            }
        }
    }

    // Off-chain: body is event description JSON; cid is pointer.
    let content = if let Some(raw) = body {
        String::from_utf8_lossy(raw).into_owned()
    } else if let Some(cid) = &meta.cid {
        // Publish pointer content so consumers can still resolve via content plane.
        serde_json::json!({
            "off_chain": true,
            "cid": cid,
            "content_path": parse_cid(cid).map(|p| content_path(&p)).unwrap_or_default(),
        })
        .to_string()
    } else {
        return Err("off-chain egress needs body or cid".into());
    };

    let kind = if is_nip52_event_kind(meta.kind) {
        meta.kind as u32
    } else {
        KIND_TIME_BASED as u32
    };

    let mut tags = base_tags(meta, action);
    if let Some(cid) = &meta.cid {
        tags.push(vec!["cid".into(), cid.clone()]);
        if let Ok(parsed) = parse_cid(cid) {
            if parsed.kind == CidKind::BudSha256 {
                tags.push(vec!["bud".into(), parsed.value]);
            }
        }
    }
    if let Some(d) = &meta.d_tag {
        tags.push(vec!["d".into(), d.clone()]);
    }

    let mut event = Nip01Event {
        id: String::new(),
        pubkey: pubkey.to_string(),
        created_at: now,
        kind,
        tags,
        content,
        sig: "00".repeat(32),
    };
    event.id = deterministic_id(&event);
    Ok(event)
}

/// Convenience: off-chain meta + resolved body → NIP-52 event.
pub fn offchain_to_nip52(
    cid: &str,
    kind: u16,
    body: &[u8],
    d_tag: Option<&str>,
    calendar_d: Option<&str>,
    pubkey: &str,
) -> Result<Nip01Event, String> {
    let meta = CalendarMetaView {
        on_chain: false,
        e_json: None,
        cid: Some(cid.to_string()),
        kind,
        d_tag: d_tag.map(|s| s.to_string()),
        calendar_d: calendar_d.map(|s| s.to_string()),
        author_pubkey: Some(pubkey.to_string()),
        nostr_e_d: None,
    };
    metadata_to_nip52_event(&meta, Some(body), None, pubkey)
}

/// On-chain meta (JSON `e`) → NIP-52 event.
pub fn onchain_to_nip52(
    e_json: &str,
    kind: u16,
    d_tag: Option<&str>,
    pubkey: &str,
) -> Result<Nip01Event, String> {
    let meta = CalendarMetaView {
        on_chain: true,
        e_json: Some(e_json.to_string()),
        cid: None,
        kind,
        d_tag: d_tag.map(|s| s.to_string()),
        calendar_d: None,
        author_pubkey: Some(pubkey.to_string()),
        nostr_e_d: None,
    };
    metadata_to_nip52_event(&meta, None, None, pubkey)
}

/// Legacy bridge: generic chain attrs → kind-31922 calendar-shaped event.
///
/// Prefer [`metadata_to_nip52_event`] when MetadataExt is available.
pub fn chain_attrs_to_nostr(
    attrs: &[(String, String)],
    height: u64,
    action: Option<&str>,
    contract: Option<&str>,
    pubkey: &str,
) -> Nip01Event {
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

    let mut tags = vec![
        vec!["t".into(), "chain-event".into()],
        vec!["t".into(), "dao-calendar".into()],
        vec!["h".into(), height.to_string()],
    ];
    if let Some(a) = action {
        tags.push(vec!["action".into(), a.to_string()]);
    }
    if let Some(c) = contract {
        tags.push(vec!["contract".into(), c.to_string()]);
    }

    let mut event = Nip01Event {
        id: String::new(),
        pubkey: pubkey.to_string(),
        created_at: now,
        kind: KIND_DATE_BASED as u32,
        tags,
        content,
        sig: "00".repeat(32),
    };
    event.id = deterministic_id(&event);
    event
}

fn base_tags(meta: &CalendarMetaView, action: Option<&CalendarChainAction>) -> Vec<Vec<String>> {
    let mut tags = vec![
        vec!["t".into(), "dao-calendar".into()],
        vec!["t".into(), "calendar".into()],
    ];
    if let Some(d) = &meta.d_tag {
        tags.push(vec!["d".into(), d.clone()]);
    }
    if let Some(cal) = &meta.calendar_d {
        tags.push(vec!["calendar".into(), cal.clone()]);
        tags.push(vec!["t".into(), cal.clone()]);
    }
    if let Some(a) = action {
        tags.push(vec!["action".into(), a.action.clone()]);
        tags.push(vec!["h".into(), a.height.to_string()]);
        if let Some(ed) = &a.e_d {
            tags.push(vec!["e_d".into(), ed.clone()]);
        }
        if let Some(c) = &a.contract {
            tags.push(vec!["contract".into(), c.clone()]);
        }
    }
    tags
}

fn nip01_from_json_value(
    v: &serde_json::Value,
    pubkey: &str,
    now: i64,
    meta: &CalendarMetaView,
    action: Option<&CalendarChainAction>,
) -> Result<Nip01Event, String> {
    let kind = v
        .get("kind")
        .and_then(|k| k.as_u64())
        .map(|k| k as u32)
        .unwrap_or(meta.kind as u32);
    let content = v
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    let mut tags: Vec<Vec<String>> = v
        .get("tags")
        .and_then(|t| serde_json::from_value(t.clone()).ok())
        .unwrap_or_default();
    for t in base_tags(meta, action) {
        if !tags.iter().any(|x| x == &t) {
            tags.push(t);
        }
    }
    if let Some(cid) = &meta.cid {
        if !tags
            .iter()
            .any(|t| t.first().map(|s| s.as_str()) == Some("cid"))
        {
            tags.push(vec!["cid".into(), cid.clone()]);
        }
    }
    let pk = v
        .get("pubkey")
        .and_then(|p| p.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(pubkey);
    let created_at = v.get("created_at").and_then(|c| c.as_i64()).unwrap_or(now);
    let id = v
        .get("id")
        .and_then(|i| i.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let sig = v
        .get("sig")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let mut event = Nip01Event {
        id: id.unwrap_or_default(),
        pubkey: pk.to_string(),
        created_at,
        kind,
        tags,
        content,
        sig: if sig.is_empty() { "00".repeat(32) } else { sig },
    };
    if event.id.is_empty() {
        event.id = deterministic_id(&event);
    }
    Ok(event)
}

fn synthesize_from_metadata_json(
    v: &serde_json::Value,
    kind: u16,
    pubkey: &str,
    now: i64,
    meta: &CalendarMetaView,
    action: Option<&CalendarChainAction>,
    raw_content: Option<&str>,
) -> Nip01Event {
    let title = v
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("calendar-event");
    let d_tag = v
        .get("d_tag")
        .or_else(|| v.get("d"))
        .and_then(|d| d.as_str())
        .or(meta.d_tag.as_deref());
    let content = v
        .get("content")
        .and_then(|c| c.as_str())
        .or(raw_content)
        .unwrap_or(title)
        .to_string();

    let mut tags = base_tags(meta, action);
    if let Some(d) = d_tag {
        tags.push(vec!["d".into(), d.to_string()]);
    }
    tags.push(vec!["title".into(), title.to_string()]);
    if let Some(start) = v.get("start_time").and_then(|s| s.as_u64()) {
        tags.push(vec!["start".into(), start.to_string()]);
    }
    if let Some(end) = v.get("end_time").and_then(|s| s.as_u64()) {
        tags.push(vec!["end".into(), end.to_string()]);
    }

    let k = if is_nip52_event_kind(kind) {
        kind as u32
    } else {
        KIND_TIME_BASED as u32
    };

    let mut event = Nip01Event {
        id: String::new(),
        pubkey: pubkey.to_string(),
        created_at: now,
        kind: k,
        tags,
        content,
        sig: "00".repeat(32),
    };
    event.id = deterministic_id(&event);
    event
}

fn deterministic_id(ev: &Nip01Event) -> String {
    let mut hasher = DefaultHasher::new();
    ev.pubkey.hash(&mut hasher);
    ev.created_at.hash(&mut hasher);
    ev.kind.hash(&mut hasher);
    ev.content.hash(&mut hasher);
    for t in &ev.tags {
        for p in t {
            p.hash(&mut hasher);
        }
    }
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{bind_offchain_event, resolve_local, KIND_TIME_BASED};
    use crate::store::TreeStore;

    #[test]
    fn parse_create_event_attrs() {
        let attrs = vec![
            ("action".into(), "create_event".into()),
            ("d".into(), "cal/1".into()),
            ("e_d".into(), "evt/cal/1/1".into()),
            ("_contract_address".into(), "terp1cal".into()),
        ];
        let a = parse_calendar_action(&attrs, 99, None).unwrap();
        assert_eq!(a.action, "create_event");
        assert_eq!(a.e_d.as_deref(), Some("evt/cal/1/1"));
        assert_eq!(a.contract.as_deref(), Some("terp1cal"));
        assert!(
            parse_calendar_action(&[("action".into(), "transfer_nft".into())], 1, None).is_none()
        );
    }

    #[test]
    fn offchain_bind_then_egress() {
        let dir = std::env::temp_dir().join(format!("hm-egress-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = TreeStore::open(dir.join("trees")).unwrap();

        let body = br#"{"title":"Town Hall","content":"discuss","start_time":1700000000,"end_time":1700003600,"d_tag":"town-1"}"#;
        let (bind, meta) = bind_offchain_event(&store, body, KIND_TIME_BASED).unwrap();
        let resolved = resolve_local(&store, &meta.cid).unwrap();

        let action = CalendarChainAction {
            action: "create_event".into(),
            e_d: Some("evt/cal/1/1".into()),
            d: Some("cal/1".into()),
            contract: Some("terp1calendar".into()),
            height: 42,
        };
        let view = CalendarMetaView {
            on_chain: false,
            e_json: None,
            cid: Some(meta.cid.clone()),
            kind: meta.kind,
            d_tag: Some("town-1".into()),
            calendar_d: Some("cal/1".into()),
            author_pubkey: Some("pub_abc".into()),
            nostr_e_d: None,
        };
        let ev =
            metadata_to_nip52_event(&view, Some(&resolved), Some(&action), "fallback_pk").unwrap();
        assert_eq!(ev.kind, KIND_TIME_BASED as u32);
        assert_eq!(ev.pubkey, "pub_abc");
        assert!(ev.content.contains("Town Hall"));
        assert!(ev
            .tags
            .iter()
            .any(|t| t == &["cid".to_string(), bind.chain_cid.clone()]));
        assert!(ev
            .tags
            .iter()
            .any(|t| t.first().map(|s| s.as_str()) == Some("bud")));
        assert!(ev
            .tags
            .iter()
            .any(|t| t == &["t".to_string(), "dao-calendar".to_string()]));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn onchain_metadata_synthesize() {
        let e = r#"{"d_tag":"x","title":"OnChain","content":"body","start_time":1,"end_time":2,"event_type":"TimeBased"}"#;
        let ev = onchain_to_nip52(e, KIND_TIME_BASED, Some("x"), "pk").unwrap();
        assert_eq!(ev.kind, KIND_TIME_BASED as u32);
        assert!(ev
            .tags
            .iter()
            .any(|t| t == &["title".to_string(), "OnChain".to_string()]));
    }

    #[test]
    fn chain_attrs_legacy_bridge() {
        let attrs = vec![("action".into(), "create_event".into())];
        let ev = chain_attrs_to_nostr(&attrs, 7, Some("create_event"), Some("terp1c"), "pk");
        assert_eq!(ev.kind, KIND_DATE_BASED as u32);
        assert!(ev.content.contains("\"chain_height\":7"));
    }
}
