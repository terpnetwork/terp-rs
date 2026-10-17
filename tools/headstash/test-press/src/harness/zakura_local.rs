//! Local Zakura (Zcash node) helper for Private Bridge D6.
//!
//! - Detect RPC health (`getblockchaininfo`)
//! - Validate receive address (`validateaddress`)
//! - Map dest display → `owner_binding` with UI domain tag
//! - Shared golden vector: `docs/plans/spectrum/e2e/zakura/golden-dest-binding.json`
//!
//! Zakura core is **not** a wallet: no `getnewaddress` / `z_getnewaccount` without
//! zcashd-compat residual. Default dest is the regtest `miner_address` used by
//! spectrum corridor compose (`tmJym…`).
//!
//! Env:
//! - `ZAKURA_RPC` — JSON-RPC base (default `http://127.0.0.1:18232`)
//! - `ZAKURA_DEST_ADDR` — optional override receive address
//! - `ZAKURA_GOLDEN_DEST_BINDING` — optional path override for golden JSON
//!
//! Honest: regtest / local only — not mainnet ZEC send.
//! Funded-stack sidecar ports (host network): RPC **18232**, P2P **18233**, metrics **19901**.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::env;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Same domain string as PrivateCorridor `zakuraDest.ts` / `mock.ts` `bindingFromDestDisplay`.
pub const DEST_BINDING_DOMAIN_PREFIX: &str = "terp-dest-binding-v0";

/// Transparent regtest address from `docs/plans/spectrum/e2e/zakura/node-corridor.toml`.
pub const REGTEST_MINER_DEST: &str = "tmJymvcUCn1ctbghvTJpXBwHiMEB8P6wxNV";

/// Primary golden `owner_binding` hex for [`REGTEST_MINER_DEST`] (must match golden JSON).
pub const REGTEST_MINER_OWNER_BINDING_HEX: &str =
    "8b5cac11e39905d56126a0c538b84ff8daa379d8009d4e8b121112479607f09b";

pub const DEFAULT_ZAKURA_RPC: &str = "http://127.0.0.1:18232";

/// Default host ports for corridor Zakura regtest compose (HARNESS funded attach).
pub const ZAKURA_RPC_PORT: u16 = 18232;
pub const ZAKURA_P2P_PORT: u16 = 18233;
pub const ZAKURA_METRICS_PORT: u16 = 19901;

#[derive(Clone, Debug)]
pub struct ZakuraLocalConfig {
    pub rpc_url: String,
    /// Timeout per HTTP request.
    pub timeout: Duration,
}

