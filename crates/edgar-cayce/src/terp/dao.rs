//! Foundation DAO notifier.
//!
//! Polls the configured DAO core (and its subDAOs) over the LCD. The
//! contract address comes from `DAO_CONTRACT` / `DAO_ALLOWLIST` — nothing
//! is hard-coded. The first successful poll records what is already on
//! chain and does not replay it. Later polls post new proposals, votes,
//! and execution / closure to `DAO_NOTIFY_CHANNEL_ID`.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Addresses from `DAO_CONTRACT` (one) and `DAO_ALLOWLIST` (comma-separated).
/// Terp Foundation DAO core. Used when `DAO_CONTRACT` and `DAO_ALLOWLIST` are unset.
pub const FOUNDATION_DAO: &str = "terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w";

pub fn configured_daos() -> BTreeSet<String> {
    daos_or_foundation(daos_from_config(
        &std::env::var("DAO_CONTRACT").unwrap_or_default(),
        &std::env::var("DAO_ALLOWLIST").unwrap_or_default(),
    ))
}

pub fn daos_or_foundation(set: BTreeSet<String>) -> BTreeSet<String> {
    if !set.is_empty() {
        return set;
    }
    BTreeSet::from([FOUNDATION_DAO.to_string()])
}

pub fn daos_from_config(contract: &str, allowlist: &str) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    if let Some(addr) = normalize_addr(contract) {
        set.insert(addr);
    }
    for part in allowlist.split(',') {
        if let Some(addr) = normalize_addr(part) {
            set.insert(addr);
        }
    }
    set
}

pub fn allowlist() -> HashSet<String> {
    configured_daos().into_iter().collect()
}

pub fn is_allowed(dao: &str) -> bool {
    allowlist().contains(dao)
}

fn normalize_addr(raw: &str) -> Option<String> {
    let s = raw.trim();
    if !(8..=128).contains(&s.len()) {
        return None;
    }
    if !s.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(s.to_string())
}

fn include_subdaos_flag(raw: Option<&str>) -> bool {
    match raw.map(str::trim) {
        None | Some("") => true,
        Some(v) => !(v.eq_ignore_ascii_case("false") || v == "0" || v.eq_ignore_ascii_case("no")),
    }
}

fn include_subdaos() -> bool {
    include_subdaos_flag(std::env::var("DAO_INCLUDE_SUBDAOS").ok().as_deref())
}

#[derive(Debug, Clone)]
pub struct DaoEvent {
    pub dao: String,
    pub kind: String,
    pub title: String,
    pub url: String,
    /// DAO proposal status key (`open`, `executed`, …). Empty for other kinds.
    pub status_key: String,
}

pub fn format_event(e: &DaoEvent) -> Option<String> {
    if !is_allowed(&e.dao) {
        return None;
    }
    format_event_in(&allowlist(), e)
}

pub fn format_event_in(allowed: &HashSet<String>, e: &DaoEvent) -> Option<String> {
    if !allowed.contains(&e.dao) {
        return None;
    }
    let status = if e.status_key.is_empty() {
        String::new()
    } else {
        format!(" ({})", e.status_key)
    };
    Some(format!(
        "🏛️ **DAO {kind}**{status} — {dao}\n**{title}**\n{url}",
        kind = e.kind,
        dao = e.dao,
        status = status,
        title = e.title,
        url = e.url
    ))
}

pub fn proposal_url(dao: &str, prefix: &str, id: u64) -> String {
    let base = std::env::var("DAO_UI_BASE").unwrap_or_else(|_| "https://daodao.zone".into());
    let chain = std::env::var("DAO_CHAIN_ID").unwrap_or_default();
    proposal_url_parts(base.trim(), chain.trim(), dao, prefix, id)
}

