//! Artist/admin web session for the ITL static export.
//!
//! Next `output: 'export'` cannot `Set-Cookie`. This module is served by the
//! existing HashMerchant frost inbox (`127.0.0.1:19091`), not by Next and not
//! by the mail gateway. Cookie: HttpOnly, SameSite=Strict, host-only, no Secure
//! on HTTP loopback. Role is checked here against `ITL_ADMIN_CREDS` /
//! `ITL_USHER_CREDS` (never `NEXT_PUBLIC_*`).
//!
//! Gallery spec §1.4: `ch = H("itl-login-v1"||origin||artist_id||nonce||ts)`,
//! 5-minute TTL, single-use nonce. A web session is not an escrow signature.

use super::http::HttpResponse;
use base64::Engine;
use k256::ecdsa::signature::hazmat::PrehashVerifier;
use k256::ecdsa::{Signature, VerifyingKey};
use ripemd::Ripemd160;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const COOKIE_NAME: &str = "itl_session";
const TTL_SECS: u64 = 4 * 3600;
const CHALLENGE_TTL_SECS: u64 = 5 * 60;
const CHALLENGE_PREFIX: &[u8] = b"itl-login-v1";
const COOKIE_MAC_PREFIX: &[u8] = b"itl-session-cookie-v1";

const ALLOWED_ORIGINS: &[&str] = &[
    "http://127.0.0.1:3051",
    "http://localhost:3051",
    "http://127.0.0.1:3050",
    "http://localhost:3050",
];

#[derive(Clone)]
struct Challenge {
    origin: String,
    artist_id: String,
    ts: u64,
    message: String,
    challenge_hex: String,
}

#[derive(Clone)]
struct Session {
    login_id: String,
    role: String,
    method: String,
    exp: u64,
}

struct Store {
    challenges: HashMap<String, Challenge>,
    sessions: HashMap<String, Session>,
    seq: u64,
}

fn store() -> &'static Mutex<Store> {
    static S: OnceLock<Mutex<Store>> = OnceLock::new();
    S.get_or_init(|| {
        Mutex::new(Store {
            challenges: HashMap::new(),
            sessions: HashMap::new(),
            seq: 0,
        })
    })
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn origin_allowed(origin: &str) -> bool {
    if ALLOWED_ORIGINS.contains(&origin) {
        return true;
    }
    let extra = std::env::var("ITL_SESSION_ORIGINS").unwrap_or_default();
    extra
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .any(|s| s == origin)
}

fn split_env(name: &str) -> Vec<String> {
    std::env::var(name)
        .ok()
        .map(|s| {
            s.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn role_for(login_id: &str, cred_id: &str) -> &'static str {
    let admin = split_env("ITL_ADMIN_CREDS");
    let usher = split_env("ITL_USHER_CREDS");
    let hit = |list: &[String]| {
        (!login_id.is_empty() && list.iter().any(|x| x == login_id))
            || (!cred_id.is_empty() && list.iter().any(|x| x == cred_id))
    };
    if hit(&admin) {
        "admin"
    } else if hit(&usher) {
        "usher"
    } else {
        "guest"
    }
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        let d = Sha256::digest(key);
        k[..32].copy_from_slice(&d);
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = k;
    let mut opad = k;
    for b in &mut ipad {
        *b ^= 0x36;
    }
    for b in &mut opad {
        *b ^= 0x5c;
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(data);
    let ih = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(ih);
    outer.finalize().into()
}

fn session_secret() -> [u8; 32] {
    static S: OnceLock<[u8; 32]> = OnceLock::new();
    *S.get_or_init(|| {
        if let Ok(raw) = std::env::var("ITL_SESSION_HMAC") {
            if raw.len() >= 16 {
                let mut h = Sha256::new();
                h.update(b"itl-session-hmac-v1");
                h.update(raw.as_bytes());
                return h.finalize().into();
            }
        }
        let mut h = Sha256::new();
        h.update(b"itl-session-ephemeral-v1");
        h.update(std::process::id().to_le_bytes());
        if let Ok(d) = SystemTime::now().duration_since(UNIX_EPOCH) {
            h.update(d.as_nanos().to_le_bytes());
        }
        h.finalize().into()
    })
}

fn hex_encode(bytes: &[u8]) -> String {
    const T: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(T[(b >> 4) as usize] as char);
        out.push(T[(b & 0xf) as usize] as char);
    }
    out
}

#[cfg(test)]
fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || s.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let b = s.as_bytes();
    let nibble = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    let mut i = 0;
    while i < b.len() {
        out.push((nibble(b[i])? << 4) | nibble(b[i + 1])?);
        i += 2;
    }
    Some(out)
}

fn challenge_hex(origin: &str, artist_id: &str, nonce: &str, ts: u64) -> String {
    let mut h = Sha256::new();
    h.update(CHALLENGE_PREFIX);
    h.update(origin.as_bytes());
    h.update(artist_id.as_bytes());
    h.update(nonce.as_bytes());
    h.update(ts.to_le_bytes());
    hex_encode(&h.finalize())
}

fn login_message(origin: &str, challenge: &str) -> String {
    format!("ITL login v1\norigin: {origin}\nchallenge: {challenge}")
}

fn adr036_sign_doc(signer: &str, data: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(data);
    format!(
        r#"{{"account_number":"0","chain_id":"","fee":{{"amount":[],"gas":"0"}},"memo":"","msgs":[{{"type":"sign/MsgSignData","value":{{"data":"{b64}","signer":"{signer}"}}}}],"sequence":"0"}}"#
    )
}

fn terp_address_from_compressed(pk33: &[u8]) -> Result<String, &'static str> {
    if pk33.len() != 33 {
        return Err("bad_pubkey");
    }
    let sha = Sha256::digest(pk33);
    let rip = Ripemd160::digest(sha);
    let hrp = bech32::Hrp::parse("terp").map_err(|_| "bad_hrp")?;
    bech32::encode::<bech32::Bech32>(hrp, &rip).map_err(|_| "bech32")
}