impl Default for ZakuraLocalConfig {
    fn default() -> Self {
        Self {
            rpc_url: env::var("ZAKURA_RPC").unwrap_or_else(|_| DEFAULT_ZAKURA_RPC.into()),
            timeout: Duration::from_secs(5),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZakuraDest {
    /// Address string for UI paste (UA or transparent).
    pub dest_display: String,
    /// 32-byte owner_binding for DepositIntentV0.
    pub owner_binding: [u8; 32],
    pub owner_binding_hex: String,
    pub rpc_validated: bool,
    pub rpc_ready: bool,
    pub chain: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ZakuraLocalError {
    RpcDown(String),
    Rpc(String),
    InvalidAddress(String),
    Io(String),
}

impl std::fmt::Display for ZakuraLocalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RpcDown(s) => write!(f, "zakura rpc down: {s}"),
            Self::Rpc(s) => write!(f, "zakura rpc: {s}"),
            Self::InvalidAddress(s) => write!(f, "invalid address: {s}"),
            Self::Io(s) => write!(f, "io: {s}"),
        }
    }
}

impl std::error::Error for ZakuraLocalError {}

/// Domain-separated dest → owner_binding (UI-compatible).
///
/// Preimage: `terp-dest-binding-v0|` ‖ utf8_trim(dest_display)
pub fn owner_binding_from_dest_display(dest_display: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(DEST_BINDING_DOMAIN_PREFIX.as_bytes());
    h.update(b"|");
    h.update(dest_display.trim().as_bytes());
    h.finalize().into()
}

pub fn owner_binding_hex(dest_display: &str) -> String {
    hex::encode(owner_binding_from_dest_display(dest_display))
}

/// Fail-closed empty dest (UI soft-validate parity).
pub fn reject_empty_dest(dest_display: &str) -> Result<(), ZakuraLocalError> {
    if dest_display.trim().is_empty() {
        return Err(ZakuraLocalError::InvalidAddress("empty destination".into()));
    }
    Ok(())
}

/// Soft prefix check — mirrors UI `softValidateZecDest` (not full bech32m).
pub fn soft_validate_dest_prefix(dest_display: &str) -> bool {
    let s = dest_display.trim();
    if s.is_empty() {
        return false;
    }
    if s.starts_with("u1") || s.starts_with("utest1") || s.starts_with("uregtest1") {
        return s.len() >= 40;
    }
    if s.starts_with("zs1") || s.starts_with("ztestsapling1") {
        return s.len() >= 50;
    }
    if s.starts_with("t1")
        || s.starts_with("t3")
        || s.starts_with("tm")
        || s.starts_with("tex1")
    {
        return s.len() >= 30;
    }
    if s.starts_with("u1sim_") || s.contains("diversified_zec") || s.starts_with("demo_zec_") {
        return true;
    }
    s.len() >= 16
}

/// Resolve dest address for corridor preauth.
pub fn default_dest_display() -> String {
    env::var("ZAKURA_DEST_ADDR").unwrap_or_else(|_| REGTEST_MINER_DEST.into())
}

// ── Golden vector (shared UI ↔ harness ↔ e2e scripts) ─────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoldenDestBindingDoc {
    pub schema: String,
    #[serde(default)]
    pub version: u32,
    pub domain: String,
    #[serde(default)]
    pub samples: Vec<GoldenDestSample>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoldenDestSample {
    pub id: String,
    pub dest_display: String,
    pub owner_binding_hex: String,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GoldenDestError {
    Io(String),
    Json(String),
    DomainMismatch(String),
    SampleMismatch { id: String, expected: String, got: String },
    MissingPrimary,
}

impl std::fmt::Display for GoldenDestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(s) => write!(f, "golden io: {s}"),
            Self::Json(s) => write!(f, "golden json: {s}"),
            Self::DomainMismatch(s) => write!(f, "golden domain: {s}"),
            Self::SampleMismatch { id, expected, got } => {
                write!(f, "golden sample {id}: expected {expected} got {got}")
            }
            Self::MissingPrimary => write!(f, "golden missing primary sample"),
        }
    }
}

impl std::error::Error for GoldenDestError {}

/// Path to shared golden JSON (env override or monorepo relative to this crate).
pub fn default_golden_dest_binding_path() -> PathBuf {
    if let Ok(p) = env::var("ZAKURA_GOLDEN_DEST_BINDING") {
        return PathBuf::from(p);
    }
    // CARGO_MANIFEST_DIR = terp-rs/tools/headstash/test-press
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/e2e/private-bridge/zakura/golden-dest-binding.json")
}

pub fn load_golden_dest_binding(path: &Path) -> Result<GoldenDestBindingDoc, GoldenDestError> {
    let raw = std::fs::read_to_string(path).map_err(|e| GoldenDestError::Io(e.to_string()))?;
    let doc: GoldenDestBindingDoc =
        serde_json::from_str(&raw).map_err(|e| GoldenDestError::Json(e.to_string()))?;
    if doc.domain != DEST_BINDING_DOMAIN_PREFIX {
        return Err(GoldenDestError::DomainMismatch(format!(
            "got {} want {}",
            doc.domain, DEST_BINDING_DOMAIN_PREFIX
        )));
    }
    Ok(doc)
}

/// Assert every sample's `owner_binding_hex` matches [`owner_binding_hex`].
pub fn assert_golden_dest_binding(doc: &GoldenDestBindingDoc) -> Result<(), GoldenDestError> {
    if doc.domain != DEST_BINDING_DOMAIN_PREFIX {
        return Err(GoldenDestError::DomainMismatch(doc.domain.clone()));
    }
    let mut saw_primary = false;
    for s in &doc.samples {
        if s.primary {
            saw_primary = true;
        }
        let got = owner_binding_hex(&s.dest_display);
        let exp = s.owner_binding_hex.to_lowercase();
        if got != exp {
            return Err(GoldenDestError::SampleMismatch {
                id: s.id.clone(),
                expected: exp,
                got,
            });
        }
    }
    if !saw_primary {
        return Err(GoldenDestError::MissingPrimary);
    }
    Ok(())
}