pub fn proposal_url_parts(base: &str, chain: &str, dao: &str, prefix: &str, id: u64) -> String {
    let base = base.trim_end_matches('/');
    let tag = if prefix.is_empty() {
        id.to_string()
    } else {
        format!("{prefix}{id}")
    };
    if chain.is_empty() {
        format!("{base}/dao/{dao}/proposals/{tag}")
    } else {
        format!("{base}/dao/{chain}/{dao}/proposals/{tag}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalView {
    pub id: u64,
    pub title: String,
    pub status_key: String,
    pub status_stored: String,
    pub proposer: String,
    /// `None` means this poll did not refresh votes; keep the previous set.
    pub votes: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PropSnap {
    pub title: String,
    pub status: String,
    #[serde(default)]
    pub voters: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DaoState {
    #[serde(default)]
    pub seeded: bool,
    /// dao core → proposal module → proposal id → snap
    #[serde(default)]
    pub daos: BTreeMap<String, BTreeMap<String, BTreeMap<String, PropSnap>>>,
}

pub fn load_state(path: &Path) -> DaoState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_state(path: &Path, state: &DaoState) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(state)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn smart_query_url(rest: &str, contract: &str, query: &Value) -> Result<String> {
    let addr = normalize_addr(contract).context("bad dao contract address")?;
    let raw = serde_json::to_vec(query)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(raw);
    Ok(format!(
        "{}/cosmwasm/wasm/v1/contract/{}/smart/{}",
        rest.trim_end_matches('/'),
        addr,
        percent_encode(&b64)
    ))
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// LCD `/smart` responses wrap the query JSON in `data` (base64 or object).
pub fn decode_smart(v: &Value) -> Result<Value> {
    if let Some(data) = v.get("data") {
        match data {
            Value::String(s) => {
                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(s.trim()) {
                    if let Ok(inner) = serde_json::from_slice::<Value>(&bytes) {
                        return Ok(inner);
                    }
                }
                if let Ok(inner) = serde_json::from_str::<Value>(s) {
                    return Ok(inner);
                }
                anyhow::bail!("smart query data was not json");
            }
            Value::Null => {}
            other => return Ok(other.clone()),
        }
    }
    Ok(v.clone())
}

pub fn parse_modules(v: &Value) -> Vec<(String, String)> {
    let list = v
        .as_array()
        .or_else(|| v.get("proposal_modules").and_then(|x| x.as_array()));
    let Some(list) = list else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in list {
        if let Some(addr) = item.as_str().and_then(normalize_addr) {
            out.push((addr, String::new()));
            continue;
        }
        let status = item
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("enabled");
        if status.eq_ignore_ascii_case("disabled") {
            continue;
        }
        let Some(addr) = item
            .get("address")
            .and_then(|a| a.as_str())
            .and_then(normalize_addr)
        else {
            continue;
        };
        let prefix = item
            .get("prefix")
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .to_string();
        out.push((addr, prefix));
    }
    out
}

pub fn parse_subdaos(v: &Value) -> Vec<String> {
    let list = v
        .as_array()
        .or_else(|| v.get("sub_daos").and_then(|x| x.as_array()));
    let Some(list) = list else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in list {
        let addr = item
            .as_str()
            .or_else(|| item.get("addr").and_then(|a| a.as_str()))
            .or_else(|| item.get("address").and_then(|a| a.as_str()));
        if let Some(addr) = addr.and_then(normalize_addr) {
            out.push(addr);
        }
    }
    out
}

pub fn parse_proposals(v: &Value) -> Vec<ProposalView> {
    if let Some(list) = v.get("proposals").and_then(|p| p.as_array()) {
        return list.iter().filter_map(parse_proposal).collect();
    }
    if v.get("id").is_some() {
        return parse_proposal(v).into_iter().collect();
    }
    if let Some(list) = v.as_array() {
        return list.iter().filter_map(parse_proposal).collect();
    }
    Vec::new()
}

fn parse_proposal(v: &Value) -> Option<ProposalView> {
    let id = json_u64(&v["id"])?;
    let prop = if v.get("proposal").is_some() {
        &v["proposal"]
    } else {
        v
    };
    let title = prop["title"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("(untitled)")
        .to_string();
    let (status_key, status_stored) = status_parts(&prop["status"]);
    let proposer = prop["proposer"].as_str().unwrap_or("").to_string();
    Some(ProposalView {
        id,
        title,
        status_key,
        status_stored,
        proposer,
        votes: None,
    })
}

fn status_parts(v: &Value) -> (String, String) {
    match v {
        Value::String(s) => (s.clone(), s.clone()),
        Value::Object(_) => {
            let key = v
                .as_object()
                .and_then(|m| m.keys().next().cloned())
                .unwrap_or_else(|| "unknown".into());
            let stored = serde_json::to_string(v).unwrap_or_else(|_| key.clone());
            (key, stored)
        }
        _ => ("unknown".into(), "unknown".into()),
    }
}

pub fn status_key_from_stored(stored: &str) -> String {
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(stored) {
        if let Some(k) = map.keys().next() {
            return k.clone();
        }
    }
    stored.to_string()
}

fn json_u64(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

pub fn parse_votes(v: &Value) -> Vec<(String, String)> {
    let Some(list) = v.get("votes").and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for vote in list {
        let Some(voter) = vote
            .get("voter")
            .and_then(|x| x.as_str())
            .and_then(normalize_addr)
        else {
            continue;
        };
        let choice = match vote.get("vote") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => serde_json::to_string(other).unwrap_or_default(),
            None => String::new(),
        };
        if choice.is_empty() {
            continue;
        }
        out.push((voter, choice));
    }
    out
}

pub fn status_event_kind(key: &str) -> Option<&'static str> {
    match key {
        "open" => None,
        "executed" => Some("execute"),
        "closed" | "rejected" | "execution_failed" | "vetoed" => Some("close"),
        "passed" => Some("passed"),
        _ => Some("update"),
    }
}

/// Diff one module. Caller drops the events when `seeded` is still false.
pub fn diff_proposals(
    dao: &str,
    prefix: &str,
    prev: &BTreeMap<String, PropSnap>,
    views: &[ProposalView],
    mut url_for: impl FnMut(u64) -> String,
) -> (BTreeMap<String, PropSnap>, Vec<DaoEvent>) {
    let mut next = BTreeMap::new();
    let mut events = Vec::new();
    for view in views {
        let id_key = view.id.to_string();
        let url = url_for(view.id);
        let label = prop_label(prefix, view.id, &view.title);
        let prev_snap = prev.get(&id_key);
        if prev_snap.is_none() {
            let title = if view.proposer.is_empty() {
                label.clone()
            } else {
                format!("{label} — proposed by {}", one_line(&view.proposer, 80))
            };
            events.push(DaoEvent {
                dao: dao.to_string(),
                kind: "proposal".into(),
                title,
                url: url.clone(),
                status_key: view.status_key.clone(),
            });
        }
        let voters = if let Some(fresh) = &view.votes {
            let old = prev_snap.map(|s| &s.voters);
            for (voter, choice) in fresh {
                let changed = match old {
                    Some(map) => map.get(voter).is_none_or(|c| c != choice),
                    None => true,
                };
                if changed {
                    events.push(DaoEvent {
                        dao: dao.to_string(),
                        kind: "vote".into(),
                        title: format!(
                            "{label} — {} voted {}",
                            one_line(voter, 80),
                            one_line(choice, 40)
                        ),
                        url: url.clone(),
                        status_key: String::new(),
                    });
                }
            }
            fresh.clone()
        } else if let Some(prev_snap) = prev_snap {
            prev_snap.voters.clone()
        } else {
            BTreeMap::new()
        };
        let status_changed = match prev_snap {
            Some(prev_snap) => prev_snap.status != view.status_stored,
            None => view.status_key != "open",
        };
        if status_changed {
            if let Some(kind) = status_event_kind(&view.status_key) {
                events.push(DaoEvent {
                    dao: dao.to_string(),
                    kind: kind.into(),
                    title: format!("{label} — {}", view.status_key),
                    url: url.clone(),
                    status_key: view.status_key.clone(),
                });
            }
        }
        next.insert(
            id_key,
            PropSnap {
                title: view.title.clone(),
                status: view.status_stored.clone(),
                voters,
            },
        );
    }
    (next, events)
}

/// New proposals only. Votes and later status changes stay in memory and
/// are not posted. The first poll still posts proposals that are open, so a
/// proposal created before this watch started is not swallowed. Closed
/// history stays quiet.
pub fn publish_events(seeded: bool, events: Vec<DaoEvent>) -> Vec<DaoEvent> {
    events
        .into_iter()
        .filter(|e| e.kind == "proposal")
        .filter(|e| seeded || e.status_key == "open")
        .collect()
}

pub fn cap_notes(mut notes: Vec<String>, cap: usize) -> Vec<String> {
    if notes.len() <= cap {
        return notes;
    }
    let extra = notes.len() - cap;
    notes.truncate(cap);
    notes.push(format!(
        "…and {extra} more DAO events this poll. state advanced; they will not be repeated."
    ));
    notes
}

fn prop_label(prefix: &str, id: u64, title: &str) -> String {
    let tag = if prefix.is_empty() {
        format!("#{id}")
    } else {
        format!("{prefix}{id}")
    };
    format!("{tag} {}", one_line(title, 120))
}

fn one_line(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(max).collect()
}

fn is_live(stored: &str) -> bool {
    matches!(
        status_key_from_stored(stored).as_str(),
        "open" | "passed" | "veto_timelock"
    )
}

fn wants_votes(prev: Option<&PropSnap>, view: &ProposalView) -> bool {
    let Some(prev) = prev else {
        return true;
    };
    prev.status != view.status_stored || is_live(&prev.status)
}

pub async fn tick(
    client: &reqwest::Client,
    rest: &str,
    state: &mut DaoState,
) -> Result<Vec<String>> {
    let cores = configured_daos();
    if cores.is_empty() {
        return Ok(Vec::new());
    }
    let mut targets = cores.clone();
    if include_subdaos() {
        for core in &cores {
            match list_subdaos(client, rest, core).await {
                Ok(subs) => targets.extend(subs),
                Err(e) => tracing::warn!(dao = %core, "subdao list failed: {e:#}"),
            }
        }
    }
    let allowed: HashSet<String> = targets.iter().cloned().collect();
    let already = state.seeded;
    let mut next_daos = state.daos.clone();
    let mut events = Vec::new();
    for dao in &targets {
        let modules = list_modules(client, rest, dao)
            .await
            .with_context(|| format!("proposal modules for {dao}"))?;
        let prev_dao = state.daos.get(dao).cloned().unwrap_or_default();
        let mut module_map = BTreeMap::new();
        for (module, prefix) in modules {
            let prev_mod = prev_dao.get(&module).cloned().unwrap_or_default();
            let mut views = list_recent_proposals(client, rest, &module).await?;
            backfill_live(client, rest, &module, &prev_mod, &mut views).await;
            for view in &mut views {
                if !wants_votes(prev_mod.get(&view.id.to_string()), view) {
                    continue;
                }
                match list_votes(client, rest, &module, view.id).await {
                    Ok(votes) => view.votes = Some(votes),
                    Err(e) => {
                        tracing::warn!(module = %module, id = view.id, "vote list failed: {e:#}")
                    }
                }
            }
            let (snap, found) = diff_proposals(dao, &prefix, &prev_mod, &views, |id| {
                proposal_url(dao, &prefix, id)
            });
            events.extend(found);
            module_map.insert(module, snap);
        }
        next_daos.insert(dao.clone(), module_map);
    }
    state.daos = next_daos;
    state.seeded = true;
    let events = publish_events(already, events);
    if !already {
        tracing::info!(
            daos = state.daos.len(),
            open = events.len(),
            "dao watch seeded; posting proposals that are still open"
        );
    }
    let notes: Vec<String> = events
        .iter()
        .filter_map(|e| format_event_in(&allowed, e))
        .collect();
    if notes.len() > 40 {
        for text in notes.iter().skip(40) {
            tracing::warn!(%text, "dao event over poll cap");
        }
    }
    Ok(cap_notes(notes, 40))
}

async fn smart(
    client: &reqwest::Client,
    rest: &str,
    contract: &str,
    query: &Value,
) -> Result<Value> {
    let url = smart_query_url(rest, contract, query)?;
    let body = crate::terp::chain_watch::fetch_json(client, &url).await?;
    decode_smart(&body)
}

async fn list_subdaos(client: &reqwest::Client, rest: &str, dao: &str) -> Result<Vec<String>> {
    let v = smart(
        client,
        rest,
        dao,
        &serde_json::json!({"list_sub_daos": {"limit": 30}}),
    )
    .await?;
    Ok(parse_subdaos(&v))
}

async fn list_modules(
    client: &reqwest::Client,
    rest: &str,
    dao: &str,
) -> Result<Vec<(String, String)>> {
    if let Ok(v) = smart(
        client,
        rest,
        dao,
        &serde_json::json!({"active_proposal_modules": {"limit": 25}}),
    )
    .await
    {
        return Ok(parse_modules(&v));
    }
    if let Ok(v) = smart(
        client,
        rest,
        dao,
        &serde_json::json!({"proposal_modules": {"limit": 25}}),
    )
    .await
    {
        return Ok(parse_modules(&v));
    }
    let v = smart(client, rest, dao, &serde_json::json!({"dump_state": {}})).await?;
    Ok(parse_modules(&v))
}

async fn list_recent_proposals(
    client: &reqwest::Client,
    rest: &str,
    module: &str,
) -> Result<Vec<ProposalView>> {
    let mut out = Vec::new();
    let mut start_before: Option<u64> = None;
    for page_ix in 0..6 {
        let mut q = serde_json::json!({"reverse_proposals": {"limit": 30}});
        if let Some(id) = start_before {
            q["reverse_proposals"]["start_before"] = serde_json::json!(id);
        }
        let v = match smart(client, rest, module, &q).await {
            Ok(v) => v,
            Err(e) if page_ix == 0 => {
                tracing::warn!("reverse_proposals failed ({e:#}); trying list_proposals");
                return list_proposals_forward(client, rest, module).await;
            }
            Err(e) => return Err(e),
        };
        let page = parse_proposals(&v);
        if page.is_empty() {
            break;
        }
        let last = page.last().map(|p| p.id);
        let n = page.len();
        out.extend(page);
        if n < 30 || start_before == last {
            break;
        }
        start_before = last;
    }
    Ok(out)
}

async fn list_proposals_forward(
    client: &reqwest::Client,
    rest: &str,
    module: &str,
) -> Result<Vec<ProposalView>> {
    let mut out = Vec::new();
    let mut start_after: Option<u64> = None;
    for _ in 0..6 {
        let mut q = serde_json::json!({"list_proposals": {"limit": 30}});
        if let Some(id) = start_after {
            q["list_proposals"]["start_after"] = serde_json::json!(id);
        }
        let v = smart(client, rest, module, &q).await?;
        let page = parse_proposals(&v);
        if page.is_empty() {
            break;
        }
        let last = page.last().map(|p| p.id);
        let n = page.len();
        out.extend(page);
        if n < 30 || start_after == last {
            break;
        }
        start_after = last;
    }
    Ok(out)
}

async fn backfill_live(
    client: &reqwest::Client,
    rest: &str,
    module: &str,
    prev: &BTreeMap<String, PropSnap>,
    views: &mut Vec<ProposalView>,
) {
    let seen: BTreeSet<u64> = views.iter().map(|v| v.id).collect();
    for (id_key, snap) in prev {
        if !is_live(&snap.status) {
            continue;
        }
        let Ok(id) = id_key.parse::<u64>() else {
            continue;
        };
        if seen.contains(&id) {
            continue;
        }
        match smart(
            client,
            rest,
            module,
            &serde_json::json!({"proposal": {"proposal_id": id}}),
        )
        .await
        {
            Ok(v) => views.extend(parse_proposals(&v)),
            Err(e) => tracing::warn!(module = %module, id, "live proposal fetch failed: {e:#}"),
        }
    }
}

async fn list_votes(
    client: &reqwest::Client,
    rest: &str,
    module: &str,
    proposal_id: u64,
) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    let mut start_after: Option<String> = None;
    for _ in 0..20 {
        let mut q = serde_json::json!({"list_votes": {"proposal_id": proposal_id, "limit": 30}});
        if let Some(voter) = &start_after {
            q["list_votes"]["start_after"] = Value::String(voter.clone());
        }
        let v = smart(client, rest, module, &q).await?;
        let page = parse_votes(&v);
        if page.is_empty() {
            break;
        }
        let last = page.last().map(|(voter, _)| voter.clone());
        let n = page.len();
        let before = out.len();
        for (voter, choice) in page {
            out.insert(voter, choice);
        }
        if out.len() == before || n < 30 || start_after == last {
            break;
        }
        start_after = last;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn addresses_come_from_config_only() {
        let set = daos_from_config(" terp1coreaaaa ", "terp1subaaaaa, terp1coreaaaa ,");
        assert_eq!(
            set,
            BTreeSet::from(["terp1coreaaaa".into(), "terp1subaaaaa".into()])
        );
        assert!(daos_from_config("", "").is_empty());
        assert_eq!(
            daos_or_foundation(BTreeSet::new()),
            BTreeSet::from([FOUNDATION_DAO.to_string()])
        );
        assert!(daos_from_config("bad/addr", "short").is_empty());
    }

    #[test]
    fn format_drops_daos_outside_the_allow_set() {
        let allowed = HashSet::from(["terp1coreaaaa".into()]);
        let kept = DaoEvent {
            dao: "terp1coreaaaa".into(),
            kind: "proposal".into(),
            title: "A1 Grants".into(),
            url: "https://daodao.zone/x".into(),
            status_key: "open".into(),
        };
        let text = format_event_in(&allowed, &kept).unwrap();
        assert!(text.contains("**DAO proposal**"));
        assert!(text.contains("terp1coreaaaa"));
        let foreign = DaoEvent {
            dao: "terp1otheraaaa".into(),
            ..kept
        };
        assert!(format_event_in(&allowed, &foreign).is_none());
    }

    #[test]
    fn proposal_url_uses_the_configured_dao() {
        let url = proposal_url_parts("https://daodao.zone/", "terpnet-1", "terp1coreaaaa", "A", 3);
        assert_eq!(
            url,
            "https://daodao.zone/dao/terpnet-1/terp1coreaaaa/proposals/A3"
        );
        assert!(!url.contains("foundationdao"));
    }

    #[test]
    fn smart_query_url_encodes_the_query() {
        let url = smart_query_url(
            "https://lcd.example/",
            "terp1abcdefgh",
            &serde_json::json!({"config": {}}),
        )
        .unwrap();
        assert!(
            url.starts_with("https://lcd.example/cosmwasm/wasm/v1/contract/terp1abcdefgh/smart/")
        );
        assert!(url.contains("eyJjb25maWciOnt9fQ%3D%3D"));
        assert!(smart_query_url("https://lcd.example", "nope", &serde_json::json!({})).is_err());
    }

    #[test]
    fn decode_smart_accepts_base64_and_objects() {
        let inner = br#"{"proposals":[]}"#;
        let b64 = base64::engine::general_purpose::STANDARD.encode(inner);
        let decoded = decode_smart(&serde_json::json!({"data": b64})).unwrap();
        assert!(decoded["proposals"].as_array().unwrap().is_empty());
        let decoded = decode_smart(&serde_json::json!({"data": {"votes": []}})).unwrap();
        assert!(decoded["votes"].as_array().unwrap().is_empty());
    }

    #[test]
    fn parsers_cover_modules_subdaos_proposals_and_votes() {
        let modules = parse_modules(&serde_json::json!([
            {"address": "terp1modaaaaaa", "prefix": "A", "status": "enabled"},
            {"address": "terp1offaaaaaa", "prefix": "B", "status": "disabled"}
        ]));
        assert_eq!(modules, vec![("terp1modaaaaaa".into(), "A".into())]);
        let from_dump = parse_modules(&serde_json::json!({"proposal_modules": ["terp1modaaaaaa"]}));
        assert_eq!(from_dump, vec![("terp1modaaaaaa".into(), String::new())]);
        let subs = parse_subdaos(&serde_json::json!([
            {"addr": "terp1subaaaaaa", "charter": null}
        ]));
        assert_eq!(subs, vec!["terp1subaaaaaa".to_string()]);
        let props = parse_proposals(&serde_json::json!({
            "proposals": [{
                "id": "3",
                "proposal": {
                    "title": "Grants",
                    "status": {"veto_timelock": {"expiration": {"at_height": 10}}},
                    "proposer": "terp1proposer"
                }
            }]
        }));
        assert_eq!(props[0].id, 3);
        assert_eq!(props[0].status_key, "veto_timelock");
        assert_eq!(status_event_kind("veto_timelock"), Some("update"));
        let votes = parse_votes(&serde_json::json!({
            "votes": [
                {"voter": "terp1voteraaaa", "vote": "yes", "power": "10"},
                {"voter": "terp1voterbbbb", "vote": {"option_id": 1}}
            ]
        }));
        assert_eq!(votes[0], ("terp1voteraaaa".into(), "yes".into()));
        assert!(votes[1].1.contains("option_id"));
    }

    fn view(id: u64, status: &str, votes: Option<Vec<(&str, &str)>>) -> ProposalView {
        ProposalView {
            id,
            title: "Grants".into(),
            status_key: status.into(),
            status_stored: status.into(),
            proposer: "terp1proposer".into(),
            votes: votes.map(|pairs| {
                pairs
                    .into_iter()
                    .map(|(v, c)| (v.to_string(), c.to_string()))
                    .collect()
            }),
        }
    }

    #[test]
    fn diff_emits_proposal_votes_execution_and_closure_once() {
        let (snap, events) = diff_proposals(
            "terp1coreaaaa",
            "A",
            &BTreeMap::new(),
            &[view(1, "open", Some(vec![("terp1voteraaaa", "yes")]))],
            |id| format!("https://example.test/{id}"),
        );
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, "proposal");
        assert!(events[0].title.contains("A1"));
        assert!(events[0].title.contains("terp1proposer"));
        assert_eq!(events[1].kind, "vote");
        assert!(events[1].title.contains("voted yes"));
        let first = publish_events(false, events.clone());
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].kind, "proposal");
        assert_eq!(first[0].status_key, "open");
        assert_eq!(publish_events(true, events).len(), 1);
        let born_closed = view(9, "executed", None);
        let (_, closed_events) =
            diff_proposals("terp1coreaaaa", "A", &BTreeMap::new(), &[born_closed], |_| {
                "u".into()
            });
        assert!(publish_events(false, closed_events).is_empty());

        let (snap, again) = diff_proposals(
            "terp1coreaaaa",
            "A",
            &snap,
            &[view(1, "open", Some(vec![("terp1voteraaaa", "yes")]))],
            |_| "u".into(),
        );
        assert!(again.is_empty());

        let mut changed = view(
            1,
            "open",
            Some(vec![("terp1voteraaaa", "no"), ("terp1voterbbbb", "yes")]),
        );
        let (_, votes) = diff_proposals("terp1coreaaaa", "A", &snap, &[changed.clone()], |_| {
            "u".into()
        });
        assert_eq!(votes.len(), 2);
        assert!(votes.iter().any(|e| e.title.contains("voted no")));
        assert!(votes.iter().all(|e| e.kind == "vote"));

        changed.status_key = "executed".into();
        changed.status_stored = "executed".into();
        changed.votes = None;
        let (_, status) = diff_proposals("terp1coreaaaa", "A", &snap, &[changed], |_| "u".into());
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].kind, "execute");

        let mut closed = view(1, "rejected", None);
        closed.status_stored = "rejected".into();
        let (_, status) = diff_proposals("terp1coreaaaa", "A", &snap, &[closed], |_| "u".into());
        assert_eq!(status[0].kind, "close");

        let born_done = view(4, "executed", None);
        let (_, both) =
            diff_proposals("terp1coreaaaa", "A", &BTreeMap::new(), &[born_done], |_| {
                "u".into()
            });
        assert_eq!(both[0].kind, "proposal");
        assert_eq!(both[1].kind, "execute");
    }

    #[test]
    fn cap_and_subdao_flag() {
        let notes = cap_notes(vec!["a".into(), "b".into(), "c".into()], 2);
        assert_eq!(notes.len(), 3);
        assert!(notes[2].contains("1 more"));
        assert!(include_subdaos_flag(None));
        assert!(!include_subdaos_flag(Some("false")));
        assert!(!include_subdaos_flag(Some("0")));
        assert!(include_subdaos_flag(Some("true")));
    }

    #[test]
    fn state_roundtrip() {
        let path = std::env::temp_dir().join(format!(
            "dao-watch-test-{}-{}.json",
            std::process::id(),
            "ec-dao"
        ));
        let mut state = DaoState::default();
        state.seeded = true;
        state.daos.insert(
            "terp1coreaaaa".into(),
            BTreeMap::from([(
                "terp1modaaaaaa".into(),
                BTreeMap::from([(
                    "1".into(),
                    PropSnap {
                        title: "Grants".into(),
                        status: "open".into(),
                        voters: BTreeMap::from([("terp1voteraaaa".into(), "yes".into())]),
                    },
                )]),
            )]),
        );
        save_state(&path, &state).unwrap();
        let loaded = load_state(&path);
        let _ = std::fs::remove_file(&path);
        assert!(loaded.seeded);
        assert_eq!(
            loaded.daos["terp1coreaaaa"]["terp1modaaaaaa"]["1"].voters["terp1voteraaaa"],
            "yes"
        );
    }
}
