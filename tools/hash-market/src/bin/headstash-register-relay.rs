//! Loopback register relay for the headstash page.
//!
//! Binds only `127.0.0.1:8791`. The page posts `ctx_hex`, `key_com_hex`,
//! `amount`, and `signature_hex`. This process recovers the signer, attaches
//! the eligibility sibling from a local file, and broadcasts `RegisterHeadstash`.
//! It does not print the body, the signature, or the key commitment.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use ff::PrimeField;
use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
#[cfg(test)]
use k256::ecdsa::SigningKey;
use pasta_curves::pallas;
use sha2::{Digest, Sha256};
use tiny_keccak::{Hasher, Keccak};

const BIND: &str = "127.0.0.1:8791";
const PAGE_ORIGIN: &str = "http://127.0.0.1:3921";
const CONTRACT: &str = "terp1x22q8lfhz7qcvtzs0dakhgx2th64l79kepjujhhxk5x804taeqlquq4ehx";
const CHAIN_ID: &str = "headstash-epoch-e2e";
const NODE: &str = "tcp://127.0.0.1:36667";
const HOME: &str = "/tmp/headstash-epoch-chain";
const FROM: &str = "owner";
const BODY_LIMIT: usize = 4096;

#[derive(Clone)]
struct Cfg {
    terpd: PathBuf,
    home: PathBuf,
}

struct App {
    cfg: Cfg,
    ctx: [u8; 32],
    tree: EligTree,
    busy: tokio::sync::Mutex<()>,
}

#[derive(Clone)]
struct EligTree {
    depth: u32,
    entries: Vec<Elig>,
}

#[derive(Clone)]
struct Elig {
    address: [u8; 20],
    amount: u64,
}

struct Landed {
    txhash: String,
    index: u32,
    count: u32,
    root_hex: String,
}