/// Primary sample dest + binding (corridor regtest miner).
pub fn primary_golden_dest() -> Result<ZakuraDest, GoldenDestError> {
    let path = default_golden_dest_binding_path();
    let doc = load_golden_dest_binding(&path)?;
    assert_golden_dest_binding(&doc)?;
    let s = doc
        .samples
        .iter()
        .find(|x| x.primary)
        .ok_or(GoldenDestError::MissingPrimary)?;
    let owner_binding = owner_binding_from_dest_display(&s.dest_display);
    Ok(ZakuraDest {
        dest_display: s.dest_display.clone(),
        owner_binding,
        owner_binding_hex: hex::encode(owner_binding),
        rpc_validated: false,
        rpc_ready: false,
        chain: Some("regtest".into()),
    })
}

// ── Funded-path dest seal (G4) ─────────────────────────────────────────────

/// Sealed dest for funded path / G1 claim dest field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedDestV0 {
    /// Operator/UI paste string (non-authoritative).
    pub dest_display: String,
    /// 32-byte seal — authoritative for watch, claim, mint owner, swap recheck, receipt.
    pub owner_binding: [u8; 32],
    /// Lowercase hex encoding of owner_binding; **width = 64**.
    pub owner_binding_hex: String,
    /// How the seal was obtained.
    pub source: SealedDestSource,
    pub rpc_ready: bool,
    pub rpc_validated: bool,
}

/// How the seal was obtained on the product funded path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SealedDestSource {
    /// Primary sample from golden-dest-binding.json (default funded offline).
    GoldenPrimary,
    /// Env override: `ZAKURA_DEST_ADDR` → digest (must pass soft validate + not placeholder).
    EnvOverride,
    /// Live: dest string + optional validateaddress when ZAKURA_RPC up.
    LiveRpc,
    /// UI paste path (product; same digest domain).
    UiPaste,
}

impl SealedDestSource {
    /// Wire / evidence label (`CORRIDOR_DEST_SEAL_SOURCE`).
    pub fn as_wire_str(&self) -> &'static str {
        match self {
            Self::GoldenPrimary => "golden_primary",
            Self::EnvOverride => "env_override",
            Self::LiveRpc => "live_rpc",
            Self::UiPaste => "ui_paste",
        }
    }
}

/// Fail-closed errors for product dest seal (funded path + claim handoff).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DestSealError {
    EmptyDest,
    PlaceholderBinding(String),
    BadHexWidth { got: usize, want: usize },
    SoftValidateFailed(String),
    Golden(String),
    RpcRequiredButDown(String),
    RpcInvalidAddress(String),
    BindingMismatch {
        left: String,
        right: String,
        surface: String,
    },
}

impl std::fmt::Display for DestSealError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyDest => write!(f, "empty dest_display / dest_owner_binding"),
            Self::PlaceholderBinding(h) => write!(f, "placeholder dest seal rejected: {h}"),
            Self::BadHexWidth { got, want } => {
                write!(f, "dest binding hex width {got} want {want}")
            }
            Self::SoftValidateFailed(s) => write!(f, "soft validate failed: {s}"),
            Self::Golden(s) => write!(f, "golden dest: {s}"),
            Self::RpcRequiredButDown(s) => write!(f, "zakura rpc required but down: {s}"),
            Self::RpcInvalidAddress(s) => write!(f, "zakura rpc invalid address: {s}"),
            Self::BindingMismatch {
                left,
                right,
                surface,
            } => write!(f, "dest binding mismatch on {surface}: {left} != {right}"),
        }
    }
}

impl std::error::Error for DestSealError {}

/// Known lab placeholders that must never ship on product funded path.
pub fn is_placeholder_owner_binding_hex(hex: &str) -> bool {
    let h = hex.trim().to_lowercase();
    if h.is_empty() {
        return true;
    }
    if h.len() != 64 {
        // Width checked separately; treat obvious mono-fillers of any length as placeholder
        // only when exactly 64 for product seal.
        return false;
    }
    if !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }
    h == "b".repeat(64)
        || h == "a".repeat(64)
        || h == "c".repeat(64)
        || h == "0".repeat(64)
        || h.chars().all(|c| c == 'b')
        || h.chars().all(|c| c == 'a')
        || h.chars().all(|c| c == 'c')
        || h.chars().all(|c| c == '0')
}