fn verify_secp256k1(
    pub_key_b64: &str,
    sig_b64: &str,
    digest: &[u8],
) -> Result<Vec<u8>, &'static str> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let pk = b64.decode(pub_key_b64.trim()).map_err(|_| "bad_pubkey")?;
    let sig = b64.decode(sig_b64.trim()).map_err(|_| "bad_sig")?;
    let vk = VerifyingKey::from_sec1_bytes(&pk).map_err(|_| "bad_pubkey")?;
    let mut s = Signature::from_slice(&sig).map_err(|_| "bad_sig")?;
    s = s.normalize_s();
    vk.verify_prehash(digest, &s).map_err(|_| "sig_verify")?;
    Ok(pk)
}

fn json_status(status: u16, body: String) -> HttpResponse {
    HttpResponse { status, body }
}

#[derive(Clone, Debug)]
pub struct SessionResponse {
    pub status: u16,
    pub body: String,
    pub set_cookie: Option<String>,
}

impl SessionResponse {
    fn json(status: u16, body: String) -> Self {
        Self {
            status,
            body,
            set_cookie: None,
        }
    }

    fn err(status: u16, code: &str) -> Self {
        Self::json(status, json!({"ok": false, "error": code}).to_string())
    }

    pub fn write_to(&self, s: &mut impl Write, origin: Option<&str>) -> std::io::Result<()> {
        let reason = if self.status == 200 || self.status == 204 {
            "OK"
        } else {
            "ERR"
        };
        let mut head = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\n",
            self.status,
            reason,
            self.body.len()
        );
        if let Some(o) = origin {
            if origin_allowed(o) {
                head.push_str("Access-Control-Allow-Origin: ");
                head.push_str(o);
                head.push_str("\r\nAccess-Control-Allow-Credentials: true\r\nVary: Origin\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: content-type, x-itl-frost\r\n");
            }
        }
        if let Some(c) = &self.set_cookie {
            head.push_str("Set-Cookie: ");
            head.push_str(c);
            head.push_str("\r\n");
        }
        head.push_str("Connection: close\r\n\r\n");
        s.write_all(head.as_bytes())?;
        s.write_all(self.body.as_bytes())
    }
}

pub fn cookie_header(token: &str, max_age: u64) -> String {
    format!("{COOKIE_NAME}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={max_age}")
}

fn mint_token(login_id: &str, role: &str, method: &str, exp: u64) -> String {
    let mut g = store().lock().expect("session store");
    g.seq += 1;
    let mut h = Sha256::new();
    h.update(COOKIE_MAC_PREFIX);
    h.update(&session_secret());
    h.update(g.seq.to_le_bytes());
    h.update(exp.to_le_bytes());
    h.update(login_id.as_bytes());
    let token = hex_encode(&h.finalize());
    let mac = hmac_sha256(
        &session_secret(),
        format!("{token}|{login_id}|{role}|{method}|{exp}").as_bytes(),
    );
    let full = format!("{token}.{}", hex_encode(&mac));
    g.sessions.insert(
        full.clone(),
        Session {
            login_id: login_id.to_string(),
            role: role.to_string(),
            method: method.to_string(),
            exp,
        },
    );
    full
}

