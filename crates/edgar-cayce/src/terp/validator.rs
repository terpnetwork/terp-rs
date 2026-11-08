//! Cosmos validator monitor — automated liveness watchdog.
//!
//! Design (ported from the astrostake `discord-cosmos-validator-monitor`
//! cogs, extended the way validator-team bots do it):
//! - A background task polls the full bonded set + signing infos on an
//!   interval and posts **automated messages** to an alert channel. There is
//!   Slash, jail, and unjail are their own posts on the poll that sees
//!   the edge. `/validator list` stays the read model.
//! - Missed blocks are attributed exactly: ed25519 consensus pubkey →
//!   the first 20 bytes of `SHA256(pubkey)` → bech32 `{prefix}valcons` →
//!   signing-info lookup. That is the Tendermint consensus address.
//!   `RIPEMD160(SHA256)` is the account-address formula and matches none
//!   of the signing infos.
//! - Missed-block bands warn before the jail line. A jail, unjail,
//!   tombstone, or slash event posts on its own, even when the band did
//!   not move.

use std::collections::{HashMap, VecDeque};

use base64::Engine;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct ChainConfig {
    pub name: String,
    pub rest: String,
    pub prefix: String,
    #[serde(default = "default_missed")]
    pub missed_threshold: u64,
    #[serde(default)]
    pub validators: Vec<String>,
}