/// Reject empty / whitespace-only dest_display (seal path).
pub fn reject_empty_dest_seal(dest_display: &str) -> Result<(), DestSealError> {
    if dest_display.trim().is_empty() {
        return Err(DestSealError::EmptyDest);
    }
    Ok(())
}

/// Fail-closed: known lab placeholders as dest seal.
pub fn reject_placeholder_binding_hex(hex: &str) -> Result<(), DestSealError> {
    let h = hex.trim().to_lowercase();
    if h.is_empty() {
        return Err(DestSealError::EmptyDest);
    }
    if is_placeholder_owner_binding_hex(&h) {
        return Err(DestSealError::PlaceholderBinding(h));
    }
    Ok(())
}

/// Exactly 64 lowercase hex chars → 32 bytes (after normalize).
pub fn require_binding_hex_width(hex: &str) -> Result<[u8; 32], DestSealError> {
    let h = hex.trim().to_lowercase();
    if h.is_empty() {
        return Err(DestSealError::EmptyDest);
    }
    if h.len() != 64 {
        return Err(DestSealError::BadHexWidth {
            got: h.len(),
            want: 64,
        });
    }
    if !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(DestSealError::BadHexWidth {
            got: h.len(),
            want: 64,
        });
    }
    let bytes = hex::decode(&h).map_err(|_| DestSealError::BadHexWidth {
        got: h.len(),
        want: 64,
    })?;
    let mut out = [0u8; 32];
    if bytes.len() != 32 {
        return Err(DestSealError::BadHexWidth {
            got: h.len(),
            want: 64,
        });
    }
    out.copy_from_slice(&bytes);
    Ok(out)
}

/// Assert continuous dest identity across surfaces (hex, lowercase, 64).
pub fn assert_dest_binding_equal(
    sealed_hex: &str,
    other_hex: &str,
    surface: &str,
) -> Result<(), DestSealError> {
    let left = sealed_hex.trim().to_lowercase();
    let right = other_hex.trim().to_lowercase();
    if left.is_empty() || right.is_empty() {
        return Err(DestSealError::EmptyDest);
    }
    if left != right {
        return Err(DestSealError::BindingMismatch {
            left,
            right,
            surface: surface.into(),
        });
    }
    Ok(())
}

fn env_truthy_dest(key: &str) -> bool {
    matches!(
        env::var(key).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    )
}

/// Resolve sealed dest for **product funded path** (never invent placeholder).
///
/// Order:
/// 1. If `ZAKURA_DEST_ADDR` set → digest that (after reject_empty + reject_placeholder + soft prefix).
/// 2. Else → `primary_golden_dest()` from golden JSON / `REGTEST_MINER_*`.
/// 3. If RPC up (`ZAKURA_RPC`): optional `validateaddress`; set flags; **do not fail closed**
///    when RPC down (offline golden remains valid). Fail closed only when
///    `CORRIDOR_REQUIRE_ZAKURA_RPC=1` and RPC down / invalid.
/// 4. If `CORRIDOR_DEST_OWNER_BINDING` set, must equal sealed hex (no silent invent).
pub fn seal_funded_dest(cfg: &ZakuraLocalConfig) -> Result<SealedDestV0, DestSealError> {
    let env_override = env::var("ZAKURA_DEST_ADDR").ok().filter(|s| !s.trim().is_empty());
    let (dest_display, mut source) = if let Some(addr) = env_override {
        reject_empty_dest_seal(&addr)?;
        if !soft_validate_dest_prefix(&addr) {
            return Err(DestSealError::SoftValidateFailed(addr));
        }
        let owner_binding = owner_binding_from_dest_display(&addr);
        let owner_binding_hex = hex::encode(owner_binding);
        reject_placeholder_binding_hex(&owner_binding_hex)?;
        (addr.trim().to_string(), SealedDestSource::EnvOverride)
    } else {
        let g = primary_golden_dest().map_err(|e| DestSealError::Golden(e.to_string()))?;
        reject_placeholder_binding_hex(&g.owner_binding_hex)?;
        (g.dest_display, SealedDestSource::GoldenPrimary)
    };

    let owner_binding = owner_binding_from_dest_display(&dest_display);
    let owner_binding_hex = hex::encode(owner_binding);
    reject_placeholder_binding_hex(&owner_binding_hex)?;

    if let Ok(pin) = env::var("CORRIDOR_DEST_OWNER_BINDING") {
        let pin = pin.trim().to_lowercase();
        if !pin.is_empty() {
            reject_placeholder_binding_hex(&pin)?;
            require_binding_hex_width(&pin)?;
            assert_dest_binding_equal(&owner_binding_hex, &pin, "CORRIDOR_DEST_OWNER_BINDING")?;
        }
    }

    let require_rpc = env_truthy_dest("CORRIDOR_REQUIRE_ZAKURA_RPC");
    let ready = rpc_ready(cfg);
    let mut rpc_validated = false;
    if ready {
        if source == SealedDestSource::EnvOverride {
            source = SealedDestSource::LiveRpc;
        } else if source == SealedDestSource::GoldenPrimary {
            // Keep GoldenPrimary as source of truth for dest selection; flags still reflect RPC.
        }
        match validate_address(cfg, &dest_display) {
            Ok(true) => rpc_validated = true,
            Ok(false) => {
                if require_rpc {
                    return Err(DestSealError::RpcInvalidAddress(dest_display));
                }
            }
            Err(e) => {
                if require_rpc {
                    return Err(DestSealError::RpcRequiredButDown(e.to_string()));
                }
            }
        }
    } else if require_rpc {
        return Err(DestSealError::RpcRequiredButDown(cfg.rpc_url.clone()));
    }

    Ok(SealedDestV0 {
        dest_display,
        owner_binding,
        owner_binding_hex,
        source,
        rpc_ready: ready,
        rpc_validated,
    })
}