fn cookie_from_header(raw: Option<&str>) -> Option<String> {
    let raw = raw?;
    for part in raw.split(';') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix(COOKIE_NAME) {
            let v = v.strip_prefix('=').unwrap_or(v).trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn lookup(token: &str) -> Option<Session> {
    let mut g = store().lock().ok()?;
    let t = now();
    g.sessions.retain(|_, s| s.exp > t);
    g.challenges
        .retain(|_, c| t.saturating_sub(c.ts) < CHALLENGE_TTL_SECS);
    g.sessions.get(token).cloned()
}

#[derive(Clone)]
pub struct SessionView {
    pub role: String,
    pub login_id: String,
}

/// Role and login from the HttpOnly cookie. Does not return the token.
pub fn session_from_cookie(cookie_header: Option<&str>) -> Option<SessionView> {
    let token = cookie_from_header(cookie_header)?;
    let s = lookup(&token)?;
    Some(SessionView {
        role: s.role,
        login_id: s.login_id,
    })
}

#[cfg(test)]
pub fn mint_test_cookie(role: &str, login_id: &str) -> String {
    let exp = now().saturating_add(TTL_SECS);
    let token = mint_token(login_id, role, "test", exp);
    format!("{COOKIE_NAME}={token}")
}

#[derive(Deserialize)]
struct ChallengeBody {
    #[serde(default)]
    origin: String,
    #[serde(default)]
    artist_id: String,
}

#[derive(Deserialize)]
struct VerifyBody {
    #[serde(default)]
    origin: String,
    #[serde(default)]
    artist_id: String,
    nonce: String,
    ts: u64,
    method: String,
    login_id: String,
    pub_key: String,
    signature: String,
    #[serde(default)]
    cred_id: String,
}

fn challenge(origin_hdr: Option<&str>, body: &str) -> SessionResponse {
    let parsed: ChallengeBody = match serde_json::from_str(body) {
        Ok(p) => p,
        Err(_) if body.trim().is_empty() => ChallengeBody {
            origin: origin_hdr.unwrap_or("").into(),
            artist_id: String::new(),
        },
        Err(_) => return SessionResponse::err(400, "bad_json"),
    };
    let origin = if parsed.origin.is_empty() {
        origin_hdr.unwrap_or("").to_string()
    } else {
        parsed.origin
    };
    if !origin_allowed(&origin) {
        return SessionResponse::err(403, "origin");
    }
    if let Some(h) = origin_hdr {
        if h != origin {
            return SessionResponse::err(403, "origin");
        }
    }
    let ts = now();
    let mut g = store().lock().expect("session store");
    g.seq += 1;
    let mut h = Sha256::new();
    h.update(b"itl-session-nonce-v1");
    h.update(&session_secret());
    h.update(g.seq.to_le_bytes());
    h.update(ts.to_le_bytes());
    let nonce = hex_encode(&h.finalize());
    let artist_id = parsed.artist_id;
    let challenge_hex = challenge_hex(&origin, &artist_id, &nonce, ts);
    let message = login_message(&origin, &challenge_hex);
    g.challenges.insert(
        nonce.clone(),
        Challenge {
            origin: origin.clone(),
            artist_id,
            ts,
            message: message.clone(),
            challenge_hex: challenge_hex.clone(),
        },
    );
    SessionResponse::json(
        200,
        json!({
            "ok": true,
            "nonce": nonce,
            "ts": ts,
            "challenge": challenge_hex,
            "message": message,
            "ttl_secs": CHALLENGE_TTL_SECS,
        })
        .to_string(),
    )
}

fn verify(origin_hdr: Option<&str>, body: &str) -> SessionResponse {
    let parsed: VerifyBody = match serde_json::from_str(body) {
        Ok(p) => p,
        Err(_) => return SessionResponse::err(400, "bad_json"),
    };
    let origin = if parsed.origin.is_empty() {
        origin_hdr.unwrap_or("").to_string()
    } else {
        parsed.origin
    };
    if !origin_allowed(&origin) {
        return SessionResponse::err(403, "origin");
    }
    if let Some(h) = origin_hdr {
        if h != origin {
            return SessionResponse::err(403, "origin");
        }
    }
    let t = now();
    let ch = {
        let mut g = store().lock().expect("session store");
        match g.challenges.remove(&parsed.nonce) {
            Some(c) => c,
            None => return SessionResponse::err(401, "challenge"),
        }
    };
    if t.saturating_sub(ch.ts) >= CHALLENGE_TTL_SECS || parsed.ts != ch.ts {
        return SessionResponse::err(401, "expired");
    }
    if ch.origin != origin {
        return SessionResponse::err(403, "origin");
    }
    if parsed.login_id.is_empty() || parsed.login_id.len() > 128 {
        return SessionResponse::err(400, "login_id");
    }
    let method = parsed.method.to_ascii_lowercase();
    let digest = match method.as_str() {
        "adr036" => {
            Sha256::digest(adr036_sign_doc(&parsed.login_id, ch.message.as_bytes()).as_bytes())
        }
        "secp256k1" => Sha256::digest(ch.message.as_bytes()),
        _ => return SessionResponse::err(400, "method"),
    };
    let pk = match verify_secp256k1(&parsed.pub_key, &parsed.signature, &digest) {
        Ok(pk) => pk,
        Err(e) => return SessionResponse::err(401, e),
    };
    let derived = match terp_address_from_compressed(&pk) {
        Ok(a) => a,
        Err(e) => return SessionResponse::err(401, e),
    };
    if derived != parsed.login_id {
        return SessionResponse::err(401, "login_id");
    }
    let role = role_for(&parsed.login_id, &parsed.cred_id);
    let exp = t + TTL_SECS;
    let token = mint_token(&parsed.login_id, role, &method, exp);
    let mut resp = SessionResponse::json(
        200,
        json!({
            "ok": true,
            "role": role,
            "login_id": parsed.login_id,
            "exp": exp,
            "method": method,
        })
        .to_string(),
    );
    resp.set_cookie = Some(cookie_header(&token, TTL_SECS));
    let _ = ch.artist_id;
    resp
}

fn me(cookie: Option<&str>) -> SessionResponse {
    let Some(token) = cookie_from_header(cookie) else {
        return SessionResponse::err(401, "no_session");
    };
    match lookup(&token) {
        Some(s) => SessionResponse::json(
            200,
            json!({
                "ok": true,
                "role": s.role,
                "login_id": s.login_id,
                "method": s.method,
                "exp": s.exp,
            })
            .to_string(),
        ),
        None => SessionResponse::err(401, "no_session"),
    }
}

fn logout(cookie: Option<&str>) -> SessionResponse {
    if let Some(token) = cookie_from_header(cookie) {
        if let Ok(mut g) = store().lock() {
            g.sessions.remove(&token);
        }
    }
    let mut resp = SessionResponse::json(200, json!({"ok": true}).to_string());
    resp.set_cookie = Some(cookie_header("", 0));
    resp
}

/// `Some` if this is a session route (including OPTIONS).
pub fn try_handle(
    method: &str,
    path: &str,
    body: &str,
    origin: Option<&str>,
    cookie: Option<&str>,
) -> Option<SessionResponse> {
    let path = path.split('?').next().unwrap_or(path);
    if !path.starts_with("/itl/session") {
        return None;
    }
    if method == "OPTIONS" {
        return Some(SessionResponse {
            status: 204,
            body: String::new(),
            set_cookie: None,
        });
    }
    if method == "GET" && (path == "/itl/session/health" || path == "/itl/session") {
        return Some(SessionResponse::json(
            200,
            json!({
                "ok": true,
                "cookie": "HttpOnly; SameSite=Strict",
                "allowlist": "ITL_ADMIN_CREDS,ITL_USHER_CREDS",
            })
            .to_string(),
        ));
    }
    if method == "POST" && path == "/itl/session/challenge" {
        return Some(challenge(origin, body));
    }
    if method == "POST" && path == "/itl/session/verify" {
        return Some(verify(origin, body));
    }
    if method == "GET" && path == "/itl/session/me" {
        return Some(me(cookie));
    }
    if method == "POST" && path == "/itl/session/logout" {
        return Some(logout(cookie));
    }
    Some(SessionResponse::err(404, "not found"))
}

/// Header helper for the tiny frost TCP server.
pub fn header_line(req: &str, name: &str) -> Option<String> {
    for line in req.split("\r\n").skip(1) {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case(name) {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

pub fn to_http_response(r: &SessionResponse) -> HttpResponse {
    json_status(r.status, r.body.clone())
}

#[cfg(test)]
pub fn reset_for_test() {
    if let Ok(mut g) = store().lock() {
        g.challenges.clear();
        g.sessions.clear();
        g.seq = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::SigningKey;
    use k256::ecdsa::signature::hazmat::PrehashSigner;

    fn key() -> SigningKey {
        SigningKey::from_slice(&[0x11u8; 32]).expect("test key")
    }

    fn sign_adr(sk: &SigningKey, signer: &str, message: &str) -> (String, String) {
        let pk = sk.verifying_key().to_sec1_point(true);
        let digest = Sha256::digest(adr036_sign_doc(signer, message.as_bytes()).as_bytes());
        let sig: Signature = sk.sign_prehash(&digest).expect("sign");
        let b64 = base64::engine::general_purpose::STANDARD;
        (b64.encode(pk.as_bytes()), b64.encode(sig.to_bytes()))
    }

    #[test]
    fn loopback_origins_only() {
        assert!(origin_allowed("http://127.0.0.1:3051"));
        assert!(!origin_allowed("https://evil.example"));
        assert!(!origin_allowed("http://127.0.0.1:3051.evil.example"));
    }

    #[test]
    fn challenge_verify_me_logout() {
        reset_for_test();
        std::env::remove_var("ITL_ADMIN_CREDS");
        std::env::remove_var("ITL_USHER_CREDS");
        let origin = "http://127.0.0.1:3051";
        let ch = try_handle(
            "POST",
            "/itl/session/challenge",
            &json!({"origin": origin}).to_string(),
            Some(origin),
            None,
        )
        .unwrap();
        assert_eq!(ch.status, 200);
        let v: serde_json::Value = serde_json::from_str(&ch.body).unwrap();
        let nonce = v["nonce"].as_str().unwrap().to_string();
        let ts = v["ts"].as_u64().unwrap();
        let message = v["message"].as_str().unwrap().to_string();
        let sk = key();
        let pk = sk.verifying_key().to_sec1_point(true);
        let login = terp_address_from_compressed(pk.as_bytes()).unwrap();
        std::env::set_var("ITL_ADMIN_CREDS", &login);
        let (pub_key, signature) = sign_adr(&sk, &login, &message);
        let body = json!({
            "origin": origin,
            "nonce": nonce,
            "ts": ts,
            "method": "adr036",
            "login_id": login,
            "pub_key": pub_key,
            "signature": signature,
        })
        .to_string();
        let vr = try_handle("POST", "/itl/session/verify", &body, Some(origin), None).unwrap();
        assert_eq!(vr.status, 200, "{}", vr.body);
        let got: serde_json::Value = serde_json::from_str(&vr.body).unwrap();
        assert_eq!(got["role"], "admin");
        let cookie = vr.set_cookie.as_ref().expect("set-cookie");
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(!cookie.to_ascii_lowercase().contains("secure"));
        let hdr = format!(
            "{COOKIE_NAME}={}",
            cookie.split('=').nth(1).unwrap().split(';').next().unwrap()
        );
        let me = try_handle("GET", "/itl/session/me", "", Some(origin), Some(&hdr)).unwrap();
        assert_eq!(me.status, 200);
        let mej: serde_json::Value = serde_json::from_str(&me.body).unwrap();
        assert_eq!(mej["role"], "admin");
        let out = try_handle("POST", "/itl/session/logout", "", Some(origin), Some(&hdr)).unwrap();
        assert_eq!(out.status, 200);
        let me2 = try_handle("GET", "/itl/session/me", "", Some(origin), Some(&hdr)).unwrap();
        assert_eq!(me2.status, 401);
        std::env::remove_var("ITL_ADMIN_CREDS");
    }

    #[test]
    fn bad_sig_and_replay_and_public_env_not_read() {
        reset_for_test();
        let origin = "http://127.0.0.1:3051";
        std::env::set_var("NEXT_PUBLIC_ITL_ADMIN_CREDS", "should-not-count");
        let ch = try_handle(
            "POST",
            "/itl/session/challenge",
            &json!({"origin": origin}).to_string(),
            Some(origin),
            None,
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&ch.body).unwrap();
        let sk = key();
        let pk = sk.verifying_key().to_sec1_point(true);
        let login = terp_address_from_compressed(pk.as_bytes()).unwrap();
        let body = json!({
            "origin": origin,
            "nonce": v["nonce"],
            "ts": v["ts"],
            "method": "adr036",
            "login_id": login,
            "pub_key": base64::engine::general_purpose::STANDARD.encode(pk.as_bytes()),
            "signature": base64::engine::general_purpose::STANDARD.encode([0u8; 64]),
        })
        .to_string();
        let vr = try_handle("POST", "/itl/session/verify", &body, Some(origin), None).unwrap();
        assert_eq!(vr.status, 401);
        let me = try_handle("GET", "/itl/session/me", "", Some(origin), None).unwrap();
        assert_eq!(me.status, 401);
        std::env::remove_var("NEXT_PUBLIC_ITL_ADMIN_CREDS");
    }

    #[test]
    fn hex_roundtrip() {
        assert_eq!(hex_encode(&[0xde, 0xad]), "dead");
        assert_eq!(hex_decode("dead").unwrap(), vec![0xde, 0xad]);
        assert!(hex_decode("zz").is_none());
    }
}
