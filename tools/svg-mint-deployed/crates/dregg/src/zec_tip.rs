//! House Zakura tip vs a public Zcash JSON-RPC tip.
//!
//! Payment watching stays on the private node (`ZCASH_DEVTOOL_SERVER` /
//! compact streamer). This probe is **getblockcount only**: if the house
//! node has drifted from the public tip (or is unreachable), checkout
//! closes so the FE can hide Pay. No refunds — the user never paid.
//!
//! `ZCASH_PUBLIC_RPC` unset → skip (native-only / lab without a sentry).
//! Public set → fail closed. Default max drift is 2 blocks.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::httpx;

/// Reason string also pushed onto `CheckoutStatus.reasons` when the circuit opens.
pub const REASON_DRIFT: &str = "zec_tip_drift";

const DEFAULT_MAX_DRIFT: u64 = 2;
const DEFAULT_CACHE_SECS: u64 = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZecTip {
    pub configured: bool,
    pub ok: bool,
    pub private_height: Option<u64>,
    pub public_height: Option<u64>,
    pub drift: Option<u64>,
    pub max_drift: u64,
    /// `drift` | `private_unreachable` | `stale_private` | `private_unconfigured`
    pub reason: Option<String>,
    /// `ok` | `degraded_public_unavailable` | `stale_private` | `drift` | `private_unreachable`
    pub status: String,
    pub public_source: Option<String>,
    pub last_public_height: Option<u64>,
    pub last_public_age_secs: Option<u64>,
    pub private_advanced_secs_ago: Option<u64>,
}

impl ZecTip {
    fn base(max_drift: u64) -> Self {
        Self {
            configured: true,
            ok: false,
            private_height: None,
            public_height: None,
            drift: None,
            max_drift,
            reason: None,
            status: "ok".into(),
            public_source: None,
            last_public_height: None,
            last_public_age_secs: None,
            private_advanced_secs_ago: None,
        }
    }

    pub fn skipped(max_drift: u64) -> Self {
        let mut t = Self::base(max_drift);
        t.configured = false;
        t.ok = true;
        t
    }

    /// Drift only fails when the house node is BEHIND the public tip by more than
    /// `max_drift`; a public source that lags the house is not a house fault.
    pub fn from_heights(private: u64, public: u64, max_drift: u64) -> Self {
        let mut t = Self::base(max_drift);
        t.private_height = Some(private);
        t.public_height = Some(public);
        t.drift = Some(private.abs_diff(public));
        if public > private && public - private > max_drift {
            t.reason = Some("drift".into());
            t.status = "drift".into();
        } else {
            t.ok = true;
        }
        t
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "ok": self.ok,
            "status": self.status,
            "configured": self.configured,
            "private_height": self.private_height,
            "public_height": self.public_height,
            "public_source": self.public_source,
            "last_public_height": self.last_public_height,
            "last_public_age_secs": self.last_public_age_secs,
            "private_advanced_secs_ago": self.private_advanced_secs_ago,
            "drift": self.drift,
            "max_drift": self.max_drift,
            "reason": self.reason,
        })
    }
}