fn default_missed() -> u64 {
    50
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidatorSnapshot {
    pub valoper: String,
    pub moniker: String,
    pub jailed: bool,
    pub status: String,
    pub tokens: String,
    pub voting_pct: f64,
    pub missed_blocks: u64,
    pub tombstoned: bool,
    pub height: i64,
    /// False when the consensus pubkey did not match a signing info.
    /// A zero counter with this flag clear is not a healthy validator.
    #[serde(default)]
    pub attributed: bool,
    /// Bech32 consensus address. Empty when the pubkey did not decode.
    /// Slash events name this address.
    #[serde(default)]
    pub cons: String,
}

#[derive(Debug, Clone, Default)]
pub struct ValidatorAlert {
    pub chain: String,
    pub valoper: String,
    pub kind: &'static str,
    pub message: String,
    pub critical: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SlashingParams {
    pub signed_blocks_window: i64,
    pub min_signed_per_window: f64,
}

impl SlashingParams {
    pub fn allowed_missed(&self) -> u64 {
        let w = self.signed_blocks_window.max(1) as f64;
        (w * (1.0 - self.min_signed_per_window)).floor().max(1.0) as u64
    }
}

// ── address derivation ──────────────────────────────────────────────────

use sha2::{Digest, Sha256};

/// Tendermint address: RIPEMD160(SHA256(pubkey)).
fn tm_address(pubkey: &[u8]) -> [u8; 20] {
    // Tendermint consensus address: SHA256(ed25519 pubkey)[..20].
    let sha = Sha256::digest(pubkey);
    let mut out = [0u8; 20];
    out.copy_from_slice(&sha[..20]);
    out
}

/// Minimal bech32 encode (hand-rolled to avoid an extra dep).
fn bech32_encode(hrp: &str, data: &[u8]) -> String {
    const CHARSET: &[u8] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";
    // convertbits 8 -> 5
    let mut acc = 0u32;
    let mut bits = 0u32;
    let mut out: Vec<u8> = Vec::new();
    for &b in data {
        acc = (acc << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push((acc >> bits) as u8 & 31);
        }
    }
    if bits > 0 {
        out.push((acc << (5 - bits)) as u8 & 31);
    }
    // checksum: polymod(hrp_expand ++ data ++ [0;6]) ^ 1
    let mut values: Vec<u8> = hrp.bytes().map(|b| b >> 5).collect();
    values.push(0);
    values.extend(hrp.bytes().map(|b| b & 31));
    values.extend_from_slice(&out);
    values.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    let mut polymod = 1u32;
    for v in &values {
        let b = polymod >> 25;
        polymod = ((polymod & 0x1ff_ffff) << 5) ^ (*v as u32);
        // BIP173 generators. The fourth word is 0x3d4233dd, not 0x3d423dd7.
        for (i, c) in [0x3b6a_57b2, 0x2650_8e6d, 0x1ea1_19fa, 0x3d42_33dd, 0x2a14_62b3]
            .iter()
            .enumerate()
        {
            if (b >> i) & 1 == 1 {
                polymod ^= c;
            }
        }
    }
    polymod ^= 1;
    let mut s = format!("{hrp}1");
    for v in out {
        s.push(CHARSET[v as usize] as char);
    }
    for i in 0..6 {
        s.push(CHARSET[((polymod >> (5 * (5 - i))) & 31) as usize] as char);
    }
    s
}

/// consensus ed25519 pubkey (base64) → `{prefix}valcons...` address.
pub fn cons_address(prefix: &str, pubkey_b64: &str) -> Option<String> {
    use base64::Engine;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(pubkey_b64)
        .ok()?;
    // consensus_pubkey is a protobuf Any wrapping ed25519 bytes; take the
    // last 32 bytes (the raw key) when longer.
    let key = if raw.len() > 32 { &raw[raw.len() - 32..] } else { &raw[..] };
    if key.len() != 32 {
        return None;
    }
    Some(bech32_encode(&format!("{prefix}valcons"), &tm_address(key)))
}

// ── fetching ────────────────────────────────────────────────────────────

fn rest(chain_rest: &str, path: &str) -> String {
    format!("{}{}", chain_rest.trim_end_matches('/'), path)
}

async fn get_json(client: &reqwest::Client, url: &str) -> anyhow::Result<serde_json::Value> {
    Ok(client
        .get(url)
        .timeout(Duration::from_secs(15))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

/// Latest height (for alert context).
pub async fn latest_height(client: &reqwest::Client, chain_rest: &str) -> Option<i64> {
    get_json(client, &rest(chain_rest, "/cosmos/base/tendermint/v1beta1/blocks/latest"))
        .await
        .ok()?
        .pointer("/block/header/height")
        .and_then(|h| h.as_str()?.parse().ok())
}

/// Chain slashing params (for the ETA math).
pub async fn slashing_params(
    client: &reqwest::Client,
    chain_rest: &str,
) -> SlashingParams {
    let d = SlashingParams::default();
    let Ok(v) = get_json(client, &rest(chain_rest, "/cosmos/slashing/v1beta1/params")).await
    else {
        return d;
    };
    SlashingParams {
        signed_blocks_window: json_i64(&v["params"]["signed_blocks_window"]),
        min_signed_per_window: json_f64(&v["params"]["min_signed_per_window"]),
    }
}

fn json_u64(v: &serde_json::Value) -> u64 {
    if let Some(n) = v.as_u64() {
        return n;
    }
    if let Some(n) = v.as_i64() {
        return n.max(0) as u64;
    }
    v.as_str().and_then(|s| s.parse().ok()).unwrap_or(0)
}

fn json_i64(v: &serde_json::Value) -> i64 {
    if let Some(n) = v.as_i64() {
        return n;
    }
    v.as_str().and_then(|s| s.parse().ok()).unwrap_or(0)
}

fn json_f64(v: &serde_json::Value) -> f64 {
    if let Some(n) = v.as_f64() {
        return n;
    }
    v.as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0)
}

/// Full bonded validator set with exact missed-block + tombstone attribution.
pub async fn fetch_all(
    client: &reqwest::Client,
    chain: &ChainConfig,
) -> anyhow::Result<Vec<ValidatorSnapshot>> {
    // 1. validators (paginated)
    let mut validators: Vec<serde_json::Value> = Vec::new();
    let mut next_key: Option<String> = None;
    loop {
        let mut url = rest(
            &chain.rest,
            "/cosmos/staking/v1beta1/validators?status=BOND_STATUS_BONDED&pagination.limit=200",
        );
        if let Some(k) = &next_key {
            url.push_str(&format!("&pagination.key={k}"));
        }
        let v = get_json(client, &url).await?;
        if let Some(arr) = v["validators"].as_array() {
            validators.extend(arr.clone());
        }
        next_key = v["pagination"]["next_key"].as_str().filter(|s| !s.is_empty()).map(|s| s.to_string());
        if next_key.is_none() {
            break;
        }
    }
    // 2. signing infos → (valcons addr → (missed, tombstoned))
    let mut signing: HashMap<String, (u64, bool)> = HashMap::new();
    let mut si_next: Option<String> = None;
    loop {
        let mut url = rest(&chain.rest, "/cosmos/slashing/v1beta1/signing_infos?pagination.limit=300");
        if let Some(k) = &si_next {
            url.push_str(&format!("&pagination.key={k}"));
        }
        let Ok(v) = get_json(client, &url).await else { break };
        if let Some(arr) = v["info"].as_array() {
            for info in arr {
                if let Some(addr) = info["address"].as_str() {
                    let missed = json_u64(&info["missed_blocks_counter"]);
                    let tomb = info["tombstoned"].as_bool().unwrap_or(false);
                    signing.insert(addr.to_string(), (missed, tomb));
                }
            }
        }
        si_next = v["pagination"]["next_key"].as_str().filter(|s| !s.is_empty()).map(|s| s.to_string());
        if si_next.is_none() {
            break;
        }
    }
    // 3. join
    let total_tokens: f64 = validators
        .iter()
        .filter_map(|v| v["tokens"].as_str()?.parse::<f64>().ok())
        .sum();
    let height = latest_height(client, &chain.rest).await.unwrap_or(0);
    let mut out = Vec::new();
    for v in &validators {
        let valoper = v["operator_address"].as_str().unwrap_or("?").to_string();
        let pubkey = v["consensus_pubkey"]["key"].as_str().unwrap_or("");
        let cons = if pubkey.is_empty() {
            String::new()
        } else {
            cons_address(&chain.prefix, pubkey).unwrap_or_default()
        };
        let joined = if cons.is_empty() {
            None
        } else {
            signing.get(&cons).copied()
        };
        let (missed, tomb, attributed) = match joined {
            Some((missed, tomb)) => (missed, tomb, true),
            None => (0, false, false),
        };
        let tokens: f64 = v["tokens"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
        out.push(ValidatorSnapshot {
            valoper,
            moniker: v["description"]["moniker"].as_str().unwrap_or("?").to_string(),
            jailed: v["jailed"].as_bool().unwrap_or(false),
            status: v["status"].as_str().unwrap_or("UNKNOWN").to_string(),
            tokens: v["tokens"].as_str().unwrap_or("0").to_string(),
            voting_pct: if total_tokens > 0.0 { 100.0 * tokens / total_tokens } else { 0.0 },
            missed_blocks: missed,
            tombstoned: tomb,
            height,
            attributed,
            cons,
        });
    }
    out.sort_by(|a, b| b.tokens.parse::<f64>().unwrap_or(0.0).partial_cmp(&a.tokens.parse::<f64>().unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal));
    Ok(out)
}

/// Single-validator snapshot (kept for the monitor's targeted checks).
pub async fn fetch_snapshot(
    client: &reqwest::Client,
    chain: &ChainConfig,
    valoper: &str,
) -> anyhow::Result<ValidatorSnapshot> {
    let all = fetch_all(client, chain).await?;
    all.into_iter()
        .find(|s| s.valoper == valoper)
        .ok_or_else(|| anyhow::anyhow!("validator not in bonded set: {valoper}"))
}

// ── offline tracking ────────────────────────────────────────────────────

/// Distance to the slashing line. Higher bands are worse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
enum JailBand {
    #[default]
    Quiet,
    Notice,
    Critical,
    Line,
}

fn jail_band(s: &ValidatorSnapshot, allowed: u64) -> JailBand {
    if s.jailed || s.tombstoned {
        return JailBand::Line;
    }
    if allowed == 0 {
        return JailBand::Quiet;
    }
    let pct = s.missed_blocks.saturating_mul(100) / allowed;
    if pct >= 95 || s.missed_blocks >= allowed {
        JailBand::Line
    } else if pct >= 80 {
        JailBand::Critical
    } else if pct >= 50 {
        JailBand::Notice
    } else {
        JailBand::Quiet
    }
}

/// Per-validator liveness memory across polls.
#[derive(Debug, Clone, Default)]
pub struct Tracked {
    pub snap: ValidatorSnapshot,
    seeded: bool,
    band: JailBand,
    last_warned: i64,
    /// Polls spent back under 50% after a higher band.
    quiet_polls: u32,
}

#[derive(Debug, Default)]
pub struct Monitor {
    pub tracked: HashMap<String, Tracked>,
    pub remind_secs: i64,
    attribution_open: bool,
    last_height: i64,
    seen_slashes: VecDeque<String>,
}

impl Monitor {
    pub fn new(remind_secs: i64) -> Self {
        Self {
            tracked: HashMap::new(),
            remind_secs,
            attribution_open: false,
            last_height: 0,
            seen_slashes: VecDeque::new(),
        }
    }

    /// Heights to scan for slash events. The first poll only records the
    /// tip. A gap larger than 20 blocks keeps the newest 20 so a restart
    /// does not walk the whole history.
    pub fn heights_since(&mut self, latest: i64) -> Vec<i64> {
        if latest <= 0 {
            return Vec::new();
        }
        if self.last_height <= 0 || latest <= self.last_height {
            self.last_height = latest;
            return Vec::new();
        }
        const CAP: i64 = 20;
        let start = if latest - self.last_height > CAP {
            latest - CAP + 1
        } else {
            self.last_height + 1
        };
        self.last_height = latest;
        (start..=latest).collect()
    }

    /// Diff fresh snapshots against memory → alerts. Pure logic, unit-tested.
    pub fn poll(
        &mut self,
        chain: &str,
        snaps: &[ValidatorSnapshot],
        now: i64,
        allowed_missed: u64,
    ) -> Vec<ValidatorAlert> {
        let remind_secs = self.remind_secs;
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let join_misses = snaps.iter().filter(|s| !s.attributed).count();
        if !snaps.is_empty() && join_misses * 2 > snaps.len() {
            if !self.attribution_open {
                out.push(ValidatorAlert {
                    chain: chain.to_string(),
                    valoper: String::new(),
                    kind: "ATTRIBUTION",
                    message: format!(
                        "signing attribution failed for {join_misses}/{} bonded validators",
                        snaps.len()
                    ),
                    critical: true,
                });
                self.attribution_open = true;
            }
        } else {
            self.attribution_open = false;
        }
        for s in snaps {
            seen.insert(s.valoper.clone());
            let t = self.tracked.entry(s.valoper.clone()).or_default();
            if !s.attributed {
                t.seeded = true;
                t.band = JailBand::Quiet;
                t.snap = s.clone();
                continue;
            }
            let next = jail_band(s, allowed_missed);
            if !t.seeded {
                t.seeded = true;
                t.band = next;
                t.snap = s.clone();
                continue;
            }
            let was_jailed = t.snap.jailed || t.snap.tombstoned;
            if was_jailed && !s.jailed && !s.tombstoned {
                out.push(ValidatorAlert {
                    chain: chain.to_string(),
                    valoper: s.valoper.clone(),
                    kind: "RECOVERED",
                    message: format!(
                        "✅ `{}` unjailed (missed {}/{allowed_missed})",
                        s.moniker, s.missed_blocks
                    ),
                    critical: false,
                });
                t.band = next;
                t.quiet_polls = 0;
                t.last_warned = now;
                t.snap = s.clone();
                continue;
            }
            let became_tomb = !t.snap.tombstoned && s.tombstoned;
            let became_jail = !t.snap.jailed && s.jailed;
            if became_tomb || became_jail {
                let (kind, message) = if became_tomb {
                    (
                        "TOMBSTONED",
                        format!(
                            "💀 `{}` tombstoned (missed {}/{allowed_missed})",
                            s.moniker, s.missed_blocks
                        ),
                    )
                } else {
                    (
                        "JAILED",
                        format!(
                            "🚨 `{}` jailed (missed {}/{allowed_missed})",
                            s.moniker, s.missed_blocks
                        ),
                    )
                };
                out.push(ValidatorAlert {
                    chain: chain.to_string(),
                    valoper: s.valoper.clone(),
                    kind,
                    message,
                    critical: true,
                });
                t.band = next;
                t.quiet_polls = 0;
                t.last_warned = now;
                t.snap = s.clone();
                continue;
            }
            if next > t.band {
                if !s.jailed && !s.tombstoned {
                    out.push(band_alert(chain, s, allowed_missed, next));
                }
                t.band = next;
                t.quiet_polls = 0;
                t.last_warned = now;
            } else if next == t.band
                && next > JailBand::Quiet
                && !s.jailed
                && !s.tombstoned
                && now - t.last_warned >= remind_secs
            {
                out.push(band_alert(chain, s, allowed_missed, next));
                t.last_warned = now;
                t.quiet_polls = 0;
            } else if next == JailBand::Quiet && t.band > JailBand::Quiet {
                t.quiet_polls += 1;
                if t.quiet_polls >= 2 {
                    out.push(ValidatorAlert {
                        chain: chain.to_string(),
                        valoper: s.valoper.clone(),
                        kind: "RECOVERED",
                        message: format!(
                            "✅ `{}` back under the jail line (missed {}/{allowed_missed})",
                            s.moniker, s.missed_blocks
                        ),
                        critical: false,
                    });
                    t.band = JailBand::Quiet;
                    t.quiet_polls = 0;
                }
            } else if next < t.band {
                t.band = next;
                t.quiet_polls = 0;
            }
            t.snap = s.clone();
        }
        self.tracked.retain(|k, _| seen.contains(k));
        out
    }
}

fn band_alert(chain: &str, s: &ValidatorSnapshot, allowed: u64, band: JailBand) -> ValidatorAlert {
    let left = allowed.saturating_sub(s.missed_blocks);
    let (kind, critical, message) = match band {
        JailBand::Line => (
            "JAIL_LINE",
            true,
            format!(
                "🚨 `{}` at the jail line — missed {}/{allowed}",
                s.moniker, s.missed_blocks
            ),
        ),
        JailBand::Critical => (
            "MISSED_CRITICAL",
            true,
            format!(
                "🚨 `{}` missed {}/{allowed} ({left} blocks to jail)",
                s.moniker, s.missed_blocks
            ),
        ),
        _ => (
            "MISSED",
            false,
            format!(
                "⚠️ `{}` missed {}/{allowed} ({left} blocks to jail)",
                s.moniker, s.missed_blocks
            ),
        ),
    };
    ValidatorAlert {
        chain: chain.to_string(),
        valoper: s.valoper.clone(),
        kind,
        message,
        critical,
    }
}

/// Bonded validator closest to the slashing line, even when still under 50%.
pub fn closest_jail_line(snaps: &[ValidatorSnapshot], allowed: u64) -> Option<String> {
    let best = snaps
        .iter()
        .filter(|s| s.attributed)
        .max_by_key(|s| {
            if allowed == 0 {
                s.missed_blocks
            } else {
                s.missed_blocks.saturating_mul(1_000_000) / allowed.max(1)
            }
        })?;
    let left = allowed.saturating_sub(best.missed_blocks);
    Some(format!(
        "closest to jail: `{}` {}/{} ({} blocks left)",
        best.moniker, best.missed_blocks, allowed, left
    ))
}

pub fn write_closest_line(path: &std::path::Path, snaps: &[ValidatorSnapshot], allowed: u64) {
    let Some(line) = closest_jail_line(snaps, allowed) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, line);
}

pub fn format_duration(secs: i64) -> String {
    let secs = secs.max(0);
    if secs < 60 {
        return format!("{secs}s");
    }
    let (m, s) = (secs / 60, secs % 60);
    if m < 60 {
        return format!("{m}m {s}s");
    }
    format!("{}h {}m", m / 60, m % 60)
}

/// Backwards-compatible diff used by targeted checks.
pub fn diff(
    chain: &str,
    prev: Option<&ValidatorSnapshot>,
    cur: &ValidatorSnapshot,
    threshold: u64,
) -> Vec<ValidatorAlert> {
    let mut out = Vec::new();
    if let Some(p) = prev {
        if !p.jailed && cur.jailed {
            out.push(ValidatorAlert {
                chain: chain.to_string(),
                valoper: cur.valoper.clone(),
                kind: "JAILED",
                message: format!("🚨 `{}` ({}) **JAILED** on {}", cur.moniker, cur.valoper, chain),
                critical: true,
            });
        } else if p.jailed && !cur.jailed {
            out.push(ValidatorAlert {
                chain: chain.to_string(),
                valoper: cur.valoper.clone(),
                kind: "UNJAILED",
                message: format!("✅ `{}` ({}) recovered (unjailed) on {}", cur.moniker, cur.valoper, chain),
                critical: false,
            });
        }
        if p.status != cur.status {
            out.push(ValidatorAlert {
                chain: chain.to_string(),
                valoper: cur.valoper.clone(),
                kind: "STATUS",
                message: format!("🔄 `{}` status: `{}` → `{}` on {}", cur.moniker, p.status, cur.status, chain),
                critical: cur.status != "BOND_STATUS_BONDED",
            });
        }
        if cur.missed_blocks >= threshold && p.missed_blocks < threshold {
            out.push(ValidatorAlert {
                chain: chain.to_string(),
                valoper: cur.valoper.clone(),
                kind: "MISSED",
                message: format!("⚠️ `{}` missed blocks {} (threshold {}) on {}", cur.moniker, cur.missed_blocks, threshold, chain),
                critical: true,
            });
        }
    }
    out
}

/// One-line row for `/validator list`.
pub fn format_row(s: &ValidatorSnapshot) -> String {
    let flag = if s.tombstoned {
        "💀"
    } else if s.jailed {
        "🔴"
    } else if s.missed_blocks > 0 {
        "🟡"
    } else {
        "🟢"
    };
    format!(
        "{} `{}` {:.2}% missed:{} {}",
        flag,
        s.moniker,
        s.voting_pct,
        s.missed_blocks,
        crate::terp::chain_watch::format_uthiol(&s.tokens)
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashNotice {
    pub height: i64,
    pub address: String,
    pub reason: String,
    pub burned: String,
}

pub async fn fetch_block_results(
    client: &reqwest::Client,
    chain_rest: &str,
    height: i64,
) -> anyhow::Result<serde_json::Value> {
    let url = rest(
        chain_rest,
        &format!("/cosmos/base/tendermint/v1beta1/block_results/{height}"),
    );
    get_json(client, &url).await
}

pub fn parse_slash_events(height: i64, body: &serde_json::Value) -> Vec<SlashNotice> {
    let mut out = Vec::new();
    collect_slash(height, body.get("finalize_block_events"), &mut out);
    collect_slash(height, body.get("begin_block_events"), &mut out);
    collect_slash(height, body.get("end_block_events"), &mut out);
    if let Some(txs) = body.get("txs_results").and_then(|v| v.as_array()) {
        for tx in txs {
            collect_slash(height, tx.get("events"), &mut out);
        }
    }
    out
}

fn collect_slash(height: i64, events: Option<&serde_json::Value>, out: &mut Vec<SlashNotice>) {
    let Some(events) = events.and_then(|v| v.as_array()) else {
        return;
    };
    for event in events {
        if event.get("type").and_then(|v| v.as_str()) != Some("slash") {
            continue;
        }
        let mut address = String::new();
        let mut reason = String::new();
        let mut burned = String::new();
        let Some(attrs) = event.get("attributes").and_then(|v| v.as_array()) else {
            continue;
        };
        for attr in attrs {
            let key = attr_text(attr.get("key").unwrap_or(&serde_json::Value::Null));
            let value = attr_text(attr.get("value").unwrap_or(&serde_json::Value::Null));
            match key.as_str() {
                "address" => address = value,
                "reason" => reason = value,
                "burned_coins" => burned = value,
                _ => {}
            }
        }
        if address.is_empty() {
            continue;
        }
        out.push(SlashNotice {
            height,
            address,
            reason,
            burned,
        });
    }
}

fn attr_text(v: &serde_json::Value) -> String {
    let raw = v.as_str().unwrap_or("").trim();
    if raw.is_empty() {
        return String::new();
    }
    decode_attr(raw).unwrap_or_else(|| raw.to_string())
}

fn decode_attr(raw: &str) -> Option<String> {
    if raw.len() < 4 || raw.len() % 4 != 0 {
        return None;
    }
    if !raw
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
    {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(raw).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    if text.is_empty() || !text.chars().all(|c| c.is_ascii() && !c.is_control()) {
        return None;
    }
    if text.len() >= raw.len() {
        return None;
    }
    Some(text)
}

impl Monitor {
    pub fn note_slashes(
        &mut self,
        chain: &str,
        snaps: &[ValidatorSnapshot],
        notices: &[SlashNotice],
    ) -> Vec<ValidatorAlert> {
        let mut out = Vec::new();
        for notice in notices {
            let key = format!("{}:{}:{}", notice.height, notice.address, notice.reason);
            if self.seen_slashes.iter().any(|seen| seen == &key) {
                continue;
            }
            self.seen_slashes.push_back(key);
            if self.seen_slashes.len() > 300 {
                self.seen_slashes.pop_front();
            }
            let named = snaps.iter().find(|s| s.cons == notice.address);
            let moniker = named.map(|s| s.moniker.as_str()).unwrap_or(notice.address.as_str());
            let reason = if notice.reason.is_empty() {
                "slash".to_string()
            } else {
                notice.reason.clone()
            };
            let burned = if notice.burned.is_empty() {
                String::new()
            } else {
                format!(" burned {}", notice.burned)
            };
            out.push(ValidatorAlert {
                chain: chain.to_string(),
                valoper: named.map(|s| s.valoper.clone()).unwrap_or_default(),
                kind: "SLASHED",
                message: format!(
                    "🚨 `{moniker}` slashed ({reason}) at height {}{burned}",
                    notice.height
                ),
                critical: true,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(moniker: &str, missed: u64, jailed: bool) -> ValidatorSnapshot {
        ValidatorSnapshot {
            valoper: format!("valoper-{moniker}"),
            moniker: moniker.into(),
            jailed,
            missed_blocks: missed,
            attributed: true,
            ..Default::default()
        }
    }

    #[test]
    fn counter_parses_string_and_number() {
        assert_eq!(json_u64(&serde_json::json!("120")), 120);
        assert_eq!(json_u64(&serde_json::json!(120)), 120);
    }

    #[test]
    fn bands_announce_upward_once_and_flat_is_quiet() {
        let mut m = Monitor::new(10_000);
        assert!(m.poll("terp", &[snap("a", 10, false)], 1, 100).is_empty());
        assert!(m.poll("terp", &[snap("a", 10, false)], 2, 100).is_empty());
        let notice = m.poll("terp", &[snap("a", 60, false)], 3, 100);
        assert_eq!(notice.len(), 1);
        assert_eq!(notice[0].kind, "MISSED");
        assert!(notice[0].message.contains("60/100"));
        assert!(m.poll("terp", &[snap("a", 60, false)], 4, 100).is_empty());
        let critical = m.poll("terp", &[snap("a", 85, false)], 5, 100);
        assert_eq!(critical[0].kind, "MISSED_CRITICAL");
        let line = m.poll("terp", &[snap("a", 96, false)], 6, 100);
        assert_eq!(line[0].kind, "JAIL_LINE");
        assert!(line[0].message.contains("jail line"));
    }

    #[test]
    fn flat_critical_is_not_recovery() {
        let mut m = Monitor::new(10_000);
        m.poll("terp", &[snap("a", 10, false)], 1, 100);
        m.poll("terp", &[snap("a", 85, false)], 2, 100);
        let again = m.poll("terp", &[snap("a", 85, false)], 3, 100);
        assert!(again.iter().all(|a| a.kind != "RECOVERED"));
    }

    #[test]
    fn two_quiet_polls_recover() {
        let mut m = Monitor::new(10_000);
        m.poll("terp", &[snap("a", 10, false)], 1, 100);
        m.poll("terp", &[snap("a", 60, false)], 2, 100);
        assert!(m.poll("terp", &[snap("a", 10, false)], 3, 100).is_empty());
        let recovered = m.poll("terp", &[snap("a", 10, false)], 4, 100);
        assert_eq!(recovered[0].kind, "RECOVERED");
    }

    #[test]
    fn join_miss_majority_is_one_attribution_line() {
        let mut bad = snap("a", 0, false);
        bad.attributed = false;
        let mut also = snap("b", 0, false);
        also.attributed = false;
        let mut m = Monitor::new(10_000);
        let alerts = m.poll("terp", &[bad.clone(), also.clone(), snap("c", 90, false)], 1, 100);
        assert!(alerts.iter().any(|a| a.kind == "ATTRIBUTION"));
        assert!(alerts.iter().all(|a| !a.message.contains("missed 0")));
        assert!(m.poll("terp", &[bad, also, snap("c", 90, false)], 2, 100).is_empty());
    }

    #[test]
    fn closest_line_names_the_highest_ratio_under_half() {
        let line = closest_jail_line(&[snap("low", 10, false), snap("high", 40, false)], 100).unwrap();
        assert!(line.contains("high"));
        assert!(line.contains("40/100"));
        assert!(line.contains("60 blocks left"));
    }

    #[test]
    fn cons_address_derives() {
        let a = cons_address("terp", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap();
        assert!(a.starts_with("terpvalcons1"));
        assert_eq!(a, "terpvalcons1ve584t0cv27hwmy0cx9ca8uwyqyfw9y9u7c35t");
    }

    #[test]
    fn jail_posts_when_the_band_does_not_move() {
        let mut m = Monitor::new(10_000);
        assert!(m.poll("terp", &[snap("a", 96, false)], 1, 100).is_empty());
        let jailed = m.poll("terp", &[snap("a", 96, true)], 2, 100);
        assert!(jailed.iter().any(|a| a.kind == "JAILED"));
        assert!(jailed.iter().all(|a| a.kind != "JAIL_LINE"));
        let mut tomb = snap("a", 96, true);
        tomb.tombstoned = true;
        let tombstoned = m.poll("terp", &[tomb], 3, 100);
        assert!(tombstoned.iter().any(|a| a.kind == "TOMBSTONED"));
    }

    #[test]
    fn unjail_posts_on_the_poll_that_sees_it() {
        let mut m = Monitor::new(10_000);
        m.poll("terp", &[snap("a", 10, false)], 1, 100);
        m.poll("terp", &[snap("a", 96, true)], 2, 100);
        let back = m.poll("terp", &[snap("a", 10, false)], 3, 100);
        assert!(back.iter().any(|a| a.message.contains("unjailed")));
    }

    #[test]
    fn slash_event_names_the_validator_once() {
        let body = serde_json::json!({
            "finalize_block_events": [{
                "type": "slash",
                "attributes": [
                    {"key": "YWRkcmVzcw==", "value": "dGVycHZhbGNvbnMxYWJj"},
                    {"key": "cmVhc29u", "value": "bWlzc2luZ19zaWduYXR1cmU="},
                    {"key": "burned_coins", "value": "1000uthiol"}
                ]
            }]
        });
        let notes = parse_slash_events(10, &body);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].address, "terpvalcons1abc");
        assert_eq!(notes[0].reason, "missing_signature");
        let mut who = snap("alpha", 96, true);
        who.cons = "terpvalcons1abc".into();
        let mut m = Monitor::new(10_000);
        let alerts = m.note_slashes("terp", &[who.clone()], &notes);
        assert_eq!(alerts[0].kind, "SLASHED");
        assert!(alerts[0].message.contains("alpha"));
        assert!(alerts[0].message.contains("missing_signature"));
        assert!(alerts[0].message.contains("1000uthiol"));
        assert!(m.note_slashes("terp", &[who], &notes).is_empty());
    }

    #[test]
    fn slash_scan_starts_at_the_tip() {
        let mut m = Monitor::new(10_000);
        assert!(m.heights_since(1000).is_empty());
        assert_eq!(m.heights_since(1002), vec![1001, 1002]);
        let gap = m.heights_since(2000);
        assert_eq!(gap.len(), 20);
        assert_eq!(*gap.last().unwrap(), 2000);
        assert!(m.heights_since(2000).is_empty());
    }
}