enum Fail {
    Phrase(&'static str),
    Retry,
    Relay,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let bind = std::env::var("HEADSTASH_RELAY_BIND").unwrap_or_else(|_| BIND.to_string());
    if let Err(phrase) = check_bind(&bind) {
        anyhow::bail!("{phrase}");
    }
    let path = std::env::var("HEADSTASH_ELIGIBILITY")
        .map_err(|_| anyhow::anyhow!("HEADSTASH_ELIGIBILITY is required"))?;
    let tree = load_tree(std::fs::read(&path)?)?;
    let cfg = Cfg {
        terpd: PathBuf::from(
            std::env::var("TERPD").unwrap_or_else(|_| "/tmp/terpd-v6.4.0-abstain".into()),
        ),
        home: PathBuf::from(HOME),
    };
    if !cfg.terpd.is_file() {
        anyhow::bail!("terpd missing");
    }
    let campaign = smart_query(&cfg, r#"{"epoch_campaign":{}}"#)?;
    let data = campaign
        .get("data")
        .ok_or_else(|| anyhow::anyhow!("chain query"))?;
    let ctx = decode_bin32(data.get("ctx").and_then(|v| v.as_str()).unwrap_or(""))
        .ok_or_else(|| anyhow::anyhow!("chain ctx"))?;
    let registration = data
        .get("registration")
        .ok_or_else(|| anyhow::anyhow!("chain registration"))?;
    let chain_root = decode_bin32(
        registration
            .get("eligibility_root")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
    )
    .ok_or_else(|| anyhow::anyhow!("chain eligibility"))?;
    let depth = registration
        .get("eligibility_depth")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("chain depth"))? as u32;
    if depth != tree.depth || chain_root != tree.root() {
        anyhow::bail!(
            "eligibility mismatch chain_prefix {} file_prefix {}",
            prefix(&chain_root),
            prefix(&tree.root())
        );
    }
    let app = Arc::new(App {
        cfg,
        ctx,
        tree,
        busy: tokio::sync::Mutex::new(()),
    });
    let router = Router::new()
        .route("/health", get(health))
        .route(
            "/headstash/register",
            post(register).options(preflight),
        )
        .with_state(app);
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    let local = listener.local_addr()?;
    if !local.ip().is_loopback() || local.port() != 8791 {
        anyhow::bail!("listener is not 127.0.0.1:8791");
    }
    println!("relay_up {bind} depth {depth} root_prefix {}", prefix(&chain_root));
    axum::serve(listener, router).await?;
    Ok(())
}

async fn health(headers: HeaderMap) -> Response {
    reply(StatusCode::OK, serde_json::json!({"ok": true}), origin_of(&headers))
}

async fn preflight(headers: HeaderMap) -> Response {
    reply(
        StatusCode::NO_CONTENT,
        serde_json::Value::Null,
        origin_of(&headers),
    )
}

async fn register(State(app): State<Arc<App>>, req: Request) -> Response {
    let origin = origin_of(req.headers()).map(str::to_string);
    let bytes = match axum::body::to_bytes(req.into_body(), BODY_LIMIT).await {
        Ok(bytes) => bytes,
        Err(_) => return fail(Fail::Relay, origin.as_deref()),
    };
    let parsed = match parse_registration(&bytes) {
        Ok(parsed) => parsed,
        Err(err) => return fail(err, origin.as_deref()),
    };
    if parsed.ctx != app.ctx {
        return fail(Fail::Phrase("registration"), origin.as_deref());
    }
    let message = register_message(&app.ctx, &parsed.key_com, parsed.amount);
    let Some(address) = recover_address(&message, &parsed.signature) else {
        return fail(Fail::Phrase("signature"), origin.as_deref());
    };
    let Some(index) = app.tree.index_of(&address, parsed.amount) else {
        return fail(Fail::Phrase("eligibility"), origin.as_deref());
    };
    let Some(path) = app.tree.path(index) else {
        return fail(Fail::Phrase("eligibility"), origin.as_deref());
    };
    let _guard = app.busy.lock().await;
    let cfg = app.cfg.clone();
    let joined = tokio::task::spawn_blocking(move || {
        broadcast(&cfg, &parsed.key_com, parsed.amount, &parsed.signature, index, &path)
    })
    .await;
    match joined {
        Ok(Ok(landed)) => reply(
            StatusCode::OK,
            serde_json::json!({
                "txhash": landed.txhash,
                "index": landed.index,
                "count": landed.count,
                "root_hex": landed.root_hex,
            }),
            origin.as_deref(),
        ),
        Ok(Err(err)) => fail(err, origin.as_deref()),
        Err(_) => fail(Fail::Relay, origin.as_deref()),
    }
}

struct Parsed {
    ctx: [u8; 32],
    key_com: [u8; 32],
    amount: u64,
    signature: [u8; 65],
}

fn parse_registration(bytes: &Bytes) -> Result<Parsed, Fail> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| Fail::Phrase("registration"))?;
    let ctx = decode_hex64(value.get("ctx_hex").and_then(|v| v.as_str()).unwrap_or(""))
        .ok_or(Fail::Phrase("registration"))?;
    let key_com = decode_hex64(value.get("key_com_hex").and_then(|v| v.as_str()).unwrap_or(""))
        .ok_or(Fail::Phrase("key commitment"))?;
    if !canonical_key(&key_com) {
        return Err(Fail::Phrase("key commitment"));
    }
    let amount = match value.get("amount") {
        Some(serde_json::Value::String(text)) => text.parse::<u64>().map_err(|_| Fail::Phrase("amount"))?,
        Some(serde_json::Value::Number(number)) => number.as_u64().ok_or(Fail::Phrase("amount"))?,
        _ => return Err(Fail::Phrase("amount")),
    };
    if amount == 0 {
        return Err(Fail::Phrase("amount"));
    }
    let signature_hex = value
        .get("signature_hex")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if signature_hex.len() != 130 || !signature_hex.chars().all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()) {
        return Err(Fail::Phrase("signature"));
    }
    let raw = hex::decode(signature_hex).map_err(|_| Fail::Phrase("signature"))?;
    let signature: [u8; 65] = raw.try_into().map_err(|_| Fail::Phrase("signature"))?;
    Ok(Parsed {
        ctx,
        key_com,
        amount,
        signature,
    })
}

