//! House wallet total via svg-mint-zrt `balance --json`.
//!
//! Compact sync stays on svg-mint-zrt against ZTREAMER_GRPC or ZCASH_DEVTOOL_SERVER
//! (loopback, never zec.rocks). Amounts are for the admin ops desk; do not put
//! totals on public `/health`.

use serde_json::{json, Value};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CACHE_TTL: Duration = Duration::from_secs(8);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZecWallet {
    pub ok: bool,
    pub total_zat: u64,
    pub sapling_spendable_zat: u64,
    pub orchard_spendable_zat: u64,
    pub ironwood_spendable_zat: u64,
    pub transparent_spendable_zat: u64,
    pub chain_tip_height: Option<u64>,
    pub queried_at: u64,
    pub compact_server: Option<String>,
    pub error: Option<String>,
    pub stale: bool,
}

impl ZecWallet {
    pub fn to_json(&self) -> Value {
        json!({
            "ok": self.ok,
            "source": if std::env::var("SVG_MINT_ZRT_BIN").ok().filter(|s| !s.is_empty()).is_some() {
                "svg-mint-zrt"
            } else {
                "zcash-devtool"
            },
            "total_zat": self.total_zat,
            "total_zec": format_zec(self.total_zat),
            "sapling_spendable_zat": self.sapling_spendable_zat,
            "orchard_spendable_zat": self.orchard_spendable_zat,
            "ironwood_spendable_zat": self.ironwood_spendable_zat,
            "transparent_spendable_zat": self.transparent_spendable_zat,
            "chain_tip_height": self.chain_tip_height,
            "queried_at": self.queried_at,
            "compact_server": self.compact_server,
            "error": self.error,
            "stale": self.stale,
        })
    }
}


