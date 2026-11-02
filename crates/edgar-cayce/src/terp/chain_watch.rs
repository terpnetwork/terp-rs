//! Local-node watches for Discord: gov proposals, new IBC channels and
//! clients,
//! and a once-per-UTC-day voting-power diff. The stake post is a short
//! line plus a Markdown file from `crate::terp::digest`.
//!
//! Gov alerts: a proposal entering the deposit period, the same proposal
//! moving into the voting period, and one reminder when a vote has at most
//! 24 hours left. Each of those messages includes a block-explorer link.
//! The reminder id is stored on `WatchState` so a restart does not repeat it.
//!
//! Staking amounts on the LCD are `uthiol` (6 decimals). Posts show TERP.
//!
//! Polls `TERP_NODE_REST` (the house LCD). A CometBFT websocket on `:26657`
//! would shave latency for tx events, but a daily staking summary is a
//! snapshot, and a 45s LCD poll still sees new proposals and clients without
//! a reconnect/gap-fill protocol. Websocket stays out of v1.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::info;

/// Ping.pub proposal pages. Chain-registry lists this explorer first for
/// terpnetwork, and Terp has pointed people at `https://ping.pub/terp/gov`.
/// Override with `GOV_EXPLORER_BASE` (no proposal id; the id is appended).
pub const DEFAULT_EXPLORER_BASE: &str = "https://ping.pub/terp/gov";

/// Fire the voting reminder when this many seconds or fewer remain, and the
/// vote has not already ended.
const REMINDER_WINDOW_SECS: i64 = 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WatchState {
    #[serde(default)]
    pub seeded: bool,
    #[serde(default)]
    pub seen_proposals: BTreeSet<String>,
    /// Last status recorded per proposal id (`DEPOSIT_PERIOD`, `VOTING_PERIOD`).
    /// Empty for ids remembered by an older state file. A missing entry is not
    /// treated as a deposit → voting transition.
    #[serde(default)]
    pub proposal_phase: BTreeMap<String, String>,
    /// Proposal ids that already received the 24h voting reminder.
    #[serde(default)]
    pub reminded_24h: BTreeSet<String>,
    /// `port/channel` keys already announced. Empty on older state files
    /// means "seed the current set, do not replay it".
    #[serde(default)]
    pub seen_channels: BTreeSet<String>,
    /// Set on the first client list, including an empty one, so a later
    /// client is announced instead of being swallowed by a quiet seed.
    #[serde(default)]
    pub clients_seeded: bool,
    #[serde(default)]
    pub seen_clients: BTreeSet<String>,
    /// `port/channel` → `STATE<TAB>counterparty`. Empty while an older file
    /// only has keys: the next poll records snaps and stays quiet.
    #[serde(default)]
    pub channel_snap: BTreeMap<String, String>,
    /// `{plan}@{halt}@{threshold}` upgrade countdowns already posted.
    #[serde(default)]
    pub upgrade_notices: BTreeSet<String>,
    /// One-line gov and IBC events since the last morning post.
    #[serde(default)]
    pub day_log: Vec<String>,
    /// `YYYY-MM-DD` of the last posted staking digest.
    #[serde(default)]
    pub last_stake_day: String,
    #[serde(default)]
    pub bonded: String,
    #[serde(default)]
    pub validators: BTreeMap<String, ValSnap>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValSnap {
    pub moniker: String,
    pub status: String,
    pub jailed: bool,
    pub tokens: String,
}

#[derive(Debug, Clone)]
pub struct ProposalNote {
    pub id: String,
    pub title: String,
    /// Canonical status: `DEPOSIT_PERIOD`, `VOTING_PERIOD`, …
    pub status: String,
    pub summary: String,
    /// Set for proposals that have entered voting. `0001-01-01` is treated as absent.
    pub voting_end: Option<DateTime<Utc>>,
}

pub fn load_state(path: &Path) -> WatchState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_state(path: &Path, state: &WatchState) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(state)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn parse_proposals(v: &Value) -> Vec<ProposalNote> {
    let mut out = Vec::new();
    let Some(list) = v["proposals"].as_array() else {
        return out;
    };
    for p in list {
        let id = json_scalar(&p["id"]);
        if id.is_empty() {
            continue;
        }
        out.push(ProposalNote {
            id,
            title: proposal_title(p),
            status: normalize_status(&json_scalar(&p["status"])),
            summary: p["summary"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(280)
                .collect(),
            voting_end: parse_time(&p["voting_end_time"]),
        });
    }
    out
}

/// LCD JSON emits uint64 ids as strings. Some proxies emit numbers.
fn json_scalar(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    if let Some(n) = v.as_u64() {
        return n.to_string();
    }
    if let Some(n) = v.as_i64() {
        return n.to_string();
    }
    String::new()
}

pub fn normalize_status(raw: &str) -> String {
    match raw.trim() {
        "1" | "PROPOSAL_STATUS_DEPOSIT_PERIOD" | "DEPOSIT_PERIOD" => "DEPOSIT_PERIOD".into(),
        "2" | "PROPOSAL_STATUS_VOTING_PERIOD" | "VOTING_PERIOD" => "VOTING_PERIOD".into(),
        "3" | "PROPOSAL_STATUS_PASSED" | "PASSED" => "PASSED".into(),
        "4" | "PROPOSAL_STATUS_REJECTED" | "REJECTED" => "REJECTED".into(),
        "5" | "PROPOSAL_STATUS_FAILED" | "FAILED" => "FAILED".into(),
        "0" | "PROPOSAL_STATUS_UNSPECIFIED" | "" => "UNSPECIFIED".into(),
        other => other.trim_start_matches("PROPOSAL_STATUS_").to_string(),
    }
}

fn parse_time(v: &Value) -> Option<DateTime<Utc>> {
    let s = v.as_str()?;
    let t = DateTime::parse_from_rfc3339(s).ok()?.with_timezone(&Utc);
    if t.year() < 2000 {
        None
    } else {
        Some(t)
    }
}

pub fn resolve_explorer_base(raw: Option<&str>) -> String {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.trim_end_matches('/').to_string())
        .unwrap_or_else(|| DEFAULT_EXPLORER_BASE.to_string())
}

pub fn explorer_base() -> String {
    resolve_explorer_base(std::env::var("GOV_EXPLORER_BASE").ok().as_deref())
}

pub fn proposal_url(base: &str, id: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), id.trim())
}