fn broadcast(
    cfg: &Cfg,
    key_com: &[u8; 32],
    amount: u64,
    signature: &[u8; 65],
    index: u32,
    path: &[[u8; 32]],
) -> Result<Landed, Fail> {
    let msg = execute_msg(key_com, amount, signature, index, path);
    let txhash = match submit(cfg, &msg, 2_000_000) {
        Err(Fail::Retry) => submit(cfg, &msg, 8_000_000)?,
        other => other?,
    };
    let viewed = poll_tx(cfg, &txhash)?;
    let code = viewed
        .get("code")
        .and_then(|v| v.as_u64())
        .ok_or(Fail::Relay)?;
    if code != 0 {
        let raw = viewed
            .get("raw_log")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        return Err(phrase_in(raw).map(Fail::Phrase).unwrap_or(Fail::Relay));
    }
    let index = event_index(&viewed).unwrap_or(index);
    let tree = smart_query(cfg, r#"{"registration_tree":{}}"#).map_err(|_| Fail::Relay)?;
    let data = tree.get("data").ok_or(Fail::Relay)?;
    let count = data
        .get("count")
        .and_then(|v| v.as_u64())
        .ok_or(Fail::Relay)? as u32;
    let root = decode_bin32(data.get("root").and_then(|v| v.as_str()).unwrap_or("")).ok_or(Fail::Relay)?;
    Ok(Landed {
        txhash,
        index,
        count,
        root_hex: hex::encode(root),
    })
}

fn submit(cfg: &Cfg, msg: &str, gas: u64) -> Result<String, Fail> {
    let gas = gas.to_string();
    let args = [
        "tx",
        "wasm",
        "execute",
        CONTRACT,
        msg,
        "--from",
        FROM,
        "--home",
        HOME,
        "--keyring-backend",
        "test",
        "--chain-id",
        CHAIN_ID,
        "--node",
        NODE,
        "--gas",
        gas.as_str(),
        "--gas-prices",
        "0.025uterp",
        "-y",
        "-b",
        "sync",
        "-o",
        "json",
    ];
    let (stdout, stderr, ok) = run(cfg, &args);
    let text = String::from_utf8_lossy(&stdout);
    let err_text = String::from_utf8_lossy(&stderr);
    if let Some(phrase) = phrase_in(&text).or_else(|| phrase_in(&err_text)) {
        return Err(Fail::Phrase(phrase));
    }
    if !ok {
        if text.contains("out of gas") || err_text.contains("out of gas") {
            return Err(Fail::Retry);
        }
        return Err(Fail::Relay);
    }
    let value = json_from(&stdout).ok_or(Fail::Relay)?;
    let code = value.get("code").and_then(|v| v.as_u64()).unwrap_or(0);
    if code != 0 {
        return Err(Fail::Relay);
    }
    value
        .get("txhash")
        .and_then(|v| v.as_str())
        .filter(|hash| hash.len() == 64 && hash.chars().all(|ch| ch.is_ascii_hexdigit()))
        .map(|hash| hash.to_string())
        .ok_or(Fail::Relay)
}

fn poll_tx(cfg: &Cfg, txhash: &str) -> Result<serde_json::Value, Fail> {
    let args = [
        "query",
        "tx",
        txhash,
        "--node",
        NODE,
        "-o",
        "json",
    ];
    for _ in 0..60 {
        let (stdout, _, ok) = run(cfg, &args);
        if ok {
            if let Some(value) = json_from(&stdout) {
                let view = value.get("tx_response").cloned().unwrap_or(value);
                if view.get("code").is_some() {
                    return Ok(view);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(Fail::Relay)
}

fn smart_query(cfg: &Cfg, query: &str) -> anyhow::Result<serde_json::Value> {
    let args = [
        "query",
        "wasm",
        "contract-state",
        "smart",
        CONTRACT,
        query,
        "--node",
        NODE,
        "-o",
        "json",
    ];
    let (stdout, _, ok) = run(cfg, &args);
    if !ok {
        anyhow::bail!("chain query");
    }
    json_from(&stdout).ok_or_else(|| anyhow::anyhow!("chain query"))
}

fn run(cfg: &Cfg, args: &[&str]) -> (Vec<u8>, Vec<u8>, bool) {
    let mut cmd = Command::new(&cfg.terpd);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear();
    if let Ok(home) = std::env::var("HOME") {
        cmd.env("HOME", home);
    }
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", path);
    }
    cmd.env("TMPDIR", std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into()));
    if let Some(user) = std::env::var_os("USER") {
        cmd.env("USER", user);
    }
    let _ = cfg.home;
    match cmd.output() {
        Ok(output) => (output.stdout, output.stderr, output.status.success()),
        Err(_) => (Vec::new(), Vec::new(), false),
    }
}

fn execute_msg(
    key_com: &[u8; 32],
    amount: u64,
    signature: &[u8; 65],
    index: u32,
    path: &[[u8; 32]],
) -> String {
    let siblings: Vec<String> = path.iter().map(|sibling| b64(sibling)).collect();
    serde_json::json!({
        "register_headstash": {
            "key_com": b64(key_com),
            "amount": amount,
            "signature": b64(signature),
            "eligibility_index": index,
            "eligibility_path": siblings,
        }
    })
    .to_string()
}

fn event_index(view: &serde_json::Value) -> Option<u32> {
    let events = view.get("events")?.as_array()?;
    for event in events {
        if event.get("type").and_then(|v| v.as_str()) != Some("wasm") {
            continue;
        }
        let attrs = event.get("attributes")?.as_array()?;
        let action = attr(attrs, "action");
        if action != Some("register_headstash") {
            continue;
        }
        if let Some(index) = attr(attrs, "index") {
            return index.parse().ok();
        }
    }
    None
}

fn attr<'a>(attrs: &'a [serde_json::Value], key: &str) -> Option<&'a str> {
    attrs.iter().find_map(|attr| {
        let found = attr.get("key").and_then(|v| v.as_str())?;
        if found == key {
            attr.get("value").and_then(|v| v.as_str())
        } else {
            None
        }
    })
}

fn check_bind(bind: &str) -> Result<(), &'static str> {
    if bind == BIND {
        Ok(())
    } else {
        Err("refusing non-loopback bind")
    }
}