/// Spendable zat the desk may show. Orchard, sapling, and transparent notes are not accepted.
pub fn ironwood_spendable_from_notes(notes: &[Value]) -> u64 {
    notes.iter().fold(0u64, |acc, n| {
        let mined = n.get("mined").and_then(|m| m.as_bool()).unwrap_or(false);
        let pool = n.get("pool").and_then(|p| p.as_str()).unwrap_or("ironwood");
        let spent = n.get("spent").and_then(|s| s.as_bool()).unwrap_or(false);
        if !mined || spent || !pool.eq_ignore_ascii_case("ironwood") {
            return acc;
        }
        let zat = n.get("zat").and_then(|z| z.as_u64()).unwrap_or(0);
        acc.saturating_add(zat)
    })
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn format_zec(zat: u64) -> String {
    let whole = zat / 100_000_000;
    let frac = zat % 100_000_000;
    format!("{whole}.{frac:08}")
}

fn zat_field(v: &Value, key: &str) -> u64 {
    match v.get(key) {
        Some(Value::Number(n)) => n.as_u64().or_else(|| n.as_i64().map(|i| i.max(0) as u64)).unwrap_or(0),
        Some(Value::String(s)) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

/// Parse `zcash-devtool wallet balance --json` stdout.
pub fn parse_balance_json(raw: &str) -> Result<ZecWallet, String> {
    let line = raw
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with('{') && l.contains("total"))
        .unwrap_or(raw.trim());
    let v: Value = serde_json::from_str(line).map_err(|e| format!("balance json: {e}"))?;
    if v.get("total").is_none() {
        return Err("balance json missing total".into());
    }
    let height = v.get("chain_tip_height").and_then(|h| {
        h.as_u64()
            .or_else(|| h.as_i64().map(|i| i.max(0) as u64))
            .or_else(|| h.as_str().and_then(|s| s.parse().ok()))
    });
    Ok(ZecWallet {
        ok: true,
        total_zat: zat_field(&v, "total"),
        sapling_spendable_zat: zat_field(&v, "sapling_spendable"),
        orchard_spendable_zat: zat_field(&v, "orchard_spendable"),
        ironwood_spendable_zat: zat_field(&v, "ironwood_spendable"),
        transparent_spendable_zat: zat_field(&v, "transparent_spendable"),
        chain_tip_height: height,
        queried_at: now_unix(),
        compact_server: compact_server_opt(),
        error: None,
        stale: false,
    })
}

fn failed(err: impl Into<String>) -> ZecWallet {
    ZecWallet {
        ok: false,
        total_zat: 0,
        sapling_spendable_zat: 0,
        orchard_spendable_zat: 0,
        ironwood_spendable_zat: 0,
        transparent_spendable_zat: 0,
        chain_tip_height: None,
        queried_at: now_unix(),
        compact_server: compact_server_opt(),
        error: Some(err.into()),
        stale: false,
    }
}

/// Compact streamer for the JSON blob. Balance itself is sqlite-only.
pub fn compact_server_opt() -> Option<String> {
    for name in ["ZTREAMER_GRPC", "ZCASH_DEVTOOL_SERVER"] {
        if let Ok(s) = std::env::var(name) {
            let s = s.trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
}

fn refuse_public_compact(server: &str) -> Result<(), String> {
    let lower = server.to_lowercase();
    if lower.contains("zec.rocks") || lower.contains("rotko.net") {
        return Err("compact server refuses public zidecar (zec.rocks / rotko)".into());
    }
    Ok(())
}

fn cache_slot() -> &'static Mutex<Option<(Instant, ZecWallet)>> {
    static C: OnceLock<Mutex<Option<(Instant, ZecWallet)>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

fn is_locked(err: &str) -> bool {
    let l = err.to_lowercase();
    l.contains("database is locked") || l.contains("error code 5")
}

/// Cached house wallet total. Never blocks on `sync`.
/// While zcash-devtool `sync` holds sqlite, return the last good total.
pub fn query() -> ZecWallet {
    let last_ok = cache_slot().lock().ok().and_then(|g| {
        g.as_ref().and_then(|(_, w)| if w.ok { Some(w.clone()) } else { None })
    });
    if let Ok(g) = cache_slot().lock() {
        if let Some((at, w)) = g.as_ref() {
            if at.elapsed() < CACHE_TTL && w.ok {
                return w.clone();
            }
        }
    }
    let mut w = failed("unset");
    for i in 0..4 {
        if i > 0 {
            std::thread::sleep(Duration::from_millis(150));
        }
        w = query_fresh();
        if w.ok {
            break;
        }
        if !w.error.as_deref().map(is_locked).unwrap_or(false) {
            break;
        }
    }
    if w.ok {
        if let Ok(mut g) = cache_slot().lock() {
            *g = Some((Instant::now(), w.clone()));
        }
        return w;
    }
    if w.error.as_deref().map(is_locked).unwrap_or(false) {
        if let Some(mut old) = last_ok {
            old.stale = true;
            old.error = Some("database_locked".into());
            old.queried_at = now_unix();
            return old;
        }
    }
    w
}

pub fn query_fresh() -> ZecWallet {
    if let Some(s) = compact_server_opt() {
        if let Err(e) = refuse_public_compact(&s) {
            return failed(e);
        }
    }
    let zrt = std::env::var("SVG_MINT_ZRT_BIN")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "svg-mint-zrt".to_string());
    let out = match Command::new(&zrt).args(["balance", "--json"]).output() {
        Ok(o) => o,
        Err(e) => return failed(format!("svg-mint-zrt: {e}")),
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        return match parse_balance_json(&stdout) {
            Ok(w) => w,
            Err(e) => failed(e),
        };
    }
    failed(format!("svg-mint-zrt balance: {} {}", stderr.trim(), stdout.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_live_shape() {
        let raw = r#"{"chain_tip_height":3492231,"ironwood_spendable":0,"orchard_spendable":12,"sapling_spendable":3,"total":15000000,"transparent_spendable":4}"#;
        let w = parse_balance_json(raw).unwrap();
        assert!(w.ok);
        assert_eq!(w.total_zat, 15_000_000);
        assert_eq!(w.orchard_spendable_zat, 12);
        assert_eq!(w.sapling_spendable_zat, 3);
        assert_eq!(w.transparent_spendable_zat, 4);
        assert_eq!(w.chain_tip_height, Some(3_492_231));
        assert_eq!(format_zec(15_000_000), "0.15000000");
        let j = w.to_json();
        assert_eq!(j["total_zat"], 15_000_000);
        assert_eq!(j["total_zec"], "0.15000000");
        assert_eq!(j["source"], "zcash-devtool");
        assert_eq!(j["stale"], false);
    }

    #[test]
    fn parse_string_zat() {
        let raw = r#"{"total":"100000000","chain_tip_height":"9"}"#;
        let w = parse_balance_json(raw).unwrap();
        assert_eq!(w.total_zat, 100_000_000);
        assert_eq!(w.chain_tip_height, Some(9));
        assert_eq!(format_zec(100_000_000), "1.00000000");
    }

    #[test]
    fn missing_total_is_err() {
        assert!(parse_balance_json("{}").is_err());
    }

    #[test]
    fn orchard_notes_are_not_spendable() {
        let notes = vec![
            json!({"pool":"ironwood","mined":true,"zat":150000}),
            json!({"pool":"orchard","mined":true,"zat":900000}),
            json!({"pool":"ironwood","mined":false,"zat":50000}),
            json!({"pool":"ironwood","mined":true,"spent":true,"zat":900000}),
        ];
        assert_eq!(ironwood_spendable_from_notes(&notes), 150000);
    }

}