fn proposal_title(p: &Value) -> String {
    if let Some(t) = p["title"].as_str().filter(|s| !s.is_empty()) {
        return t.to_string();
    }
    if let Some(msgs) = p["messages"].as_array() {
        for m in msgs {
            if let Some(t) = m["content"]["title"].as_str().filter(|s| !s.is_empty()) {
                return t.to_string();
            }
            if let Some(n) = m["plan"]["name"].as_str().filter(|s| !s.is_empty()) {
                return format!("upgrade {n}");
            }
        }
    }
    "(untitled)".into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelNote {
    pub key: String,
    pub counterparty: String,
    pub state: String,
    /// Counterparty chain id from the connection's light client. Empty until joined.
    pub chain_id: String,
    /// First connection hop (`connection-N`). Empty when the LCD omits it.
    pub connection: String,
}

/// `uthiol` base units to a TERP display amount (6 decimal places).
pub fn format_uthiol(raw: &str) -> String {
    const DECIMALS: usize = 6;
    let neg = raw.trim_start().starts_with('-');
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() || digits.chars().all(|c| c == '0') {
        return "0 TERP".into();
    }
    let digits = digits.trim_start_matches('0').to_string();
    let (whole, frac) = if digits.len() <= DECIMALS {
        let pad = "0".repeat(DECIMALS - digits.len());
        ("0".to_string(), format!("{pad}{digits}"))
    } else {
        let split = digits.len() - DECIMALS;
        (digits[..split].to_string(), digits[split..].to_string())
    };
    let frac = frac.trim_end_matches('0');
    let whole = group_thousands(&whole);
    let body = if frac.is_empty() {
        whole
    } else {
        format!("{whole}.{frac}")
    };
    if neg {
        format!("-{body} TERP")
    } else {
        format!("{body} TERP")
    }
}

fn group_thousands(whole: &str) -> String {
    let chars: Vec<char> = whole.chars().collect();
    let mut out = String::new();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*c);
    }
    out
}