fn load_tree(bytes: Vec<u8>) -> anyhow::Result<EligTree> {
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let depth = value
        .get("depth")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("eligibility"))? as u32;
    if !(1..=16).contains(&depth) {
        anyhow::bail!("eligibility");
    }
    let rows = value
        .get("entries")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("eligibility"))?;
    if rows.len() != 1usize << depth {
        anyhow::bail!("eligibility");
    }
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        let address = decode_hex_n(
            row.get("address_hex").and_then(|v| v.as_str()).unwrap_or(""),
            20,
        )
        .ok_or_else(|| anyhow::anyhow!("eligibility"))?;
        let amount = row
            .get("amount")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow::anyhow!("eligibility"))?;
        if amount == 0 {
            anyhow::bail!("eligibility");
        }
        entries.push(Elig {
            address: address.try_into().unwrap(),
            amount,
        });
    }
    Ok(EligTree { depth, entries })
}

impl EligTree {
    fn index_of(&self, address: &[u8; 20], amount: u64) -> Option<u32> {
        self.entries
            .iter()
            .position(|entry| entry.address == *address && entry.amount == amount)
            .map(|index| index as u32)
    }

    fn path(&self, index: u32) -> Option<Vec<[u8; 32]>> {
        if index >= self.entries.len() as u32 {
            return None;
        }
        let mut nodes: Vec<[u8; 32]> = self
            .entries
            .iter()
            .map(|entry| eligibility_leaf(&entry.address, entry.amount))
            .collect();
        let mut cursor = index as usize;
        let mut path = Vec::with_capacity(self.depth as usize);
        for _ in 0..self.depth {
            path.push(nodes[cursor ^ 1]);
            let mut next = Vec::with_capacity(nodes.len() / 2);
            for pair in nodes.chunks(2) {
                next.push(eligibility_node(&pair[0], &pair[1]));
            }
            nodes = next;
            cursor /= 2;
        }
        Some(path)
    }