/// Minimal JSON-RPC over HTTP/1.0 (no extra deps).
pub fn json_rpc_call(
    cfg: &ZakuraLocalConfig,
    method: &str,
    params: Value,
) -> Result<Value, ZakuraLocalError> {
    let url = cfg.rpc_url.trim_end_matches('/');
    let (host, port, path) = parse_http_url(url)?;
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    })
    .to_string();
    let req = format!(
        "POST {path} HTTP/1.0\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect_timeout(
        &format!("{host}:{port}")
            .parse()
            .map_err(|e| ZakuraLocalError::Io(format!("parse addr: {e}")))?,
        cfg.timeout,
    )
    .map_err(|e| ZakuraLocalError::RpcDown(e.to_string()))?;
    stream
        .set_read_timeout(Some(cfg.timeout))
        .map_err(|e| ZakuraLocalError::Io(e.to_string()))?;
    stream
        .set_write_timeout(Some(cfg.timeout))
        .map_err(|e| ZakuraLocalError::Io(e.to_string()))?;
    stream
        .write_all(req.as_bytes())
        .map_err(|e| ZakuraLocalError::Io(e.to_string()))?;
    let mut resp = String::new();
    stream
        .read_to_string(&mut resp)
        .map_err(|e| ZakuraLocalError::Io(e.to_string()))?;
    let body = resp
        .split("\r\n\r\n")
        .nth(1)
        .or_else(|| resp.split("\n\n").nth(1))
        .ok_or_else(|| ZakuraLocalError::Rpc("no HTTP body".into()))?;
    let v: Value = serde_json::from_str(body)
        .map_err(|e| ZakuraLocalError::Rpc(format!("json: {e}; body={body}")))?;
    if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
        return Err(ZakuraLocalError::Rpc(err.to_string()));
    }
    Ok(v.get("result").cloned().unwrap_or(Value::Null))
}

fn parse_http_url(url: &str) -> Result<(String, u16, String), ZakuraLocalError> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| ZakuraLocalError::Io(format!("only http:// supported: {url}")))?;
    let (hostport, path) = match rest.split_once('/') {
        Some((hp, p)) => (hp, format!("/{p}")),
        None => (rest, "/".into()),
    };
    let (host, port) = if let Some((h, p)) = hostport.split_once(':') {
        (
            h.to_string(),
            p.parse()
                .map_err(|e| ZakuraLocalError::Io(format!("port: {e}")))?,
        )
    } else {
        (hostport.to_string(), 80u16)
    };
    Ok((host, port, path))
}

