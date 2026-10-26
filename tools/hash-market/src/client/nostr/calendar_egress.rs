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
//!
//! Every builder takes a caller-supplied [`NostrSigner`] and returns a fully
//! signed NIP-01 event (sha256 id + BIP-340 sig). Every calendar event is checked
//! with [`validate_nip52_calendar_event`] before it is returned.

use crate::content::calendar::{KIND_DATE_BASED, KIND_TIME_BASED};
use crate::content::{parse_cid, CidKind};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub use super::nip01::{is_valid_pubkey_hex, Nip01Event, NostrSigner};

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

/// Build a signed NIP-52 EVENT from resolved calendar body + meta.
///
/// * `body` — for on-chain: JSON of event fields; for off-chain: bytes fetched
///   from content plane (`GET /content/{cid}`).
///   - A full Nostr event JSON whose id + BIP-340 sig verify is passed through
///     unchanged (author-signed; must still be valid NIP-52).
///   - An unsigned/unverifiable event template (`kind`/`tags`/`content`) is
///     re-built and signed by `signer`.
///   - Otherwise the body is calendar metadata JSON
///     (`title`, `start_time`, `end_time`, `d_tag`, `content`, …).
/// * `signer` — publishing key (operator / mesh). Event `pubkey` is always the
///   signer's x-only key. `meta.author_pubkey`, when it is a valid x-only hex key
///   distinct from the signer, is attached as `["p", <author>, "", "author"]`.
pub fn metadata_to_nip52_event(
    meta: &CalendarMetaView,
    body: Option<&[u8]>,
    action: Option<&CalendarChainAction>,
    signer: &NostrSigner,
) -> Result<Nip01Event, String> {
    let now = now_secs();

    let mut source: Option<serde_json::Value> = None;
    if let Some(raw) = body {
        source = serde_json::from_slice::<serde_json::Value>(raw)
            .ok()
            .filter(|v| v.is_object());
    }
    if source.is_none() && meta.on_chain {
        if let Some(ej) = meta.e_json.as_deref() {
            source = serde_json::from_str::<serde_json::Value>(ej)
                .ok()
                .filter(|v| v.is_object());
        }
    }

    if let Some(v) = &source {
        if v.get("kind").is_some() && v.get("tags").is_some() {
            return from_event_template(v, meta, action, signer, now);
        }
    }

    let kind = nip52_kind(meta.kind as u32)?;
    let fields = match &source {
        Some(v) => v.clone(),
        None if body.is_some() => {
            return Err("NIP-52 egress: body is not calendar metadata JSON (need title/start)".into())
        }
        None => return Err("NIP-52 egress: need body or on-chain e_json with title/start".into()),
    };

    let d = meta
        .d_tag
        .clone()
        .or_else(|| str_field(&fields, &["d_tag", "d"]))
        .filter(|s| !s.is_empty())
        .ok_or("NIP-52 requires a d tag (meta.d_tag or body d_tag)")?;
    let title = str_field(&fields, &["title", "name"])
        .filter(|s| !s.is_empty())
        .ok_or("NIP-52 requires a title")?;
    let start = time_field(&fields, &["start_time", "start"], kind)?
        .ok_or("NIP-52 requires a start (start_time)")?;
    let end = time_field(&fields, &["end_time", "end"], kind)?;
    let content = str_field(&fields, &["content", "description"]).unwrap_or_default();

    let mut tags: Vec<Vec<String>> = vec![
        vec!["d".into(), d],
        vec!["title".into(), title],
        vec!["start".into(), start],
    ];
    if let Some(e) = end {
        tags.push(vec!["end".into(), e]);
    }
    if kind == KIND_TIME_BASED as u32 {
        for k in ["start_tzid", "end_tzid"] {
            if let Some(tz) = str_field(&fields, &[k]).filter(|s| !s.is_empty()) {
                tags.push(vec![k.into(), tz]);
            }
        }
    }
    if let Some(loc) = str_field(&fields, &["location"]).filter(|s| !s.is_empty()) {
        tags.push(vec!["location".into(), loc]);
    }
    for t in context_tags(meta, action, signer) {
        push_unique(&mut tags, t);
    }

    let ev = signer.sign_event(now, kind, tags, content)?;
    validate_nip52_calendar_event(&ev)?;
    Ok(ev)
}