    fn root(&self) -> [u8; 32] {
        let mut nodes: Vec<[u8; 32]> = self
            .entries
            .iter()
            .map(|entry| eligibility_leaf(&entry.address, entry.amount))
            .collect();
        while nodes.len() > 1 {
            let mut next = Vec::with_capacity(nodes.len() / 2);
            for pair in nodes.chunks(2) {
                next.push(eligibility_node(&pair[0], &pair[1]));
            }
            nodes = next;
        }
        nodes[0]
    }
}

fn eligibility_leaf(address: &[u8; 20], amount: u64) -> [u8; 32] {
    let mut pre = b"terp-hs-elig-leaf-v1".to_vec();
    pre.extend_from_slice(address);
    pre.extend_from_slice(&amount.to_le_bytes());
    sha256(&pre)
}

fn eligibility_node(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut pre = b"terp-hs-elig-node-v1".to_vec();
    pre.extend_from_slice(left);
    pre.extend_from_slice(right);
    sha256(&pre)
}

fn register_message(ctx: &[u8], key_com: &[u8], amount: u64) -> String {
    format!(
        "headstash-register-v1:{}:{}:{amount}",
        hex::encode(ctx),
        hex::encode(key_com)
    )
}

fn recover_address(message: &str, signature: &[u8; 65]) -> Option<[u8; 20]> {
    let recid = match signature[64] {
        0 | 27 => 0,
        1 | 28 => 1,
        _ => return None,
    };
    let digest = keccak256(&eip191(message));
    let sig = Signature::from_slice(&signature[..64]).ok()?;
    let id = RecoveryId::from_byte(recid)?;
    let vk = VerifyingKey::recover_from_prehash(&digest, &sig, id).ok()?;
    let point = vk.to_encoded_point(false);
    let bytes = point.as_bytes();
    if bytes.len() != 65 || bytes[0] != 0x04 {
        return None;
    }
    let hash = keccak256(&bytes[1..]);
    let mut address = [0u8; 20];
    address.copy_from_slice(&hash[12..]);
    Some(address)
}

fn eip191(message: &str) -> Vec<u8> {
    let mut out = b"\x19Ethereum Signed Message:\n".to_vec();
    out.extend_from_slice(message.len().to_string().as_bytes());
    out.extend_from_slice(message.as_bytes());
    out
}

fn canonical_key(bytes: &[u8; 32]) -> bool {
    Option::<pallas::Base>::from(pallas::Base::from_repr(*bytes)).is_some()
}

fn keccak256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    hasher.update(bytes);
    let mut out = [0u8; 32];
    hasher.finalize(&mut out);
    out
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let dig = Sha256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&dig);
    out
}

fn decode_hex64(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.chars().all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()) {
        return None;
    }
    let raw = hex::decode(text).ok()?;
    raw.try_into().ok()
}

fn decode_hex_n(text: &str, len: usize) -> Option<Vec<u8>> {
    if text.len() != len * 2 || !text.chars().all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()) {
        return None;
    }
    hex::decode(text).ok()
}

fn decode_bin32(text: &str) -> Option<[u8; 32]> {
    let raw = base64::engine::general_purpose::STANDARD.decode(text.trim()).ok()?;
    raw.try_into().ok()
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn prefix(bytes: &[u8]) -> String {
    hex::encode(&bytes[..4])
}

fn phrase_in(text: &str) -> Option<&'static str> {
    const PHRASES: &[&str] = &[
        "already registered",
        "tree full",
        "key commitment",
        "anonymity set",
        "eligibility",
        "signature",
        "anchor early",
        "anchor unchanged",
    ];
    PHRASES.iter().copied().find(|phrase| text.contains(phrase))
}

fn json_from(bytes: &[u8]) -> Option<serde_json::Value> {
    let text = std::str::from_utf8(bytes).ok()?;
    let start = text.find('{')?;
    serde_json::from_str(&text[start..]).ok()
}

fn origin_of(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
}

fn fail(err: Fail, origin: Option<&str>) -> Response {
    let phrase = match err {
        Fail::Phrase(phrase) => phrase,
        Fail::Retry | Fail::Relay => "relay",
    };
    reply(
        StatusCode::BAD_REQUEST,
        serde_json::json!({"error": phrase}),
        origin,
    )
}