/// True if `getblockchaininfo` returns a result object.
pub fn rpc_ready(cfg: &ZakuraLocalConfig) -> bool {
    matches!(json_rpc_call(cfg, "getblockchaininfo", json!([])), Ok(Value::Object(_)))
}

pub fn get_blockchain_info(cfg: &ZakuraLocalConfig) -> Result<Value, ZakuraLocalError> {
    json_rpc_call(cfg, "getblockchaininfo", json!([]))
}

/// Call `validateaddress`; returns isvalid flag when present.
pub fn validate_address(cfg: &ZakuraLocalConfig, addr: &str) -> Result<bool, ZakuraLocalError> {
    let r = json_rpc_call(cfg, "validateaddress", json!([addr]))?;
    Ok(r.get("isvalid").and_then(|v| v.as_bool()).unwrap_or(false))
}

/// Fetch dest for corridor: offline digest always; live validate when RPC up.
pub fn fetch_corridor_dest(cfg: &ZakuraLocalConfig) -> ZakuraDest {
    let dest_display = default_dest_display();
    let owner_binding = owner_binding_from_dest_display(&dest_display);
    let owner_binding_hex = hex::encode(owner_binding);
    let mut out = ZakuraDest {
        dest_display,
        owner_binding,
        owner_binding_hex,
        rpc_validated: false,
        rpc_ready: false,
        chain: None,
    };
    if !rpc_ready(cfg) {
        return out;
    }
    out.rpc_ready = true;
    if let Ok(info) = get_blockchain_info(cfg) {
        out.chain = info
            .get("chain")
            .and_then(|c| c.as_str())
            .map(|s| s.to_string());
    }
    if let Ok(true) = validate_address(cfg, &out.dest_display) {
        out.rpc_validated = true;
    }
    out
}

/// Like [`fetch_corridor_dest`] but errors if RPC is required and down.
pub fn fetch_corridor_dest_require_rpc(
    cfg: &ZakuraLocalConfig,
) -> Result<ZakuraDest, ZakuraLocalError> {
    if !rpc_ready(cfg) {
        return Err(ZakuraLocalError::RpcDown(cfg.rpc_url.clone()));
    }
    let d = fetch_corridor_dest(cfg);
    if !d.rpc_validated {
        return Err(ZakuraLocalError::InvalidAddress(format!(
            "{} not valid on {}",
            d.dest_display, cfg.rpc_url
        )));
    }
    Ok(d)
}