/// Convenience: off-chain meta + resolved body → signed NIP-52 event.
pub fn offchain_to_nip52(
    cid: &str,
    kind: u16,
    body: &[u8],
    d_tag: Option<&str>,
    calendar_d: Option<&str>,
    signer: &NostrSigner,
) -> Result<Nip01Event, String> {
    let meta = CalendarMetaView {
        on_chain: false,
        e_json: None,
        cid: Some(cid.to_string()),
        kind,
        d_tag: d_tag.map(|s| s.to_string()),
        calendar_d: calendar_d.map(|s| s.to_string()),
        author_pubkey: None,
        nostr_e_d: None,
    };
    metadata_to_nip52_event(&meta, Some(body), None, signer)
}

/// On-chain meta (JSON `e`) → signed NIP-52 event.
pub fn onchain_to_nip52(
    e_json: &str,
    kind: u16,
    d_tag: Option<&str>,
    signer: &NostrSigner,
) -> Result<Nip01Event, String> {
    let meta = CalendarMetaView {
        on_chain: true,
        e_json: Some(e_json.to_string()),
        cid: None,
        kind,
        d_tag: d_tag.map(|s| s.to_string()),
        calendar_d: None,
        author_pubkey: None,
        nostr_e_d: None,
    };
    metadata_to_nip52_event(&meta, None, None, signer)
}