pub fn parse_channels(v: &Value) -> Vec<ChannelNote> {
    let mut out = Vec::new();
    let Some(list) = v["channels"].as_array() else {
        return out;
    };
    for ch in list {
        let port = ch["port_id"].as_str().unwrap_or("");
        let id = ch["channel_id"].as_str().unwrap_or("");
        if port.is_empty() || id.is_empty() {
            continue;
        }
        let cp_port = ch["counterparty"]["port_id"].as_str().unwrap_or("?");
        let cp_id = ch["counterparty"]["channel_id"].as_str().unwrap_or("?");
        let hop = ch["connection_hops"]
            .as_array()
            .and_then(|hops| hops.first())
            .and_then(|h| h.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        out.push(ChannelNote {
            key: format!("{port}/{id}"),
            counterparty: format!("{cp_port}/{cp_id}"),
            state: ch["state"].as_str().unwrap_or("?").to_string(),
            chain_id: String::new(),
            connection: hop,
        });
    }
    out
}

pub fn parse_pool_bonded(v: &Value) -> String {
    v["pool"]["bonded_tokens"]
        .as_str()
        .unwrap_or("0")
        .to_string()
}

pub fn parse_validators(v: &Value) -> BTreeMap<String, ValSnap> {
    let mut out = BTreeMap::new();
    let Some(list) = v["validators"].as_array() else {
        return out;
    };
    for val in list {
        let addr = val["operator_address"].as_str().unwrap_or("").to_string();
        if addr.is_empty() {
            continue;
        }
        out.insert(
            addr,
            ValSnap {
                moniker: val["description"]["moniker"]
                    .as_str()
                    .unwrap_or("?")
                    .chars()
                    .take(40)
                    .collect(),
                status: val["status"].as_str().unwrap_or("?").to_string(),
                jailed: val["jailed"].as_bool().unwrap_or(false),
                tokens: val["tokens"].as_str().unwrap_or("0").to_string(),
            },
        );
    }
    out
}

/// New ids in `now` that are not in `seen`. Does not mutate `seen`.
pub fn newly(seen: &BTreeSet<String>, now: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut n: Vec<String> = now.into_iter().filter(|id| !seen.contains(id)).collect();
    n.sort();
    n
}

pub fn staking_digest(
    day: &str,
    prev_bonded: &str,
    prev: &BTreeMap<String, ValSnap>,
    bonded: &str,
    next: &BTreeMap<String, ValSnap>,
) -> String {
    let mut lines = vec![format!("**Voting power** `{day}`")];
    lines.push(format!(
        "bonded {} → {} ({})",
        format_uthiol(prev_bonded),
        format_uthiol(bonded),
        pct_delta(prev_bonded, bonded)
    ));
    let mut news = Vec::new();
    for (addr, v) in next {
        if !prev.contains_key(addr) {
            news.push(format!("+ {}", v.moniker));
        }
    }
    for (addr, v) in prev {
        if !next.contains_key(addr) {
            news.push(format!("- {}", v.moniker));
        }
    }
    for (addr, v) in next {
        let Some(old) = prev.get(addr) else { continue };
        if old.jailed != v.jailed {
            news.push(format!(
                "{} {}",
                if v.jailed { "jailed" } else { "unjailed" },
                v.moniker
            ));
        }
        if old.status != v.status {
            news.push(format!(
                "{} status {} → {}",
                v.moniker,
                short_status(&old.status),
                short_status(&v.status)
            ));
        }
        if material_token_move(&old.tokens, &v.tokens) {
            news.push(format!(
                "{} {} → {} ({}) {}",
                v.moniker,
                format_uthiol(&old.tokens),
                format_uthiol(&v.tokens),
                pct_delta(&old.tokens, &v.tokens),
                addr_tail(addr)
            ));
        }
    }
    if news.is_empty() {
        lines.push("no validator set or >1% token changes".into());
    } else {
        lines.extend(news.into_iter().take(25));
    }
    lines.join("\n")
}

fn collect_power_changes(
    prev: &BTreeMap<String, ValSnap>,
    next: &BTreeMap<String, ValSnap>,
) -> Vec<crate::terp::digest::PowerChange> {
    let mut changes = Vec::new();
    for (addr, v) in next {
        if !prev.contains_key(addr) {
            changes.push(power_change(
                addr,
                v,
                "joined",
                "",
                &format_uthiol(&v.tokens),
                "",
            ));
        }
    }
    for (addr, v) in prev {
        if !next.contains_key(addr) {
            changes.push(power_change(
                addr,
                v,
                "left",
                &format_uthiol(&v.tokens),
                "",
                "",
            ));
        }
    }
    for (addr, v) in next {
        let Some(old) = prev.get(addr) else { continue };
        if old.jailed != v.jailed {
            let (kind, before, after) = if v.jailed {
                ("jailed", "no", "yes")
            } else {
                ("unjailed", "yes", "no")
            };
            changes.push(power_change(addr, v, kind, before, after, ""));
        }
        if old.status != v.status {
            changes.push(power_change(
                addr,
                v,
                "status",
                short_status(&old.status),
                short_status(&v.status),
                "",
            ));
        }
        if material_token_move(&old.tokens, &v.tokens) {
            changes.push(power_change(
                addr,
                v,
                "stake",
                &format_uthiol(&old.tokens),
                &format_uthiol(&v.tokens),
                &pct_delta(&old.tokens, &v.tokens),
            ));
        }
    }
    changes
}

fn power_change(
    addr: &str,
    v: &ValSnap,
    kind: &str,
    before: &str,
    after: &str,
    delta: &str,
) -> crate::terp::digest::PowerChange {
    crate::terp::digest::PowerChange {
        moniker: v.moniker.clone(),
        address: addr.to_string(),
        kind: kind.to_string(),
        before: before.to_string(),
        after: after.to_string(),
        delta: delta.to_string(),
    }
}

/// Short Discord line plus the Markdown attachment for one UTC day.

fn stake_moves(changes: &[crate::terp::digest::PowerChange]) -> Vec<crate::terp::news::StakeMove> {
    changes
        .iter()
        .filter_map(|c| {
            let action = match c.kind.as_str() {
                "joined" => crate::terp::news::StakeAction::Joined,
                "left" => crate::terp::news::StakeAction::Left,
                "jailed" => crate::terp::news::StakeAction::Jailed,
                "unjailed" => crate::terp::news::StakeAction::Unjailed,
                "stake" if c.delta.trim_start().starts_with('-') => crate::terp::news::StakeAction::Unbonded,
                "stake" => crate::terp::news::StakeAction::Bonded,
                "status" => crate::terp::news::StakeAction::StatusChanged,
                _ => return None,
            };
            Some(crate::terp::news::StakeMove {
                name: c.moniker.clone(),
                action,
            })
        })
        .collect()
}

pub fn voting_power_post(
    day: &str,
    prev_bonded: &str,
    prev: &BTreeMap<String, ValSnap>,
    bonded: &str,
    next: &BTreeMap<String, ValSnap>,
) -> crate::terp::digest::VotingPowerPost {
    let bonded_line = format!(
        "bonded {} → {} ({})",
        format_uthiol(prev_bonded),
        format_uthiol(bonded),
        pct_delta(prev_bonded, bonded)
    );
    crate::terp::digest::render_voting_power(day, &bonded_line, &collect_power_changes(prev, next))
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn short_status(s: &str) -> &str {
    s.rsplit('_').next().unwrap_or(s)
}

fn addr_tail(addr: &str) -> String {
    let n = addr.len();
    if n <= 8 {
        addr.to_string()
    } else {
        format!("…{}", &addr[n - 6..])
    }
}

fn parse_int(s: &str) -> u128 {
    s.chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

fn pct_delta(prev: &str, next: &str) -> String {
    let a = parse_int(prev);
    let b = parse_int(next);
    if a == 0 {
        return "n/a".into();
    }
    let diff = b as i128 - a as i128;
    let pct = (diff as f64) * 100.0 / (a as f64);
    format!("{pct:+.2}%")
}

fn material_token_move(prev: &str, next: &str) -> bool {
    let a = parse_int(prev);
    let b = parse_int(next);
    if a == 0 {
        return b != 0;
    }
    let pct = ((b as i128 - a as i128).unsigned_abs() as f64) * 100.0 / (a as f64);
    pct >= 1.0
}

fn push_summary(msg: &mut String, summary: &str) {
    if !summary.is_empty() {
        msg.push('\n');
        msg.push_str(summary);
    }
}

fn push_link(msg: &mut String, explorer_base: &str, id: &str) {
    msg.push('\n');
    msg.push_str(&proposal_url(explorer_base, id));
}

pub fn period_phrase(status: &str) -> String {
    match status {
        "DEPOSIT_PERIOD" => "deposit period".into(),
        "VOTING_PERIOD" => "voting period".into(),
        "PASSED" => "passed".into(),
        "REJECTED" => "rejected".into(),
        "FAILED" => "failed".into(),
        other => other.to_ascii_lowercase().replace('_', " "),
    }
}

pub fn format_submitted(p: &ProposalNote, explorer_base: &str) -> String {
    let mut msg = format!(
        "**Gov proposal #{id} submitted** {title}\nin the {period}",
        id = p.id,
        title = p.title,
        period = period_phrase(&p.status)
    );
    push_summary(&mut msg, &p.summary);
    push_link(&mut msg, explorer_base, &p.id);
    msg
}

pub fn format_voting_started(p: &ProposalNote, explorer_base: &str) -> String {
    let mut msg = format!(
        "**Gov proposal #{id} entered voting** {title}\nin the {period}",
        id = p.id,
        title = p.title,
        period = period_phrase(&p.status)
    );
    if let Some(end) = p.voting_end {
        msg.push('\n');
        msg.push_str(&format!("voting ends {}", end.format("%Y-%m-%d %H:%M UTC")));
    }
    push_summary(&mut msg, &p.summary);
    push_link(&mut msg, explorer_base, &p.id);
    msg
}

fn format_remaining(secs: i64) -> String {
    let secs = secs.max(0);
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    if h >= 1 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

pub fn format_reminder(p: &ProposalNote, remaining_secs: i64, explorer_base: &str) -> String {
    let mut msg = format!(
        "**Gov proposal #{id}** {title} — {left} left in voting",
        id = p.id,
        title = p.title,
        left = format_remaining(remaining_secs)
    );
    if let Some(end) = p.voting_end {
        msg.push('\n');
        msg.push_str(&format!("voting ends {}", end.format("%Y-%m-%d %H:%M UTC")));
    }
    push_link(&mut msg, explorer_base, &p.id);
    msg
}

fn id_ord(id: &str) -> u128 {
    id.parse().unwrap_or(0)
}

fn reminder_secs(end: DateTime<Utc>, now: DateTime<Utc>) -> Option<i64> {
    let remaining = end.timestamp() - now.timestamp();
    if remaining > 0 && remaining <= REMINDER_WINDOW_SECS {
        Some(remaining)
    } else {
        None
    }
}

/// Deposit submission, deposit → voting, and the one-shot 24h reminder.
///
/// An id already in `seen_proposals` with no `proposal_phase` (state written
/// by an older build) is recorded at its current phase and is not announced
/// again. The 24h reminder still fires once if the vote is inside the window.
pub fn plan_gov_notes(
    state: &mut WatchState,
    proposals: &[ProposalNote],
    now: DateTime<Utc>,
    explorer_base: &str,
) -> Vec<WatchNote> {
    let mut ordered: Vec<&ProposalNote> = proposals.iter().collect();
    ordered.sort_by(|a, b| id_ord(&a.id).cmp(&id_ord(&b.id)).then(a.id.cmp(&b.id)));
    let mut notes = Vec::new();
    for p in ordered {
        let phase = p.status.as_str();
        let seen = state.seen_proposals.contains(&p.id);
        let prev = state.proposal_phase.get(&p.id).map(String::as_str);
        if !seen {
            let text = if phase == "VOTING_PERIOD" {
                format_voting_started(p, explorer_base)
            } else {
                format_submitted(p, explorer_base)
            };
            notes.push(proposal_note(p, text.clone()));
            push_log(state, text.lines().next().unwrap_or(&text));
            state.seen_proposals.insert(p.id.clone());
            state.proposal_phase.insert(p.id.clone(), p.status.clone());
        } else if prev == Some("DEPOSIT_PERIOD") && phase == "VOTING_PERIOD" {
            let text = format_voting_started(p, explorer_base);
            notes.push(proposal_note(p, text.clone()));
            push_log(state, text.lines().next().unwrap_or(&text));
            state.proposal_phase.insert(p.id.clone(), p.status.clone());
        } else if prev != Some(phase) {
            state.proposal_phase.insert(p.id.clone(), p.status.clone());
        }
        if phase == "VOTING_PERIOD" && !state.reminded_24h.contains(&p.id) {
            if let Some(end) = p.voting_end {
                if let Some(secs) = reminder_secs(end, now) {
                    state.reminded_24h.insert(p.id.clone());
                    let text = format_reminder(p, secs, explorer_base);
                    notes.push(proposal_note(p, text.clone()));
                    push_log(state, text.lines().next().unwrap_or(&text));
                }
            }
        }
    }
    notes
}

pub fn counterparty_label(ch: &ChannelNote) -> String {
    let chain = if ch.chain_id.is_empty() {
        "unknown chain".to_string()
    } else {
        ch.chain_id.clone()
    };
    format!("{chain} `{}`", ch.counterparty)
}

pub fn format_channel(ch: &ChannelNote) -> String {
    format!(
        "**New IBC channel** `{}` ↔ {} {}",
        ch.key,
        counterparty_label(ch),
        ch.state
    )
}

pub fn channel_snap_value(ch: &ChannelNote) -> String {
    format!("{}\t{}\t{}", ch.state, ch.counterparty, ch.chain_id)
}

fn snap_state_and_counterparty(prev: &str) -> (&str, &str) {
    let mut parts = prev.split('\t');
    let state = parts.next().unwrap_or("");
    let counterparty = parts.next().unwrap_or("");
    (state, counterparty)
}

pub fn counterparty_known(cp: &str) -> bool {
    let cp = cp.trim();
    !(cp.is_empty() || cp.contains('?') || cp.ends_with('/'))
}

fn push_log(state: &mut WatchState, line: impl Into<String>) {
    if state.day_log.len() >= 40 {
        return;
    }
    let line = line.into();
    let line: String = line.chars().take(160).collect();
    state.day_log.push(line);
}

fn note(kind: NoteKind, text: String) -> WatchNote {
    WatchNote {
        kind,
        text,
        file_name: None,
        file_body: None,
        thread_query: None,
        speech: None,
    }
}

fn proposal_note(p: &ProposalNote, text: String) -> WatchNote {
    WatchNote {
        kind: NoteKind::Proposal,
        text,
        file_name: None,
        file_body: None,
        thread_query: Some(format!("proposal {} {}", p.id, p.title)),
        speech: None,
    }
}

/// New channels, a close after `STATE_OPEN`, and a counterparty that fills in.
/// The first time snaps are missing, record them and stay quiet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientNote {
    pub client_id: String,
    pub chain_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionNote {
    pub id: String,
    pub client_id: String,
}

pub fn parse_connections(v: &Value) -> Vec<ConnectionNote> {
    let Some(list) = v["connections"].as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for row in list {
        let id = row["id"]
            .as_str()
            .or_else(|| row["connection_id"].as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let client_id = row["client_id"].as_str().unwrap_or("").trim().to_string();
        if id.is_empty() || client_id.is_empty() {
            continue;
        }
        out.push(ConnectionNote { id, client_id });
    }
    out
}

/// Fill `chain_id` from connection hop → client → client state.
pub fn attach_counterparty_chains(
    channels: &mut [ChannelNote],
    connections: &[ConnectionNote],
    clients: &[ClientNote],
) {
    let client_of: BTreeMap<&str, &str> = connections
        .iter()
        .map(|c| (c.id.as_str(), c.client_id.as_str()))
        .collect();
    let chain_of: BTreeMap<&str, &str> = clients
        .iter()
        .map(|c| (c.client_id.as_str(), c.chain_id.as_str()))
        .collect();
    for ch in channels.iter_mut() {
        let Some(client) = client_of.get(ch.connection.as_str()) else {
            continue;
        };
        let Some(chain) = chain_of.get(client).copied().filter(|s| !s.is_empty()) else {
            continue;
        };
        ch.chain_id = chain.to_string();
    }
}

pub fn parse_clients(v: &Value) -> Vec<ClientNote> {
    let Some(list) = v["client_states"].as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for row in list {
        let client_id = row["client_id"].as_str().unwrap_or("").trim().to_string();
        if client_id.is_empty() {
            continue;
        }
        let chain_id = row["client_state"]["chain_id"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        out.push(ClientNote {
            client_id,
            chain_id,
        });
    }
    out
}

/// New IBC clients. The first poll records the current set and stays quiet,
/// including when that set is empty.
pub fn plan_client_notes(state: &mut WatchState, clients: &[ClientNote]) -> Vec<WatchNote> {
    if !state.clients_seeded {
        for client in clients {
            state.seen_clients.insert(client.client_id.clone());
        }
        state.clients_seeded = true;
        info!(
            clients = state.seen_clients.len(),
            "ibc clients seeded (no historical alerts)"
        );
        return Vec::new();
    }
    let mut notes = Vec::new();
    for client in clients {
        if !state.seen_clients.insert(client.client_id.clone()) {
            continue;
        }
        let chain = if client.chain_id.is_empty() {
            String::new()
        } else {
            format!(" chain `{}`", client.chain_id)
        };
        let text = format!("**New IBC client** `{}`{chain}", client.client_id);
        push_log(state, format!("client {}", client.client_id));
        notes.push(note(NoteKind::Channel, text));
    }
    notes
}

pub fn plan_channel_notes(state: &mut WatchState, channels: &[ChannelNote]) -> Vec<WatchNote> {
    if state.seen_channels.is_empty() || state.channel_snap.is_empty() {
        for ch in channels {
            state.seen_channels.insert(ch.key.clone());
            state.channel_snap
                .insert(ch.key.clone(), channel_snap_value(ch));
        }
        info!(
            channels = state.seen_channels.len(),
            "ibc channels seeded (no historical alerts)"
        );
        return Vec::new();
    }
    let mut notes = Vec::new();
    for ch in channels {
        match state.channel_snap.get(&ch.key).cloned() {
            None => {
                let text = format_channel(ch);
                push_log(state, format!("opened {}", ch.key));
                notes.push(note(NoteKind::Channel, text));
            }
            Some(prev) => {
                let (old_state, old_cp) = snap_state_and_counterparty(&prev);
                if old_state.contains("OPEN") && ch.state.contains("CLOSED") {
                    let text = format!("**IBC channel closed** `{}`", ch.key);
                    push_log(state, format!("closed {}", ch.key));
                    notes.push(note(NoteKind::Channel, text));
                } else if !counterparty_known(old_cp) && counterparty_known(&ch.counterparty) {
                    let text = format!(
                        "**IBC channel** `{}` ↔ {} {}",
                        ch.key,
                        counterparty_label(ch),
                        ch.state
                    );
                    push_log(state, format!("counterparty {} {}", ch.key, ch.counterparty));
                    notes.push(note(NoteKind::Channel, text));
                }
            }
        }
        state.seen_channels.insert(ch.key.clone());
        state
            .channel_snap
            .insert(ch.key.clone(), channel_snap_value(ch));
    }
    notes
}

pub fn format_resolved(p: &ProposalNote, explorer_base: &str) -> String {
    let word = match p.status.as_str() {
        "PASSED" => "passed",
        "REJECTED" => "rejected",
        "FAILED" => "failed",
        other => other,
    };
    let mut msg = format!(
        "**Gov proposal #{id} {word}** {title}",
        id = p.id,
        title = p.title
    );
    push_link(&mut msg, explorer_base, &p.id);
    msg
}

/// One countdown when the remaining blocks first fall under 10_000, 1_000, or 100.
/// Wider steps are marked seen so a late start does not also announce them.
pub fn halt_notice(
    name: &str,
    halt: u64,
    height: u64,
    notices: &mut BTreeSet<String>,
) -> Option<String> {
    if height == 0 || halt == 0 || height >= halt {
        return None;
    }
    let remaining = halt - height;
    const STEPS: [u64; 3] = [10_000, 1_000, 100];
    let step = STEPS.into_iter().filter(|s| remaining <= *s).min()?;
    let key = format!("{name}@{halt}@{step}");
    if notices.contains(&key) {
        return None;
    }
    notices.insert(key);
    for wider in STEPS {
        if wider > step {
            notices.insert(format!("{name}@{halt}@{wider}"));
        }
    }
    Some(format!(
        "**Upgrade `{name}`** halt at height `{halt}`, about {remaining} blocks left (under {step})"
    ))
}

pub fn parse_upgrade_plan(v: &Value) -> Option<(String, u64)> {
    let name = v["plan"]["name"].as_str().unwrap_or("").trim().to_string();
    let height = json_scalar(&v["plan"]["height"]);
    let height = height.parse::<u64>().ok()?;
    if name.is_empty() || height == 0 {
        None
    } else {
        Some((name, height))
    }
}

pub fn parse_block_height(v: &Value) -> Option<u64> {
    json_scalar(&v["block"]["header"]["height"]).parse().ok()
}

pub async fn fetch_json(client: &reqwest::Client, url: &str) -> Result<Value> {
    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    let text = resp.text().await.context("read body")?;
    if !status.is_success() {
        anyhow::bail!("HTTP {status} {url}");
    }
    serde_json::from_str(&text).context("json")
}

async fn list_proposals_with_status(
    client: &reqwest::Client,
    rest: &str,
    status: u8,
) -> Result<Vec<ProposalNote>> {
    let mut out = Vec::new();
    let mut key = String::new();
    for _ in 0..4 {
        let mut url = format!(
            "{}/cosmos/gov/v1/proposals?proposal_status={status}&pagination.limit=50",
            rest.trim_end_matches('/')
        );
        if !key.is_empty() {
            url.push_str("&pagination.key=");
            url.push_str(&url_encode(&key));
        }
        let v = fetch_json(client, &url).await?;
        out.extend(parse_proposals(&v));
        key = v["pagination"]["next_key"]
            .as_str()
            .unwrap_or("")
            .to_string();
        if key.is_empty() {
            break;
        }
    }
    Ok(out)
}

/// Deposit-period (`proposal_status=1`) and voting-period (`proposal_status=2`)
/// proposals. Voting wins if an id shows up in both responses.
///
/// This does not use an unfiltered `pagination.reverse` window. That window is
/// the oldest ids when Cosmos SDK's filtered paginate ignores `reverse`, so a
/// new proposal never appears. Status queries stay small either way.
pub async fn list_proposals(client: &reqwest::Client, rest: &str) -> Result<Vec<ProposalNote>> {
    let mut by_id = BTreeMap::new();
    for status in [1u8, 2u8] {
        for p in list_proposals_with_status(client, rest, status).await? {
            by_id.insert(p.id.clone(), p);
        }
    }
    Ok(by_id.into_values().collect())
}

pub async fn list_clients(client: &reqwest::Client, rest: &str) -> Result<Vec<ClientNote>> {
    let mut out = Vec::new();
    let mut key = String::new();
    for _ in 0..8 {
        let mut url = format!(
            "{}/ibc/core/client/v1/client_states?pagination.limit=200",
            rest.trim_end_matches('/')
        );
        if !key.is_empty() {
            url.push_str("&pagination.key=");
            url.push_str(&url_encode(&key));
        }
        let v = fetch_json(client, &url).await?;
        out.extend(parse_clients(&v));
        key = v["pagination"]["next_key"]
            .as_str()
            .unwrap_or("")
            .to_string();
        if key.is_empty() {
            break;
        }
    }
    Ok(out)
}

pub async fn list_connections(
    client: &reqwest::Client,
    rest: &str,
) -> Result<Vec<ConnectionNote>> {
    let mut out = Vec::new();
    let mut key = String::new();
    for _ in 0..8 {
        let mut url = format!(
            "{}/ibc/core/connection/v1/connections?pagination.limit=200",
            rest.trim_end_matches('/')
        );
        if !key.is_empty() {
            url.push_str("&pagination.key=");
            url.push_str(&url_encode(&key));
        }
        let v = fetch_json(client, &url).await?;
        out.extend(parse_connections(&v));
        key = v["pagination"]["next_key"]
            .as_str()
            .unwrap_or("")
            .to_string();
        if key.is_empty() {
            break;
        }
    }
    Ok(out)
}

pub async fn list_channels(client: &reqwest::Client, rest: &str) -> Result<Vec<ChannelNote>> {
    let mut out = Vec::new();
    let mut key = String::new();
    for _ in 0..8 {
        let mut url = format!(
            "{}/ibc/core/channel/v1/channels?pagination.limit=100",
            rest.trim_end_matches('/')
        );
        if !key.is_empty() {
            url.push_str("&pagination.key=");
            url.push_str(&url_encode(&key));
        }
        let v = fetch_json(client, &url).await?;
        out.extend(parse_channels(&v));
        key = v["pagination"]["next_key"]
            .as_str()
            .unwrap_or("")
            .to_string();
        if key.is_empty() {
            break;
        }
    }
    Ok(out)
}

pub async fn staking_snapshot(
    client: &reqwest::Client,
    rest: &str,
) -> Result<(String, BTreeMap<String, ValSnap>)> {
    let base = rest.trim_end_matches('/');
    let pool = fetch_json(client, &format!("{base}/cosmos/staking/v1beta1/pool")).await?;
    let mut vals = BTreeMap::new();
    let mut key = String::new();
    for _ in 0..6 {
        let mut url = format!("{base}/cosmos/staking/v1beta1/validators?pagination.limit=200");
        if !key.is_empty() {
            url.push_str("&pagination.key=");
            url.push_str(&url_encode(&key));
        }
        let v = fetch_json(client, &url).await?;
        vals.extend(parse_validators(&v));
        key = v["pagination"]["next_key"]
            .as_str()
            .unwrap_or("")
            .to_string();
        if key.is_empty() {
            break;
        }
    }
    Ok((parse_pool_bonded(&pool), vals))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteKind {
    Proposal,
    Channel,
    Stake,
    Baseline,
    /// Upgrade countdown and other activity-channel lines.
    Notice,
}

#[derive(Debug)]
pub struct WatchNote {
    pub kind: NoteKind,
    pub text: String,
    /// Set for the daily stake post. `text` stays the short summary.
    pub file_name: Option<String>,
    pub file_body: Option<String>,
    /// Proposal id and title. The poster opens a thread, and `/edgar ask`
    /// in that thread prepends this string to the question.
    pub thread_query: Option<String>,
    /// Spoken clip. Discord still shows `text`.
    pub speech: Option<String>,
}

/// One poll. On the first successful seed, returns a baseline line and does
/// not replay proposals or channels already on chain. A proposal already
/// inside the 24h voting window still gets one reminder. An older state file
/// with no `seen_channels` records the current channels once and stays quiet.
pub async fn tick(
    client: &reqwest::Client,
    rest: &str,
    state: &mut WatchState,
    today: &str,
) -> Result<Vec<WatchNote>> {
    let proposals = list_proposals(client, rest).await?;
    let mut channels = list_channels(client, rest).await?;
    let clients = list_clients(client, rest).await?;
    match list_connections(client, rest).await {
        Ok(connections) => attach_counterparty_chains(&mut channels, &connections, &clients),
        Err(e) => tracing::warn!("ibc connection list failed; channel notes omit chain id: {e:#}"),
    }
    let (bonded, vals) = staking_snapshot(client, rest).await?;
    let now = Utc::now();
    let explorer = explorer_base();

    if !state.seeded {
        for p in &proposals {
            state.seen_proposals.insert(p.id.clone());
            state.proposal_phase.insert(p.id.clone(), p.status.clone());
        }
        for ch in &channels {
            state.seen_channels.insert(ch.key.clone());
            state
                .channel_snap
                .insert(ch.key.clone(), channel_snap_value(ch));
        }
        state.bonded = bonded;
        state.validators = vals;
        state.last_stake_day = today.to_string();
        state.seeded = true;
        info!(
            proposals = state.seen_proposals.len(),
            channels = state.seen_channels.len(),
            validators = state.validators.len(),
            "chain watch seeded (no historical alerts)"
        );
        let mut notes = vec![WatchNote {
            kind: NoteKind::Baseline,
            text: format!(
            "chain watch baseline: {} open proposals, {} IBC channels, {} validators, bonded {}. next voting-power digest after `{today}`.",
            state.seen_proposals.len(),
            state.seen_channels.len(),
            state.validators.len(),
            format_uthiol(&state.bonded)
        ),
            file_name: None,
            file_body: None,
            thread_query: None,
            speech: None,
        }];
        // Phases were just recorded, so this does not replay submissions.
        // It does emit a 24h reminder for a vote already inside the window.
        notes.extend(plan_gov_notes(state, &proposals, now, &explorer));
        let _ = plan_client_notes(state, &clients);
        return Ok(notes);
    }

    let mut notes = Vec::new();
    notes.extend(plan_gov_notes(state, &proposals, now, &explorer));
    notes.extend(resolve_left_proposals(client, rest, state, &proposals, &explorer).await?);
    notes.extend(plan_channel_notes(state, &channels));
    notes.extend(plan_client_notes(state, &clients));
    if let Some(text) = upgrade_countdown(client, rest, state).await {
        push_log(state, text.lines().next().unwrap_or(&text));
        notes.push(note(NoteKind::Notice, text));
    }
    if state.last_stake_day != today {
        let post = voting_power_post(today, &state.bonded, &state.validators, &bonded, &vals);
        let stake_changes = post.inline.lines().nth(2).unwrap_or("").to_string();
        let bonded_line = post.inline.lines().nth(1).unwrap_or("").to_string();
        let jail = std::fs::read_to_string("./data/closest-jail.txt").unwrap_or_default();
        let dao_lines = crate::terp::news::take_lines(std::path::Path::new("./data/desk-dao.log"));
        let (contribs, rejected) =
            crate::terp::news::take_inbox(std::path::Path::new("./data/desk-inbox"));
        let moves = stake_moves(&collect_power_changes(&state.validators, &vals));
        let desk = crate::terp::news::compose_desk(
            today,
            &bonded_line,
            &stake_changes,
            &state.day_log,
            &dao_lines,
            &contribs,
            &rejected,
            jail.trim(),
            &moves,
        );
        let text = desk.caption.clone();
        let mut markdown = desk.discord.clone();
        markdown.push_str("\n\n");
        markdown.push_str(&post.markdown);
        state.day_log.clear();
        notes.push(WatchNote {
            kind: NoteKind::Stake,
            text,
            file_name: Some(format!("terp-desk-{today}.md")),
            file_body: Some(markdown),
            thread_query: None,
            speech: Some(desk.speech),
        });
        state.bonded = bonded;
        state.validators = vals;
        state.last_stake_day = today.to_string();
    }
    Ok(notes)
}

async fn resolve_left_proposals(
    client: &reqwest::Client,
    rest: &str,
    state: &mut WatchState,
    active: &[ProposalNote],
    explorer: &str,
) -> Result<Vec<WatchNote>> {
    let open: BTreeSet<String> = active.iter().map(|p| p.id.clone()).collect();
    let pending: Vec<String> = state
        .proposal_phase
        .iter()
        .filter(|(id, phase)| {
            matches!(phase.as_str(), "DEPOSIT_PERIOD" | "VOTING_PERIOD") && !open.contains(*id)
        })
        .map(|(id, _)| id.clone())
        .take(5)
        .collect();
    let mut notes = Vec::new();
    for id in pending {
        let Some(p) = fetch_proposal(client, rest, &id).await? else {
            continue;
        };
        if matches!(p.status.as_str(), "PASSED" | "REJECTED" | "FAILED") {
            let text = format_resolved(&p, explorer);
            push_log(state, text.lines().next().unwrap_or(&text));
            notes.push(proposal_note(&p, text));
        }
        state.proposal_phase.insert(p.id, p.status);
    }
    Ok(notes)
}

async fn fetch_proposal(
    client: &reqwest::Client,
    rest: &str,
    id: &str,
) -> Result<Option<ProposalNote>> {
    let url = format!(
        "{}/cosmos/gov/v1/proposals/{}",
        rest.trim_end_matches('/'),
        id
    );
    let v = match fetch_json(client, &url).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("proposal {id} lookup failed: {e:#}");
            return Ok(None);
        }
    };
    let wrapped = serde_json::json!({ "proposals": [v.get("proposal").cloned().unwrap_or(v)] });
    Ok(parse_proposals(&wrapped).into_iter().next())
}

async fn upgrade_countdown(
    client: &reqwest::Client,
    rest: &str,
    state: &mut WatchState,
) -> Option<String> {
    let base = rest.trim_end_matches('/');
    let plan = fetch_json(client, &format!("{base}/cosmos/upgrade/v1beta1/current_plan"))
        .await
        .ok()?;
    let (name, halt) = parse_upgrade_plan(&plan)?;
    let block = fetch_json(
        client,
        &format!("{base}/cosmos/base/tendermint/v1beta1/blocks/latest"),
    )
    .await
    .ok()?;
    let height = parse_block_height(&block)?;
    halt_notice(&name, halt, height, &mut state.upgrade_notices)
}

pub fn remember_thread(id: u64, prefix: &str) {
    let path = std::path::PathBuf::from("./data/alert-threads.json");
    let mut map: BTreeMap<String, String> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    if map.len() >= 200 {
        if let Some(oldest) = map.keys().next().cloned() {
            map.remove(&oldest);
        }
    }
    map.insert(id.to_string(), prefix.chars().take(300).collect());
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(body) = serde_json::to_vec(&map) {
        let _ = std::fs::write(path, body);
    }
}

pub fn thread_prefix(id: u64) -> Option<String> {
    let raw = std::fs::read_to_string("./data/alert-threads.json").ok()?;
    let map: BTreeMap<String, String> = serde_json::from_str(&raw).ok()?;
    map.get(&id.to_string()).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_ids_and_upgrade_title() {
        let v: Value = serde_json::json!({
            "proposals": [{
                "id": "60",
                "status": "PROPOSAL_STATUS_DEPOSIT_PERIOD",
                "messages": [{"@type": "/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade", "plan": {"name": "v6.2"}}]
            }]
        });
        let ps = parse_proposals(&v);
        assert_eq!(ps[0].title, "upgrade v6.2");
        let seen = BTreeSet::from(["59".into()]);
        assert_eq!(
            newly(&seen, ps.into_iter().map(|p| p.id)),
            vec!["60".to_string()]
        );
    }

    #[test]
    fn staking_digest_names_jail_and_skips_dust() {
        let mut prev = BTreeMap::new();
        prev.insert(
            "terpvaloper1abc".into(),
            ValSnap {
                moniker: "alpha".into(),
                status: "BOND_STATUS_BONDED".into(),
                jailed: false,
                tokens: "1000".into(),
            },
        );
        let mut next = prev.clone();
        next.get_mut("terpvaloper1abc").unwrap().jailed = true;
        next.get_mut("terpvaloper1abc").unwrap().tokens = "1004".into();
        let text = staking_digest("2026-09-15", "10000", &prev, "11000", &next);
        assert!(text.contains("jailed alpha"));
        assert!(text.contains("0.01 TERP"));
        assert!(text.contains("0.011 TERP"));
        assert!(text.contains("+10.00%"));
        assert!(!text.contains("0.001004"));
    }

    #[test]
    fn voting_power_post_attaches_markdown_and_skips_dust() {
        let mut prev = BTreeMap::new();
        prev.insert(
            "terpvaloper1abc".into(),
            ValSnap {
                moniker: "alpha".into(),
                status: "BOND_STATUS_BONDED".into(),
                jailed: false,
                tokens: "1000".into(),
            },
        );
        let mut next = prev.clone();
        next.get_mut("terpvaloper1abc").unwrap().jailed = true;
        next.get_mut("terpvaloper1abc").unwrap().tokens = "1004".into();
        let post = voting_power_post("2026-09-15", "10000", &prev, "11000", &next);
        assert!(post.inline.contains("jailed alpha"));
        assert!(post.inline.contains("+10.00%"));
        assert!(!post.inline.contains("terpvaloper1abc"));
        assert!(post.filename.ends_with(".md"));
        assert!(post
            .markdown
            .contains("| alpha | `terpvaloper1abc` | jailed |"));
        assert!(!post.markdown.contains("0.001004"));
        assert!(post.inline.len() < post.markdown.len());

        next.get_mut("terpvaloper1abc").unwrap().tokens = "2000".into();
        let post = voting_power_post("2026-09-15", "10000", &prev, "11000", &next);
        assert!(post.markdown.contains("| stake |"));
        assert!(post.markdown.contains("+100.00%"));
    }

    #[test]
    fn uthiol_displays_as_terp() {
        assert_eq!(format_uthiol("100000000"), "100 TERP");
        assert_eq!(format_uthiol("7228319733183"), "7,228,319.733183 TERP");
        assert_eq!(format_uthiol("0"), "0 TERP");
    }

    #[test]
    fn channel_key_is_port_and_id() {
        let v = serde_json::json!({
            "channels": [{
                "state": "STATE_OPEN",
                "port_id": "transfer",
                "channel_id": "channel-11",
                "counterparty": {"port_id": "transfer", "channel_id": "channel-9"}
            }]
        });
        let ch = parse_channels(&v);
        assert_eq!(ch[0].key, "transfer/channel-11");
        assert!(format_channel(&ch[0]).contains("transfer/channel-9"));
    }

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn sample(id: &str, status: &str, end: Option<&str>) -> ProposalNote {
        ProposalNote {
            id: id.into(),
            title: format!("title {id}"),
            status: status.into(),
            summary: "sum".into(),
            voting_end: end.map(ts),
        }
    }

    const BASE: &str = "https://ping.pub/terp/gov";

    #[test]
    fn parses_numeric_id_status_and_ignores_zero_voting_end() {
        let v = serde_json::json!({
            "proposals": [{
                "id": 62,
                "status": 2,
                "title": "Software upgrade v6.3, then v6.4",
                "summary": "halt",
                "voting_end_time": "2026-10-15T08:29:24.429888170Z"
            }, {
                "id": "9",
                "status": "PROPOSAL_STATUS_DEPOSIT_PERIOD",
                "voting_end_time": "0001-01-01T00:00:00Z"
            }]
        });
        let ps = parse_proposals(&v);
        assert_eq!(ps[0].id, "62");
        assert_eq!(ps[0].status, "VOTING_PERIOD");
        assert_eq!(ps[0].voting_end, Some(ts("2026-10-15T08:29:24.429888170Z")));
        assert_eq!(ps[1].status, "DEPOSIT_PERIOD");
        assert!(ps[1].voting_end.is_none());
    }

    #[test]
    fn explorer_base_trims_and_defaults() {
        assert_eq!(
            resolve_explorer_base(Some(" https://example.test/gov/ ")),
            "https://example.test/gov"
        );
        assert_eq!(resolve_explorer_base(Some("")), DEFAULT_EXPLORER_BASE);
        assert_eq!(resolve_explorer_base(None), DEFAULT_EXPLORER_BASE);
        assert_eq!(
            proposal_url("https://ping.pub/terp/gov/", "62"),
            "https://ping.pub/terp/gov/62"
        );
    }

    #[test]
    fn deposit_then_voting_then_one_reminder_survives_restart() {
        let mut state = WatchState {
            seeded: true,
            ..WatchState::default()
        };
        let deposit = sample("63", "DEPOSIT_PERIOD", None);
        let now = ts("2026-10-14T08:00:00Z");
        let n = plan_gov_notes(&mut state, &[deposit], now, BASE);
        assert_eq!(n.len(), 1);
        assert!(n[0].text.contains("submitted"));
        assert!(n[0].text.contains("https://ping.pub/terp/gov/63"));
        assert!(plan_gov_notes(&mut state, &[sample("63", "DEPOSIT_PERIOD", None)], now, BASE).is_empty());

        let voting = sample("63", "VOTING_PERIOD", Some("2026-10-18T08:00:00Z"));
        let n = plan_gov_notes(&mut state, &[voting.clone()], now, BASE);
        assert_eq!(n.len(), 1, "{n:?}");
        assert!(n[0].text.contains("entered voting"));
        assert!(n[0].text.contains("https://ping.pub/terp/gov/63"));
        assert!(!n[0].text.contains("left in voting"));

        let late = ts("2026-10-17T08:00:00Z");
        let n = plan_gov_notes(&mut state, &[voting.clone()], late, BASE);
        assert_eq!(n.len(), 1);
        assert!(n[0].text.contains("left in voting"));
        assert!(n[0].text.contains("https://ping.pub/terp/gov/63"));
        assert!(plan_gov_notes(&mut state, &[voting.clone()], late, BASE).is_empty());

        let dumped = serde_json::to_string(&state).unwrap();
        let mut restored: WatchState = serde_json::from_str(&dumped).unwrap();
        assert!(restored.reminded_24h.contains("63"));
        assert!(plan_gov_notes(&mut restored, &[voting], late, BASE).is_empty());
    }

    #[test]
    fn twenty_four_hour_vote_reminds_on_first_sight_and_not_before_window() {
        let mut state = WatchState {
            seeded: true,
            ..WatchState::default()
        };
        let end = "2026-10-15T08:29:24Z";
        let start = ts("2026-10-14T08:29:24Z");
        let p = sample("62", "VOTING_PERIOD", Some(end));
        let n = plan_gov_notes(&mut state, &[p.clone()], start, BASE);
        assert_eq!(n.len(), 2);
        assert!(n[0].text.contains("entered voting"));
        assert!(n[1].text.contains("24h 0m left in voting"));
        assert!(n[1].text.contains("https://ping.pub/terp/gov/62"));

        let mut later = WatchState {
            seeded: true,
            ..WatchState::default()
        };
        let early = sample("70", "VOTING_PERIOD", Some("2026-10-18T08:00:01Z"));
        let n = plan_gov_notes(&mut later, &[early], ts("2026-10-14T08:00:00Z"), BASE);
        assert_eq!(n.len(), 1);
        assert!(n[0].text.contains("entered voting"));
        assert!(!later.reminded_24h.contains("70"));
    }

    #[test]
    fn already_seen_without_phase_does_not_replay_but_can_remind() {
        let raw = r#"{"seeded":true,"seen_proposals":["62"],"seen_channels":[],"last_stake_day":"2026-10-14","bonded":"0","validators":{}}"#;
        let mut state: WatchState = serde_json::from_str(raw).unwrap();
        assert!(state.proposal_phase.is_empty());
        let p = sample("62", "VOTING_PERIOD", Some("2026-10-15T08:29:24Z"));
        let n = plan_gov_notes(&mut state, &[p], ts("2026-10-14T20:29:24Z"), BASE);
        assert_eq!(n.len(), 1);
        assert!(n[0].text.contains("left in voting"));
        assert!(!n[0].text.contains("entered voting"));
        assert_eq!(state.proposal_phase.get("62").map(String::as_str), Some("VOTING_PERIOD"));
    }

    #[test]
    fn ended_vote_does_not_remind() {
        let mut state = WatchState {
            seeded: true,
            seen_proposals: BTreeSet::from(["62".into()]),
            ..WatchState::default()
        };
        state.proposal_phase.insert("62".into(), "VOTING_PERIOD".into());
        let p = sample("62", "VOTING_PERIOD", Some("2026-10-15T08:29:24Z"));
        let n = plan_gov_notes(&mut state, &[p], ts("2026-10-15T08:29:24Z"), BASE);
        assert!(n.is_empty());
        assert!(!state.reminded_24h.contains("62"));
    }

    #[test]
    fn init_channel_names_chain_then_channel_id() {
        let mut state = WatchState::default();
        state.seen_channels.insert("transfer/channel-0".into());
        state.channel_snap.insert(
            "transfer/channel-0".into(),
            "STATE_OPEN\ttransfer/channel-1\tosmosis-1".into(),
        );
        let init = ChannelNote {
            key: "transfer/channel-12".into(),
            counterparty: "transfer/".into(),
            state: "STATE_INIT".into(),
            chain_id: "osmosis-1".into(),
            connection: "connection-12".into(),
        };
        let notes = plan_channel_notes(&mut state, &[init.clone()]);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].text.contains("osmosis-1"));
        assert!(notes[0].text.contains("transfer/"));
        assert!(notes[0].text.contains("STATE_INIT"));
        let opened = ChannelNote {
            counterparty: "transfer/channel-441".into(),
            state: "STATE_OPEN".into(),
            ..init
        };
        let notes = plan_channel_notes(&mut state, &[opened.clone()]);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].text.contains("channel-441"));
        assert!(notes[0].text.contains("osmosis-1"));
        assert!(plan_channel_notes(&mut state, &[opened]).is_empty());
    }

    #[test]
    fn channel_close_and_counterparty_fill_once() {
        let open = ChannelNote {
            key: "transfer/channel-1".into(),
            counterparty: "transfer/?".into(),
            state: "STATE_OPEN".into(),
            chain_id: "osmosis-1".into(),
            connection: "connection-1".into(),
        };
        let mut state = WatchState::default();
        state.seen_channels.insert(open.key.clone());
        state
            .channel_snap
            .insert(open.key.clone(), channel_snap_value(&open));
        let filled = ChannelNote {
            counterparty: "transfer/channel-9".into(),
            ..open.clone()
        };
        let notes = plan_channel_notes(&mut state, &[filled.clone()]);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].text.contains("channel-9"));
        assert!(notes[0].text.contains("osmosis-1"));
        assert!(plan_channel_notes(&mut state, &[filled]).is_empty());
        let closed = ChannelNote {
            state: "STATE_CLOSED".into(),
            counterparty: "transfer/channel-9".into(),
            ..open
        };
        let notes = plan_channel_notes(&mut state, &[closed]);
        assert!(notes[0].text.contains("closed"));
    }

    #[test]
    fn halt_announces_tightest_step_once() {
        let mut seen = BTreeSet::new();
        let first = halt_notice("v6.2", 1000, 950, &mut seen).unwrap();
        assert!(first.contains("under 100"));
        assert!(halt_notice("v6.2", 1000, 950, &mut seen).is_none());
        assert!(halt_notice("v6.2", 1000, 500, &mut seen).is_none());
        let far = halt_notice("v6.3", 20_000, 5_000, &mut BTreeSet::new());
        assert!(far.is_none());
        let entered = halt_notice("v6.3", 20_000, 12_000, &mut BTreeSet::new()).unwrap();
        assert!(entered.contains("under 10000"));
    }

    #[test]
    fn resolved_copy_names_the_outcome() {
        let p = sample("70", "PASSED", None);
        let text = format_resolved(&p, "https://ping.pub/terp/gov");
        assert!(text.contains("passed"));
        assert!(text.contains("https://ping.pub/terp/gov/70"));
        let note = proposal_note(&p, text);
        assert_eq!(
            note.thread_query.as_deref(),
            Some("proposal 70 title 70")
        );
    }

    #[test]
    fn new_client_is_quiet_on_seed_then_announced() {
        let mut state = WatchState::default();
        let existing = ClientNote {
            client_id: "07-tendermint-1".into(),
            chain_id: "osmosis-1".into(),
        };
        assert!(plan_client_notes(&mut state, &[existing.clone()]).is_empty());
        assert!(state.clients_seeded);
        let fresh = ClientNote {
            client_id: "07-tendermint-99".into(),
            chain_id: "noble-1".into(),
        };
        let notes = plan_client_notes(&mut state, &[existing.clone(), fresh]);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].text.contains("07-tendermint-99"));
        assert!(notes[0].text.contains("noble-1"));
        assert!(plan_client_notes(&mut state, &[existing]).is_empty());
        let parsed = parse_clients(&serde_json::json!({
            "client_states": [{
                "client_id": "07-tendermint-3",
                "client_state": {"chain_id": "akashnet-2"}
            }]
        }));
        assert_eq!(parsed[0].client_id, "07-tendermint-3");
        assert_eq!(parsed[0].chain_id, "akashnet-2");
    }
}