pub fn max_drift() -> u64 {
    std::env::var("ZCASH_TIP_MAX_DRIFT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MAX_DRIFT)
}

pub fn public_rpc() -> Option<String> {
    std::env::var("ZCASH_PUBLIC_RPC")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Keyless secondary public height (JSON GET). Default Blockchair stats.
/// `ZCASH_PUBLIC_HEIGHT_URL=off` disables it.
fn public_height_url() -> Option<String> {
    let v = std::env::var("ZCASH_PUBLIC_HEIGHT_URL")
        .unwrap_or_else(|_| "https://api.blockchair.com/zcash/stats".into());
    let v = v.trim().to_string();
    if v.is_empty() || v == "off" { None } else { Some(v) }
}

fn secondary_height(url: &str) -> Result<u64, String> {
    let v = httpx::get_json(url)?;
    for p in ["/data/best_block_height", "/result", "/height", "/blocks"] {
        if let Some(h) = v.pointer(p).and_then(|x| x.as_u64()) {
            return Ok(h);
        }
    }
    Err("no height in response".into())
}

static SEC_CACHE: Mutex<Option<(u64, Instant)>> = Mutex::new(None);

/// Secondary source is rate limited: read at most once a minute.
fn secondary_cached(url: &str) -> Result<u64, String> {
    if let Ok(g) = SEC_CACHE.lock() {
        if let Some((h, at)) = g.as_ref() {
            if at.elapsed() < Duration::from_secs(60) {
                return Ok(*h);
            }
        }
    }
    let h = secondary_height(url)?;
    if let Ok(mut g) = SEC_CACHE.lock() {
        *g = Some((h, Instant::now()));
    }
    Ok(h)
}

pub fn private_rpc() -> Option<String> {
    std::env::var("ZCASH_PRIVATE_RPC")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var("ZAKURA_RPC")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
}

fn cache_ttl() -> Duration {
    let secs = std::env::var("ZCASH_TIP_CACHE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_CACHE_SECS);
    Duration::from_secs(secs)
}

fn private_stale_secs() -> u64 {
    std::env::var("ZCASH_PRIVATE_STALE_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(900)
}

struct Cache {
    at: Instant,
    tip: ZecTip,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);
/// (height, when it last increased)
static PRIV_SEEN: Mutex<Option<(u64, Instant)>> = Mutex::new(None);
/// (height, when read, source)
static PUB_LAST: Mutex<Option<(u64, Instant, String)>> = Mutex::new(None);

/// Serializes env mutations in unit tests (same idea as `minter_env_lock`).
pub fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

pub fn clear_cache() {
    if let Ok(mut g) = CACHE.lock() {
        *g = None;
    }
}

/// Live probe (cached a few seconds so `/health` polls do not hammer RPC).
pub fn probe() -> ZecTip {
    let ttl = cache_ttl();
    if !ttl.is_zero() {
        if let Ok(g) = CACHE.lock() {
            if let Some(c) = g.as_ref() {
                if c.at.elapsed() < ttl {
                    return c.tip.clone();
                }
            }
        }
    }
    let tip = probe_uncached();
    if let Ok(mut g) = CACHE.lock() {
        *g = Some(Cache {
            at: Instant::now(),
            tip: tip.clone(),
        });
    }
    tip
}

/// ZEC rail health. The private (house) node is the authority: the rail is OK when
/// its height is known and advanced within `ZCASH_PRIVATE_STALE_SECS`, and — only if a
/// public height is available — the house is not behind it by more than `max_drift`.
/// No public height (all sources failed) is a warning, `degraded_public_unavailable`.
pub fn probe_uncached() -> ZecTip {
    let max = max_drift();
    let public = public_rpc();
    let secondary = public_height_url();
    if public.is_none() && secondary.is_none() {
        return ZecTip::skipped(max);
    }
    let Some(private) = private_rpc() else {
        let mut t = ZecTip::base(max);
        t.reason = Some("private_unconfigured".into());
        t.status = "private_unreachable".into();
        return t;
    };
    let priv_h = getblockcount(&private, "ZCASH_PRIVATE_RPC_COOKIE");
    let mut pub_h: Option<(u64, String)> = None;
    if let Some(p) = public.as_deref() {
        if let Ok(h) = getblockcount(p, "ZCASH_PUBLIC_RPC_COOKIE") {
            pub_h = Some((h, "public_rpc".into()));
        }
    }
    if let Some(u) = secondary.as_deref() {
        if let Ok(h) = secondary_cached(u) {
            if pub_h.as_ref().map_or(true, |(x, _)| h > *x) {
                pub_h = Some((h, "public_height_url".into()));
            }
        }
    }
    let now = Instant::now();
    if let Some((h, src)) = pub_h.as_ref() {
        if let Ok(mut g) = PUB_LAST.lock() {
            *g = Some((*h, now, src.clone()));
        }
    }
    let last_pub = PUB_LAST.lock().ok().and_then(|g| g.clone());
    let Ok(p) = priv_h else {
        let mut t = ZecTip::base(max);
        t.public_height = pub_h.as_ref().map(|x| x.0);
        t.reason = Some("private_unreachable".into());
        t.status = "private_unreachable".into();
        return t;
    };
    let advanced_ago = {
        let mut g = PRIV_SEEN.lock().unwrap_or_else(|e| e.into_inner());
        match g.as_ref() {
            Some((h, _)) if p > *h => *g = Some((p, now)),
            None => *g = Some((p, now)),
            _ => {}
        }
        g.as_ref().map(|(_, at)| now.duration_since(*at).as_secs()).unwrap_or(0)
    };
    let mut t = match pub_h.as_ref() {
        Some((u, src)) => {
            let mut t = ZecTip::from_heights(p, *u, max);
            t.public_source = Some(src.clone());
            t
        }
        None => {
            let mut t = ZecTip::base(max);
            t.private_height = Some(p);
            t.ok = true;
            t.status = "degraded_public_unavailable".into();
            // Last-good public height still catches a house node that fell far behind.
            if let Some((lh, at, _)) = last_pub.as_ref() {
                let age = now.duration_since(*at).as_secs();
                let expected = lh + age / 75;
                if expected > p + max + 2 {
                    t.ok = false;
                    t.status = "drift".into();
                    t.reason = Some("drift".into());
                    t.drift = Some(expected - p);
                }
            }
            t
        }
    };
    if let Some((lh, at, _)) = last_pub.as_ref() {
        t.last_public_height = Some(*lh);
        t.last_public_age_secs = Some(now.duration_since(*at).as_secs());
    }
    t.private_advanced_secs_ago = Some(advanced_ago);
    if advanced_ago > private_stale_secs() {
        t.ok = false;
        t.status = "stale_private".into();
        t.reason = Some("stale_private".into());
    }
    t
}

fn getblockcount(url: &str, cookie_env: &str) -> Result<u64, String> {
    let body = serde_json::json!({
        "jsonrpc": "1.0",
        "id": "dregg-tip",
        "method": "getblockcount",
        "params": []
    });
    let auth = rpc_authorization(cookie_env);
    let v = if url.starts_with("https://") {
        post_https_json(url, &body, auth.as_deref())?
    } else {
        httpx::post_json_timeout_auth(
            url,
            &body,
            Duration::from_secs(2),
            Duration::from_secs(2),
            auth.as_deref(),
        )?
    };
    parse_height(&v)
}

/// Cookie file (`user:password`) for house Zakura. Public HTTPS tips use curl.
fn rpc_authorization(cookie_env: &str) -> Option<String> {
    let cookie_path = std::env::var(cookie_env)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    let raw = std::fs::read_to_string(&cookie_path).ok()?;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let pair = if raw.contains(':') {
        raw.to_string()
    } else {
        format!("__cookie__:{raw}")
    };
    Some(format!("Basic {}", basic_b64(pair.as_bytes())))
}

fn basic_b64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < data.len() {
        let b0 = data[i];
        let b1 = data.get(i + 1).copied();
        let b2 = data.get(i + 2).copied();
        out.push(T[(b0 >> 2) as usize] as char);
        out.push(T[(((b0 & 0x03) << 4) | (b1.unwrap_or(0) >> 4)) as usize] as char);
        if b1.is_some() {
            out.push(
                T[(((b1.unwrap_or(0) & 0x0f) << 2) | (b2.unwrap_or(0) >> 6)) as usize] as char,
            );
        } else {
            out.push('=');
        }
        if b2.is_some() {
            out.push(T[(b2.unwrap_or(0) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}

fn post_https_json(
    url: &str,
    body: &serde_json::Value,
    authorization: Option<&str>,
) -> Result<serde_json::Value, String> {
    let payload = body.to_string();
    let mut cmd = std::process::Command::new("curl");
    cmd.args([
        "-sS",
        "--max-time",
        "5",
        "-X",
        "POST",
        url,
        "-H",
        "Content-Type: application/json",
        "-d",
        &payload,
    ]);
    if let Some(a) = authorization.filter(|s| !s.is_empty()) {
        cmd.args(["-H", &format!("Authorization: {a}")]);
    }
    let out = cmd.output().map_err(|e| format!("curl https: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "curl https {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("rpc json: {e}"))
}

fn parse_height(v: &serde_json::Value) -> Result<u64, String> {
    if let Some(err) = v.get("error") {
        if !err.is_null() {
            return Err(format!("rpc error: {err}"));
        }
    }
    let r = v
        .get("result")
        .ok_or_else(|| "getblockcount: no result".to_string())?;
    if let Some(n) = r.as_u64() {
        return Ok(n);
    }
    if let Some(n) = r.as_i64() {
        if n < 0 {
            return Err("getblockcount: negative".into());
        }
        return Ok(n as u64);
    }
    if let Some(s) = r.as_str() {
        return s
            .parse()
            .map_err(|_| format!("getblockcount: not a height ({s})"));
    }
    Err("getblockcount: not a height".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn serve_height(height: u64) -> (u16, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let h = thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf);
            let payload =
                format!("{{\"jsonrpc\":\"1.0\",\"id\":\"dregg-tip\",\"result\":{height}}}");
            let resp = format!(
                "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
            let _ = s.write_all(resp.as_bytes());
        });
        (port, h)
    }

    #[test]
    fn heights_within_two_ok() {
        let t = ZecTip::from_heights(100, 102, 2);
        assert!(t.ok);
        assert_eq!(t.drift, Some(2));
        let t = ZecTip::from_heights(102, 100, 2);
        assert!(t.ok);
    }

    #[test]
    fn heights_three_is_drift() {
        let t = ZecTip::from_heights(100, 103, 2);
        assert!(!t.ok);
        assert_eq!(t.reason.as_deref(), Some("drift"));
        assert_eq!(t.drift, Some(3));
    }

    #[test]
    fn skip_when_public_unset() {
        let _g = env_lock();
        clear_cache();
        unsafe { std::env::remove_var("ZCASH_PUBLIC_RPC") };
        unsafe { std::env::remove_var("ZCASH_PRIVATE_RPC") };
        unsafe { std::env::remove_var("ZAKURA_RPC") };
        let t = probe_uncached();
        assert!(!t.configured);
        assert!(t.ok);
    }

    #[test]
    fn fail_closed_public_without_private() {
        let _g = env_lock();
        clear_cache();
        unsafe { std::env::set_var("ZCASH_PUBLIC_RPC", "http://127.0.0.1:1") };
        unsafe { std::env::remove_var("ZCASH_PRIVATE_RPC") };
        unsafe { std::env::remove_var("ZAKURA_RPC") };
        let t = probe_uncached();
        unsafe { std::env::remove_var("ZCASH_PUBLIC_RPC") };
        assert!(t.configured);
        assert!(!t.ok);
        assert_eq!(t.reason.as_deref(), Some("private_unconfigured"));
    }

    #[test]
    fn live_mock_in_sync_and_private_down() {
        let _g = env_lock();
        clear_cache();
        let (pub_port, pub_h) = serve_height(40);
        let (priv_port, priv_h) = serve_height(40);
        unsafe { std::env::set_var("ZCASH_PUBLIC_RPC", format!("http://127.0.0.1:{pub_port}")) };
        unsafe { std::env::set_var("ZCASH_PRIVATE_RPC", format!("http://127.0.0.1:{priv_port}")) };
        let ok = probe_uncached();
        let _ = pub_h.join();
        let _ = priv_h.join();
        assert!(ok.ok, "{ok:?}");
        assert_eq!(ok.private_height, Some(40));
        assert_eq!(ok.public_height, Some(40));

        clear_cache();
        let (pub_port2, pub_h2) = serve_height(41);
        unsafe { std::env::set_var("ZCASH_PUBLIC_RPC", format!("http://127.0.0.1:{pub_port2}")) };
        unsafe { std::env::set_var("ZCASH_PRIVATE_RPC", "http://127.0.0.1:1") };
        let down = probe_uncached();
        let _ = pub_h2.join();
        unsafe { std::env::remove_var("ZCASH_PUBLIC_RPC") };
        unsafe { std::env::remove_var("ZCASH_PRIVATE_RPC") };
        assert!(!down.ok);
        assert_eq!(down.reason.as_deref(), Some("private_unreachable"));
        assert_eq!(down.public_height, Some(41));
    }

    #[test]
    fn parse_numeric_and_string_result() {
        assert_eq!(
            parse_height(&serde_json::json!({"result": 12})).unwrap(),
            12
        );
        assert_eq!(
            parse_height(&serde_json::json!({"result": "12"})).unwrap(),
            12
        );
        assert!(parse_height(&serde_json::json!({"error": "nope"})).is_err());
    }

    #[test]
    fn cookie_basic_auth_reaches_height() {
        let _g = env_lock();
        clear_cache();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let h = thread::spawn(move || {
            for _ in 0..2 {
                let (mut s, _) = listener.accept().unwrap();
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let ok = req.contains("Authorization: Basic ");
                let payload = if ok {
                    "{\"jsonrpc\":\"1.0\",\"id\":\"dregg-tip\",\"result\":77}"
                } else {
                    "{\"jsonrpc\":\"1.0\",\"id\":\"dregg-tip\",\"error\":\"noauth\"}"
                };
                let resp = format!(
                    "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });
        let dir = std::env::temp_dir().join("dregg-zec-tip-cookie-test");
        let _ = std::fs::create_dir_all(&dir);
        let cookie = dir.join(".cookie");
        std::fs::write(&cookie, "rpcuser:rpcpass\n").unwrap();
        unsafe { std::env::set_var("ZCASH_PUBLIC_RPC", format!("http://127.0.0.1:{port}")) };
        unsafe { std::env::set_var("ZCASH_PRIVATE_RPC", format!("http://127.0.0.1:{port}")) };
        unsafe { std::env::set_var(
            "ZCASH_PRIVATE_RPC_COOKIE",
            cookie.to_string_lossy().as_ref(),
        ) };
        unsafe { std::env::remove_var("ZCASH_PUBLIC_RPC_COOKIE") };
        // First call is public (no cookie) — spawn a second listener in the same
        // process by probing private only via getblockcount helper path:
        // public will fail auth; treat this test as private-with-cookie by
        // giving both the cookie env for public too.
        unsafe { std::env::set_var("ZCASH_PUBLIC_RPC_COOKIE", cookie.to_string_lossy().as_ref()) };
        let t = probe_uncached();
        let _ = h.join();
        unsafe { std::env::remove_var("ZCASH_PUBLIC_RPC") };
        unsafe { std::env::remove_var("ZCASH_PRIVATE_RPC") };
        unsafe { std::env::remove_var("ZCASH_PRIVATE_RPC_COOKIE") };
        unsafe { std::env::remove_var("ZCASH_PUBLIC_RPC_COOKIE") };
        assert!(t.ok, "{t:?}");
        assert_eq!(t.private_height, Some(77));
    }
}