/// Run cashapp W0–W7 with dest_owner_binding from Zakura dest (when available).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::cashapp_zec_corridor::{
        run_cashapp_zec_corridor_w0_w7, CorridorAssetBackend, CorridorScenario,
    };

    #[test]
    fn owner_binding_domain_stable_and_ui_compatible() {
        let a = owner_binding_from_dest_display(REGTEST_MINER_DEST);
        let b = owner_binding_from_dest_display(REGTEST_MINER_DEST);
        assert_eq!(a, b);
        assert_eq!(hex::encode(a).len(), 64);
        assert_eq!(hex::encode(a), REGTEST_MINER_OWNER_BINDING_HEX);
        // Different dest → different bind
        let c = owner_binding_from_dest_display("u1different_dest");
        assert_ne!(a, c);
        // Prefix matters
        let mut h = Sha256::new();
        h.update(REGTEST_MINER_DEST.as_bytes());
        let wrong: [u8; 32] = h.finalize().into();
        assert_ne!(a, wrong);
        // Trim whitespace
        let d = owner_binding_from_dest_display(&format!("  {REGTEST_MINER_DEST}  "));
        assert_eq!(a, d);
        assert!(reject_empty_dest("").is_err());
        assert!(reject_empty_dest("   ").is_err());
        assert!(soft_validate_dest_prefix(REGTEST_MINER_DEST));
        assert!(!soft_validate_dest_prefix(""));
    }

    #[test]
    fn golden_vector_matches_owner_binding() {
        let path = default_golden_dest_binding_path();
        assert!(
            path.is_file(),
            "missing golden vector at {} — create docs/plans/spectrum/e2e/zakura/golden-dest-binding.json",
            path.display()
        );
        let doc = load_golden_dest_binding(&path).expect("load golden");
        assert_eq!(doc.domain, DEST_BINDING_DOMAIN_PREFIX);
        assert_golden_dest_binding(&doc).expect("golden samples match harness digest");
        let primary = primary_golden_dest().expect("primary");
        assert_eq!(primary.dest_display, REGTEST_MINER_DEST);
        assert_eq!(primary.owner_binding_hex, REGTEST_MINER_OWNER_BINDING_HEX);
        eprintln!(
            "golden OK path={} primary_binding={}",
            path.display(),
            primary.owner_binding_hex
        );
    }

    #[test]
    fn fetch_dest_offline_still_produces_binding() {
        // Point at closed port — should not panic
        let cfg = ZakuraLocalConfig {
            rpc_url: "http://127.0.0.1:1".into(),
            timeout: Duration::from_millis(200),
        };
        let d = fetch_corridor_dest(&cfg);
        assert!(!d.rpc_ready);
        assert!(!d.rpc_validated);
        assert_eq!(d.owner_binding, owner_binding_from_dest_display(&d.dest_display));
        // Default dest is primary golden when env unset
        if env::var("ZAKURA_DEST_ADDR").is_err() {
            assert_eq!(d.dest_display, REGTEST_MINER_DEST);
            assert_eq!(d.owner_binding_hex, REGTEST_MINER_OWNER_BINDING_HEX);
        }
    }

    /// Live path: only runs assertions when ZAKURA_RPC is up (clean skip otherwise).
    #[test]
    fn zakura_rpc_health_and_validate_when_available() {
        let cfg = ZakuraLocalConfig::default();
        if !rpc_ready(&cfg) {
            eprintln!(
                "skip live Zakura: RPC not at {} (run docs/plans/spectrum/e2e/zakura/zakura-local.sh up)",
                cfg.rpc_url
            );
            return;
        }
        let info = get_blockchain_info(&cfg).expect("getblockchaininfo");
        assert!(info.get("chain").is_some() || info.get("blocks").is_some());
        let d = fetch_corridor_dest(&cfg);
        assert!(d.rpc_ready);
        // Regtest miner dest should validate on regtest node
        assert!(
            d.rpc_validated || d.chain.as_deref() != Some("regtest"),
            "expected validateaddress true on regtest for {}",
            d.dest_display
        );
        // Live dest binding still matches golden domain
        assert_eq!(
            d.owner_binding_hex,
            owner_binding_hex(&d.dest_display),
            "live dest binding must use terp-dest-binding-v0"
        );
        eprintln!(
            "zakura live OK chain={:?} dest={} binding={}",
            d.chain, d.dest_display, d.owner_binding_hex
        );
    }

    /// When Zakura up: W0–W7 with real dest binding from address string.
    #[test]
    fn cashapp_w0_w7_with_zakura_dest_when_available() {
        let cfg = ZakuraLocalConfig::default();
        if !rpc_ready(&cfg) {
            eprintln!("skip cashapp+zakura: RPC not available");
            return;
        }
        let d = fetch_corridor_dest(&cfg);
        assert!(d.rpc_ready);
        let mut backend = CorridorAssetBackend::simulated();
        let scenario = CorridorScenario {
            dest_owner_binding: d.owner_binding,
            suite_label: "zakura-local-dest".into(),
            ..CorridorScenario::default()
        };
        let out = run_cashapp_zec_corridor_w0_w7(&mut backend, &scenario)
            .expect("W0–W7 with Zakura-derived dest_owner_binding");
        assert_eq!(out.mint_note.owner_binding, d.owner_binding);
        assert_eq!(
            out.receipt.dest_owner_binding_hex,
            d.owner_binding_hex
        );
    }

    /// Offline W0–W7 using primary golden dest binding (funded-profile dest film).
    #[test]
    fn cashapp_w0_w7_with_golden_primary_dest() {
        let d = primary_golden_dest().expect("primary golden");
        let mut backend = CorridorAssetBackend::simulated();
        let scenario = CorridorScenario {
            dest_owner_binding: d.owner_binding,
            suite_label: "zakura-golden-dest".into(),
            ..CorridorScenario::default()
        };
        let out = run_cashapp_zec_corridor_w0_w7(&mut backend, &scenario)
            .expect("W0–W7 with golden dest_owner_binding");
        assert_eq!(out.mint_note.owner_binding, d.owner_binding);
        assert_eq!(out.receipt.dest_owner_binding_hex, REGTEST_MINER_OWNER_BINDING_HEX);
    }

    /// T2: seal_funded_dest → primary golden when ZAKURA_DEST_ADDR unset.
    #[test]
    fn seal_funded_dest_primary_golden_when_env_unset() {
        // Avoid polluting other tests if env is set by operator.
        if env::var("ZAKURA_DEST_ADDR").is_ok() {
            eprintln!("skip seal_funded_dest_primary: ZAKURA_DEST_ADDR set");
            return;
        }
        if env::var("CORRIDOR_DEST_OWNER_BINDING").is_ok() {
            eprintln!("skip seal_funded_dest_primary: CORRIDOR_DEST_OWNER_BINDING set");
            return;
        }
        let cfg = ZakuraLocalConfig {
            rpc_url: "http://127.0.0.1:1".into(),
            timeout: Duration::from_millis(150),
        };
        let sealed = seal_funded_dest(&cfg).expect("seal offline golden");
        assert_eq!(sealed.dest_display, REGTEST_MINER_DEST);
        assert_eq!(sealed.owner_binding_hex, REGTEST_MINER_OWNER_BINDING_HEX);
        assert_eq!(sealed.source, SealedDestSource::GoldenPrimary);
        assert!(!sealed.rpc_ready);
        assert!(!is_placeholder_owner_binding_hex(&sealed.owner_binding_hex));
        eprintln!(
            "seal_funded_dest OK source={} binding={}",
            sealed.source.as_wire_str(),
            sealed.owner_binding_hex
        );
    }

    /// T3: reject known placeholders + empty + width fail.
    #[test]
    fn reject_placeholder_binding_hex_product_path() {
        assert!(reject_placeholder_binding_hex("").is_err());
        assert!(reject_placeholder_binding_hex(&"b".repeat(64)).is_err());
        assert!(reject_placeholder_binding_hex(&"B".repeat(64)).is_err());
        assert!(reject_placeholder_binding_hex(&"0".repeat(64)).is_err());
        assert!(reject_placeholder_binding_hex(&"a".repeat(64)).is_err());
        assert!(reject_placeholder_binding_hex(&"c".repeat(64)).is_err());
        assert!(is_placeholder_owner_binding_hex(&"b".repeat(64)));
        assert!(!is_placeholder_owner_binding_hex(REGTEST_MINER_OWNER_BINDING_HEX));
        assert!(require_binding_hex_width("abcd").is_err());
        assert!(require_binding_hex_width(REGTEST_MINER_OWNER_BINDING_HEX).is_ok());
        assert!(assert_dest_binding_equal(
            REGTEST_MINER_OWNER_BINDING_HEX,
            REGTEST_MINER_OWNER_BINDING_HEX,
            "self"
        )
        .is_ok());
        assert!(assert_dest_binding_equal(
            REGTEST_MINER_OWNER_BINDING_HEX,
            &"b".repeat(64),
            "watch"
        )
        .is_err());
    }

    /// Funded film: seal → scenario → receipt equality (T9 shape).
    #[test]
    fn cashapp_w0_w7_with_seal_funded_dest() {
        if env::var("ZAKURA_DEST_ADDR").is_ok() || env::var("CORRIDOR_DEST_OWNER_BINDING").is_ok() {
            eprintln!("skip seal film: dest env override present");
            return;
        }
        let cfg = ZakuraLocalConfig {
            rpc_url: "http://127.0.0.1:1".into(),
            timeout: Duration::from_millis(150),
        };
        let sealed = seal_funded_dest(&cfg).expect("seal");
        let mut backend = CorridorAssetBackend::simulated();
        let scenario = CorridorScenario {
            dest_owner_binding: sealed.owner_binding,
            suite_label: "zakura-sealed-funded".into(),
            ..CorridorScenario::default()
        };
        let out = run_cashapp_zec_corridor_w0_w7(&mut backend, &scenario)
            .expect("W0–W7 with sealed dest");
        assert_dest_binding_equal(
            &sealed.owner_binding_hex,
            &out.receipt.dest_owner_binding_hex,
            "receipt",
        )
        .expect("receipt == sealed");
        assert_ne!(out.receipt.dest_owner_binding_hex, "b".repeat(64));
    }
}