/// Legacy bridge: generic chain attrs → signed, valid kind-31922 (date-based) event.
///
/// `d = chain/{contract}/{height}/{action}`, `title = action`, `start = today (UTC)`.
/// Prefer [`metadata_to_nip52_event`] when MetadataExt is available.
pub fn chain_attrs_to_nostr(
    attrs: &[(String, String)],
    height: u64,
    action: Option<&str>,
    contract: Option<&str>,
    signer: &NostrSigner,
) -> Result<Nip01Event, String> {
    let content = serde_json::json!({
        "chain_height": height,
        "action": action,
        "contract": contract,
        "attributes": attrs,
    })
    .to_string();

    let now = now_secs();
    let action_s = action.unwrap_or("chain-event");
    let mut tags = vec![
        vec![
            "d".into(),
            format!("chain/{}/{}/{}", contract.unwrap_or("unknown"), height, action_s),
        ],
        vec!["title".into(), action_s.to_string()],
        vec!["start".into(), unix_to_ymd(now)?],
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
    let ev = signer.sign_event(now, KIND_DATE_BASED as u32, tags, content)?;
    validate_nip52_calendar_event(&ev)?;
    Ok(ev)
}

/// Validate NIP-52 calendar-event structure (kinds 31922 / 31923):
/// exactly one non-empty `d`, `title`, `start`; at most one `end`;
/// 31922 dates are ISO `YYYY-MM-DD` (end exclusive, > start);
/// 31923 times are unix seconds (end > start); tzid tags only on 31923.
pub fn validate_nip52_calendar_event(ev: &Nip01Event) -> Result<(), String> {
    let kind = nip52_kind(ev.kind)?;
    let one = |name: &str, required: bool| -> Result<Option<String>, String> {
        let vals: Vec<&Vec<String>> = ev
            .tags
            .iter()
            .filter(|t| t.first().map(|s| s.as_str()) == Some(name))
            .collect();
        match vals.len() {
            0 if required => Err(format!("NIP-52: missing `{name}` tag")),
            0 => Ok(None),
            1 => match vals[0].get(1) {
                Some(v) if !v.is_empty() => Ok(Some(v.clone())),
                _ => Err(format!("NIP-52: `{name}` tag has empty value")),
            },
            n => Err(format!("NIP-52: `{name}` tag must appear once (found {n})")),
        }
    };
    one("d", true)?;
    one("title", true)?;
    let start = one("start", true)?.unwrap();
    let end = one("end", false)?;
    if kind == KIND_DATE_BASED as u32 {
        let s = parse_ymd(&start).ok_or("NIP-52 31922: start must be YYYY-MM-DD")?;
        if let Some(e) = end {
            let e = parse_ymd(&e).ok_or("NIP-52 31922: end must be YYYY-MM-DD")?;
            if e <= s {
                return Err("NIP-52 31922: end (exclusive) must be after start".into());
            }
        }
        for tz in ["start_tzid", "end_tzid"] {
            if ev.tags.iter().any(|t| t.first().map(|s| s.as_str()) == Some(tz)) {
                return Err(format!("NIP-52 31922: `{tz}` only valid on 31923"));
            }
        }
    } else {
        let s = parse_unix(&start).ok_or("NIP-52 31923: start must be unix seconds")?;
        if let Some(e) = end {
            let e = parse_unix(&e).ok_or("NIP-52 31923: end must be unix seconds")?;
            if e <= s {
                return Err("NIP-52 31923: end must be after start".into());
            }
        }
        one("start_tzid", false)?;
        one("end_tzid", false)?;
    }
    Ok(())
}

fn nip52_kind(kind: u32) -> Result<u32, String> {
    if kind == KIND_DATE_BASED as u32 || kind == KIND_TIME_BASED as u32 {
        Ok(kind)
    } else {
        Err(format!(
            "kind {kind} is not a NIP-52 calendar event (want {KIND_DATE_BASED} or {KIND_TIME_BASED})"
        ))
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn push_unique(tags: &mut Vec<Vec<String>>, t: Vec<String>) {
    if !tags.iter().any(|x| x == &t) {
        tags.push(t);
    }
}

/// Non-structural context tags (never d/title/start/end).
fn context_tags(
    meta: &CalendarMetaView,
    action: Option<&CalendarChainAction>,
    signer: &NostrSigner,
) -> Vec<Vec<String>> {
    let mut tags = vec![
        vec!["t".into(), "dao-calendar".into()],
        vec!["t".into(), "calendar".into()],
    ];
    if let Some(cal) = &meta.calendar_d {
        tags.push(vec!["calendar".into(), cal.clone()]);
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
    if let Some(cid) = &meta.cid {
        tags.push(vec!["cid".into(), cid.clone()]);
        if let Ok(parsed) = parse_cid(cid) {
            if parsed.kind == CidKind::BudSha256 {
                tags.push(vec!["bud".into(), parsed.value]);
            }
        }
    }
    if let Some(author) = meta.author_pubkey.as_deref() {
        if is_valid_pubkey_hex(author) && author != signer.pubkey_hex() {
            tags.push(vec!["p".into(), author.to_string(), String::new(), "author".into()]);
        }
    }
    tags
}

/// Full event JSON: pass through if its id + sig verify, else rebuild + sign.
fn from_event_template(
    v: &serde_json::Value,
    meta: &CalendarMetaView,
    action: Option<&CalendarChainAction>,
    signer: &NostrSigner,
    now: i64,
) -> Result<Nip01Event, String> {
    if let Ok(ev) = serde_json::from_value::<Nip01Event>(v.clone()) {
        if ev.verify().is_ok() {
            validate_nip52_calendar_event(&ev)?;
            return Ok(ev);
        }
    }
    let kind = v
        .get("kind")
        .and_then(|k| k.as_u64())
        .ok_or("event template: kind must be an integer")?;
    let kind = nip52_kind(u32::try_from(kind).map_err(|_| "kind out of range")?)?;
    let content = v
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    let mut tags: Vec<Vec<String>> = v
        .get("tags")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| format!("event template: tags must be string arrays: {e}"))?
        .unwrap_or_default();
    if let Some(d) = &meta.d_tag {
        if !tags.iter().any(|t| t.first().map(|s| s.as_str()) == Some("d")) {
            tags.insert(0, vec!["d".into(), d.clone()]);
        }
    }
    for t in context_tags(meta, action, signer) {
        push_unique(&mut tags, t);
    }
    let created_at = v
        .get("created_at")
        .and_then(|c| c.as_i64())
        .filter(|c| *c >= 0)
        .unwrap_or(now);
    let ev = signer.sign_event(created_at, kind, tags, content)?;
    validate_nip52_calendar_event(&ev)?;
    Ok(ev)
}

fn str_field(v: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| v.get(*k).and_then(|x| x.as_str()).map(|s| s.to_string()))
}

/// Read a start/end field and render it in the kind's NIP-52 format.
/// Accepts unix seconds (number or numeric string); 31922 also accepts `YYYY-MM-DD`.
fn time_field(v: &serde_json::Value, keys: &[&str], kind: u32) -> Result<Option<String>, String> {
    let Some(raw) = keys.iter().find_map(|k| v.get(*k)).filter(|x| !x.is_null()) else {
        return Ok(None);
    };
    let unix = match raw {
        serde_json::Value::Number(n) => n.as_i64(),
        serde_json::Value::String(s) => parse_unix(s),
        _ => None,
    };
    if kind == KIND_DATE_BASED as u32 {
        if let Some(u) = unix {
            return unix_to_ymd(u).map(Some);
        }
        if let Some(s) = raw.as_str() {
            if parse_ymd(s).is_some() {
                return Ok(Some(s.to_string()));
            }
        }
        Err(format!("NIP-52 31922: `{}` must be unix seconds or YYYY-MM-DD", keys[0]))
    } else {
        unix.filter(|u| *u >= 0)
            .map(|u| Some(u.to_string()))
            .ok_or_else(|| format!("NIP-52 31923: `{}` must be unix seconds", keys[0]))
    }
}

fn parse_unix(s: &str) -> Option<i64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<i64>().ok()
}

/// Days since 1970-01-01 for a proleptic-Gregorian civil date (H. Hinnant).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Unix seconds → `YYYY-MM-DD` (UTC).
fn unix_to_ymd(secs: i64) -> Result<String, String> {
    if secs < 0 {
        return Err("negative timestamp".into());
    }
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    Ok(format!("{y:04}-{m:02}-{d:02}"))
}

/// Strict `YYYY-MM-DD` → day number (rejects impossible dates).
fn parse_ymd(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let digits = |r: std::ops::Range<usize>| -> Option<i64> {
        let p = &s[r];
        p.bytes().all(|c| c.is_ascii_digit()).then(|| p.parse().ok()).flatten()
    };
    let (y, m, d) = (digits(0..4)?, digits(5..7)? as u32, digits(8..10)? as u32);
    if !(1..=12).contains(&m) || d == 0 || d > 31 {
        return None;
    }
    let days = days_from_civil(y, m, d);
    (civil_from_days(days) == (y, m, d)).then_some(days)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{bind_offchain_event, resolve_local, KIND_TIME_BASED};
    use crate::store::TreeStore;

    fn tag<'a>(ev: &'a Nip01Event, name: &str) -> Vec<&'a Vec<String>> {
        ev.tags
            .iter()
            .filter(|t| t.first().map(|s| s.as_str()) == Some(name))
            .collect()
    }

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
        let signer = NostrSigner::generate();
        let author = NostrSigner::generate().pubkey_hex();
        let dir = std::env::temp_dir().join(format!("hm-egress-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = TreeStore::open(dir.join("trees")).unwrap();

        let body = br#"{"title":"Town Hall","content":"discuss","start_time":1700000000,"end_time":1700003600,"d_tag":"town-1","start_tzid":"Africa/Dakar"}"#;
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
            author_pubkey: Some(author.clone()),
            nostr_e_d: None,
        };
        let ev = metadata_to_nip52_event(&view, Some(&resolved), Some(&action), &signer).unwrap();
        ev.verify().unwrap();
        validate_nip52_calendar_event(&ev).unwrap();
        assert_eq!(ev.kind, KIND_TIME_BASED as u32);
        assert_eq!(ev.pubkey, signer.pubkey_hex());
        assert_eq!(ev.content, "discuss");
        assert_eq!(tag(&ev, "d").len(), 1, "exactly one d tag");
        assert_eq!(tag(&ev, "title")[0][1], "Town Hall");
        assert_eq!(tag(&ev, "start")[0][1], "1700000000");
        assert_eq!(tag(&ev, "end")[0][1], "1700003600");
        assert_eq!(tag(&ev, "start_tzid")[0][1], "Africa/Dakar");
        assert!(ev
            .tags
            .iter()
            .any(|t| t == &vec!["p".to_string(), author.clone(), String::new(), "author".into()]));
        assert!(ev
            .tags
            .iter()
            .any(|t| t == &["cid".to_string(), bind.chain_cid.clone()]));
        assert_eq!(tag(&ev, "bud").len(), 1);
        assert!(ev
            .tags
            .iter()
            .any(|t| t == &["t".to_string(), "dao-calendar".to_string()]));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn onchain_metadata_date_based() {
        let signer = NostrSigner::generate();
        // 1700000000 = 2023-11-14T22:13:20Z ; 1700100000 = 2023-11-16
        let e = r#"{"d_tag":"x","title":"OnChain","content":"body","start_time":1700000000,"end_time":1700100000}"#;
        let ev = onchain_to_nip52(e, KIND_DATE_BASED, Some("x"), &signer).unwrap();
        ev.verify().unwrap();
        assert_eq!(ev.kind, KIND_DATE_BASED as u32);
        assert_eq!(tag(&ev, "start")[0][1], "2023-11-14");
        assert_eq!(tag(&ev, "end")[0][1], "2023-11-16");
        assert_eq!(tag(&ev, "title")[0][1], "OnChain");
    }

    #[test]
    fn missing_start_or_title_is_rejected() {
        let signer = NostrSigner::generate();
        assert!(onchain_to_nip52(r#"{"d_tag":"x","title":"t"}"#, KIND_TIME_BASED, None, &signer)
            .unwrap_err()
            .contains("start"));
        assert!(onchain_to_nip52(r#"{"d_tag":"x","start_time":5}"#, KIND_TIME_BASED, None, &signer)
            .unwrap_err()
            .contains("title"));
        assert!(onchain_to_nip52(r#"{"title":"t","start_time":5}"#, KIND_TIME_BASED, None, &signer)
            .unwrap_err()
            .contains("d tag"));
        assert!(onchain_to_nip52(
            r#"{"d_tag":"x","title":"t","start_time":10,"end_time":5}"#,
            KIND_TIME_BASED,
            None,
            &signer
        )
        .is_err());
        assert!(onchain_to_nip52(r#"{"d_tag":"x","title":"t","start_time":5}"#, 1, None, &signer)
            .is_err());
    }

    #[test]
    fn validator_rules() {
        let s = NostrSigner::generate();
        let mk = |kind: u32, tags: Vec<Vec<&str>>| {
            let tags = tags
                .into_iter()
                .map(|t| t.into_iter().map(String::from).collect())
                .collect();
            s.sign_event(1, kind, tags, String::new()).unwrap()
        };
        let ok = mk(31922, vec![vec!["d", "a"], vec!["title", "t"], vec!["start", "2024-02-29"]]);
        validate_nip52_calendar_event(&ok).unwrap();
        for bad in [
            mk(31922, vec![vec!["d", "a"], vec!["d", "b"], vec!["title", "t"], vec!["start", "2024-02-29"]]),
            mk(31922, vec![vec!["d", "a"], vec!["title", "t"], vec!["start", "2023-02-29"]]),
            mk(31922, vec![vec!["d", "a"], vec!["title", "t"], vec!["start", "1700000000"]]),
            mk(31922, vec![vec!["d", "a"], vec!["title", "t"], vec!["start", "2024-01-02"], vec!["end", "2024-01-02"]]),
            mk(31922, vec![vec!["d", "a"], vec!["title", "t"], vec!["start", "2024-01-02"], vec!["start_tzid", "UTC"]]),
            mk(31923, vec![vec!["d", "a"], vec!["title", "t"], vec!["start", "2024-01-02"]]),
            mk(31923, vec![vec!["d", "a"], vec!["start", "5"]]),
            mk(31923, vec![vec!["d", ""], vec!["title", "t"], vec!["start", "5"]]),
            mk(31924, vec![vec!["d", "a"], vec!["title", "t"], vec!["start", "5"]]),
        ] {
            assert!(validate_nip52_calendar_event(&bad).is_err(), "{:?}", bad.tags);
        }
    }

    #[test]
    fn signed_template_passthrough_and_unsigned_resign() {
        let author = NostrSigner::generate();
        let bridge = NostrSigner::generate();
        let tags = vec![
            vec!["d".to_string(), "tpl".to_string()],
            vec!["title".to_string(), "Tpl".to_string()],
            vec!["start".to_string(), "100".to_string()],
        ];
        let signed = author
            .sign_event(50, KIND_TIME_BASED as u32, tags.clone(), "c".into())
            .unwrap();
        let view = CalendarMetaView {
            on_chain: true,
            e_json: Some(serde_json::to_string(&signed).unwrap()),
            cid: None,
            kind: KIND_TIME_BASED,
            d_tag: None,
            calendar_d: None,
            author_pubkey: None,
            nostr_e_d: None,
        };
        let out = metadata_to_nip52_event(&view, None, None, &bridge).unwrap();
        assert_eq!(out, signed, "author-signed event passes through untouched");

        // forged sig → rebuilt and signed by the bridge key, never forwarded
        let mut forged = signed.clone();
        forged.sig = "00".repeat(64);
        let view2 = CalendarMetaView {
            e_json: Some(serde_json::to_string(&forged).unwrap()),
            ..view
        };
        let out = metadata_to_nip52_event(&view2, None, None, &bridge).unwrap();
        out.verify().unwrap();
        assert_eq!(out.pubkey, bridge.pubkey_hex());
        assert_eq!(tag(&out, "d").len(), 1);
    }

    #[test]
    fn chain_attrs_legacy_bridge_is_valid_31922() {
        let signer = NostrSigner::generate();
        let attrs = vec![("action".into(), "create_event".into())];
        let ev =
            chain_attrs_to_nostr(&attrs, 7, Some("create_event"), Some("terp1c"), &signer).unwrap();
        ev.verify().unwrap();
        validate_nip52_calendar_event(&ev).unwrap();
        assert_eq!(ev.kind, KIND_DATE_BASED as u32);
        assert_eq!(tag(&ev, "d")[0][1], "chain/terp1c/7/create_event");
        assert!(ev.content.contains("\"chain_height\":7"));
    }

    #[test]
    fn civil_date_roundtrip() {
        assert_eq!(unix_to_ymd(0).unwrap(), "1970-01-01");
        assert_eq!(unix_to_ymd(951_782_400).unwrap(), "2000-02-29");
        assert_eq!(unix_to_ymd(1_700_000_000).unwrap(), "2023-11-14");
        for day in (0..80_000).step_by(37) {
            let s = unix_to_ymd(day * 86_400).unwrap();
            assert_eq!(parse_ymd(&s), Some(day));
        }
        assert!(parse_ymd("2023-02-29").is_none());
        assert!(parse_ymd("2023-13-01").is_none());
    }
}