fn reply(status: StatusCode, body: serde_json::Value, origin: Option<&str>) -> Response {
    let mut response = if body.is_null() {
        status.into_response()
    } else {
        (status, Json(body)).into_response()
    };
    if origin == Some(PAGE_ORIGIN) {
        let headers = response.headers_mut();
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            header::HeaderValue::from_static(PAGE_ORIGIN),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            header::HeaderValue::from_static("content-type"),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            header::HeaderValue::from_static("POST, OPTIONS"),
        );
        headers.insert(header::VARY, header::HeaderValue::from_static("Origin"));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> EligTree {
        EligTree {
            depth: 1,
            entries: vec![
                Elig {
                    address: hex_addr("7e5f4552091a69125d5dfcb7b8c2659029395bdf"),
                    amount: 15,
                },
                Elig {
                    address: hex_addr("2b5ad5c4795c026514f8317c7a215e218dccd6cf"),
                    amount: 15,
                },
            ],
        }
    }

    fn hex_addr(text: &str) -> [u8; 20] {
        let raw = hex::decode(text).unwrap();
        raw.try_into().unwrap()
    }

    #[test]
    fn two_public_leaves_open_the_live_eligibility_root() {
        let opened = tree().root();
        assert!(hex::encode(opened).starts_with("ddfd4cb0"));
        let path = tree().path(1).unwrap();
        assert_eq!(path.len(), 1);
        assert_eq!(path[0], eligibility_leaf(&hex_addr("7e5f4552091a69125d5dfcb7b8c2659029395bdf"), 15));
        let mut node = eligibility_leaf(&hex_addr("2b5ad5c4795c026514f8317c7a215e218dccd6cf"), 15);
        node = eligibility_node(&path[0], &node);
        assert_eq!(node, opened);
    }

    #[test]
    fn signer_byte_two_recovers_the_unused_leaf() {
        let mut secret = [0u8; 32];
        secret[31] = 2;
        let mut key_com = [0u8; 32];
        key_com[0] = 21;
        assert!(canonical_key(&key_com));
        let ctx = [3u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let message = register_message(&ctx, &key_com, 15);
        let key = SigningKey::from_slice(&secret).unwrap();
        let digest = keccak256(&eip191(&message));
        let (signature, recid) = key.sign_prehash_recoverable(&digest).unwrap();
        let raw = signature.to_bytes();
        let slice: &[u8] = raw.as_ref();
        let mut bytes = slice.to_vec();
        bytes.push(recid.to_byte());
        let signature: [u8; 65] = bytes.try_into().unwrap();
        let address = recover_address(&message, &signature).unwrap();
        assert_eq!(hex::encode(address), "2b5ad5c4795c026514f8317c7a215e218dccd6cf");
        assert_eq!(tree().index_of(&address, 15), Some(1));
    }

    #[test]
    fn bind_and_origin_stay_on_the_page_loopback() {
        assert!(check_bind(BIND).is_ok());
        assert!(check_bind("0.0.0.0:8791").is_err());
        assert!(check_bind("127.0.0.1:8792").is_err());
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, header::HeaderValue::from_static(PAGE_ORIGIN));
        let response = reply(StatusCode::OK, serde_json::json!({"ok": true}), origin_of(&headers));
        assert_eq!(
            response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
            PAGE_ORIGIN
        );
        let bare = reply(StatusCode::OK, serde_json::json!({"ok": true}), None);
        assert!(bare.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
        let other = reply(
            StatusCode::OK,
            serde_json::json!({"ok": true}),
            Some("http://127.0.0.1:9999"),
        );
        assert!(other.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
    }

    #[test]
    fn malformed_signature_is_a_phrase_not_an_echo() {
        let body = br#"{"ctx_hex":"aa","key_com_hex":"bb","amount":"15","signature_hex":"ABCD"}"#;
        assert!(matches!(
            parse_registration(&Bytes::from_static(body)),
            Err(Fail::Phrase("registration"))
        ));
    }
}
