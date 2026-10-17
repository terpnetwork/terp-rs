//! Option D Zcash leg: burn-gated pay — **product** threshold escrow or **lab** inventory.
//!
//! **Not Option B:** [`lab_pay_zec_after_burn`] refuses missing evidence, dest seal
//! mismatch, or zero value.
//!
//! ## Release policy (`CORRIDOR_ZEC_RELEASE`)
//!
//! | Value | Behavior |
//! |-------|----------|
//! | `threshold_escrow` (product) | Committee signs escrow-release digest →
//!   [`authorize_escrow_release`] → optional send from escrow (P-lab). Modes:
//!   `threshold_escrow_release` / `threshold_escrow_release_simulated`. |
//! | `lab_inventory` (residual) | Host wallet inventory pay. Modes:
//!   `lab_inventory_pay` / `lab_inventory_pay_simulated`. |
//!
//! Default: `lab_inventory` for legacy demos; **omni/product profiles** default to
//! `threshold_escrow`. Inventory-only under product profile is **refused** unless
//! `CORRIDOR_ALLOW_LAB_INVENTORY_RESIDUAL=1` (must FAIL product green).
//!
//! Threshold crypto is in-house **lab t-of-n multi-sig** (`threshold_committee`),
//! same digests/wire as hash-market `ThresholdCommitteeCustody` — **not FROST**.
//!
//! Env:
//! - `CORRIDOR_ZEC_RELEASE=threshold_escrow|lab_inventory`
//! - `CORRIDOR_PROFILE=omni_production_shaped|omni|product|…`
//! - `CORRIDOR_ALLOW_LAB_INVENTORY_RESIDUAL=1` — residual escape hatch only
//! - `CORRIDOR_LAB_ZEC_FORCE_SIMULATED=1` — force simulated send even when wallet RPC up
//! - `CORRIDOR_THRESHOLD_T` / `CORRIDOR_THRESHOLD_N` — lab committee params (default 2/3)
//! - `HASH_MARKET_URL` / `CORRIDOR_HASH_MARKET_URL` — live hashmerchant for release sig
//! - `CORRIDOR_REQUIRE_LIVE_THRESHOLD=1` — refuse in-process lab_sign fallback
//!
//! Auth preference (product path):
//! 1. Live `POST {HASH_MARKET_URL}/escrow-release/sign` (hashmerchant process)
//! 2. In-process `LabCommittee::generate` only as labeled residual unless require-live
//!
//! Types SSOT: `terp_seams::dex::egress` (PURE-EGRESS + authorize_escrow_release).
//! Host attach: Zakura RPC + G4 [`SealedDestV0`].

use crate::harness::zakura_local::{
    SealedDestV0, ZakuraLocalConfig, ZakuraLocalError, assert_dest_binding_equal,
    is_placeholder_owner_binding_hex, json_rpc_call, reject_placeholder_binding_hex, rpc_ready,
    soft_validate_dest_prefix, validate_address,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use terp_seams::dex::{
    DestKind, EgressBurnEvidenceV0, EgressBurnPublic, ZecEgressReceiptV0, hex32,
    lab_receipt_after_burn,
};
use threshold_committee::{LabCommittee, lab_sign_with_threshold};

/// Lab mode when RPC down or send RPCs unavailable — synthetic inventory film.
pub const MODE_LAB_INVENTORY_PAY_SIMULATED: &str = "lab_inventory_pay_simulated";
/// Lab mode when a real Zcash send RPC returned a txid.
pub const MODE_LAB_INVENTORY_PAY: &str = "lab_inventory_pay";
/// Reserved prod residual (not implemented this wave).
pub const MODE_LC_MINT: &str = "lc_mint";

/// Synthetic txid prefix (must appear in mock receipt `zec_txid`).
pub const LAB_SIMULATED_TXID_PREFIX: &str = "lab_inventory_pay_simulated";
/// Synthetic txid prefix for threshold path when send is simulated (P-lab offline).
pub const THRESHOLD_SIMULATED_TXID_PREFIX: &str = "threshold_escrow_release_simulated";
/// Synthetic txid prefix for FROST Object A path when send is simulated.
pub const FROST_SIMULATED_TXID_PREFIX: &str = "frost_escrow_release_simulated";
/// Synthetic when Object B signed but remote wallet send residual.
pub const FROST_SPEND_SIMULATED_TXID_PREFIX: &str = "frost_escrow_spend_simulated";

/// Honesty label: lab multi-sig, not FROST claim.
pub const COMMITTEE_CRYPTO_LAB_T_OF_N: &str = "lab_t_of_n_multisig";
/// Honesty label: real ZF FROST lab (Object A digest).
pub const COMMITTEE_CRYPTO_FROST_LAB: &str = "frost_ed25519_lab";
/// P-lab: committee auth gates send; harness may hold escrow key.
pub const ESCROW_SIGN_PATH_P_LAB: &str = "P-lab";
/// Release sig obtained from running hashmerchant process API.
pub const AUTH_SOURCE_HASHMERCHANT_LIVE: &str = "hashmerchant_live";
/// Release sig from in-process lab DKG (residual when live server down).
pub const AUTH_SOURCE_IN_PROCESS_LAB: &str = "in_process_lab";

/// Env keys (product demo).
pub const ENV_CORRIDOR_ZEC_RELEASE: &str = "CORRIDOR_ZEC_RELEASE";
pub const ENV_CORRIDOR_PROFILE: &str = "CORRIDOR_PROFILE";
pub const ENV_CORRIDOR_ALLOW_LAB_INVENTORY_RESIDUAL: &str = "CORRIDOR_ALLOW_LAB_INVENTORY_RESIDUAL";
pub const ENV_CORRIDOR_THRESHOLD_T: &str = "CORRIDOR_THRESHOLD_T";
pub const ENV_CORRIDOR_THRESHOLD_N: &str = "CORRIDOR_THRESHOLD_N";
pub const ENV_HASH_MARKET_URL: &str = "HASH_MARKET_URL";
pub const ENV_CORRIDOR_HASH_MARKET_URL: &str = "CORRIDOR_HASH_MARKET_URL";
pub const ENV_CORRIDOR_REQUIRE_LIVE_THRESHOLD: &str = "CORRIDOR_REQUIRE_LIVE_THRESHOLD";

/// Open/confirm skip labels (honest residual — not "ZEC received" claims).
pub const OPEN_CONFIRM_RPC_OK: &str = "rpc_open_confirm";
pub const OPEN_CONFIRM_SKIP_NO_RPC: &str = "skip_clean_no_rpc";
pub const OPEN_CONFIRM_SKIP_NO_WALLET: &str = "skip_clean_no_wallet_balance";
pub const OPEN_CONFIRM_SKIP_VALIDATE_ONLY: &str = "rpc_validate_only";

/// Product vs residual release policy for the Zcash leg.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZecReleasePolicy {
    /// OmniBridge-shaped: burn + dest + threshold auth → escrow release.
    ThresholdEscrow,
    /// Lab residual: inventory / host wallet pay (not product green alone).
    LabInventory,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LabPayError {
    /// Product / lab fail-closed: no burn evidence payload.
    EvidenceMissing(String),
    DestMismatch {
        evidence: String,
        sealed: String,
    },
    ValueZero,
    SoftValidateFailed(String),
    PlaceholderDest(String),
    /// Threshold committee auth failed or missing.
    ThresholdAuth(String),
    /// Product profile refused inventory-only / funder-only release mode.
    ProductRejectFunderOnly(String),
    Pure(String),
    Rpc(String),
    Io(String),
    Json(String),
}

impl std::fmt::Display for LabPayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EvidenceMissing(s) => write!(f, "egress burn evidence missing: {s}"),
            Self::DestMismatch { evidence, sealed } => {
                write!(
                    f,
                    "egress dest mismatch: evidence={evidence} sealed={sealed}"
                )
            }
            Self::ValueZero => write!(f, "egress value is 0 — refuse Zcash pay"),
            Self::SoftValidateFailed(s) => write!(f, "dest soft validate failed: {s}"),
            Self::PlaceholderDest(s) => write!(f, "placeholder dest rejected: {s}"),
            Self::ThresholdAuth(s) => write!(f, "threshold escrow auth: {s}"),
            Self::ProductRejectFunderOnly(s) => {
                write!(f, "product profile rejects funder-only release: {s}")
            }
            Self::Pure(s) => write!(f, "pure egress: {s}"),
            Self::Rpc(s) => write!(f, "zakura rpc: {s}"),
            Self::Io(s) => write!(f, "io: {s}"),
            Self::Json(s) => write!(f, "json: {s}"),
        }
    }
}

impl std::error::Error for LabPayError {}

impl From<ZakuraLocalError> for LabPayError {
    fn from(e: ZakuraLocalError) -> Self {
        Self::Rpc(e.to_string())
    }
}

/// Infer dest kind from display prefix (UI soft-validate parity).
pub fn dest_kind_from_display(dest_display: &str) -> DestKind {
    let s = dest_display.trim();
    if s.starts_with("u1")
        || s.starts_with("utest1")
        || s.starts_with("uregtest1")
        || s.starts_with("zs1")
        || s.starts_with("ztestsapling1")
        || s.starts_with("u1sim_")
    {
        DestKind::Shielded
    } else {
        DestKind::Transparent
    }
}

/// Default path for ZEC egress receipt artifact.
pub fn zec_egress_receipt_path() -> PathBuf {
    std::env::var("CORRIDOR_ZEC_EGRESS_RECEIPT_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/corridor-zec-egress-receipt.json"))
}

/// Default path for burn evidence artifact.
pub fn egress_burn_evidence_path() -> PathBuf {
    std::env::var("CORRIDOR_EGRESS_BURN_EVIDENCE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/corridor-egress-burn-evidence.json"))
}

/// Synthetic labeled txid for offline / send-unavailable lab path.
pub fn synthetic_lab_txid(nullifier: &[u8; 32], amount_zat: u64) -> String {
    let mut h = Sha256::new();
    h.update(LAB_SIMULATED_TXID_PREFIX.as_bytes());
    h.update(b"|");
    h.update(nullifier);
    h.update(amount_zat.to_le_bytes());
    let dig = h.finalize();
    format!("{LAB_SIMULATED_TXID_PREFIX}:{}", hex::encode(&dig[..16]))
}

/// Gate: evidence present + dest seal equality + value > 0.
pub fn validate_lab_pay_gates(
    evidence: &EgressBurnEvidenceV0,
    sealed: &SealedDestV0,
) -> Result<(), LabPayError> {
    if evidence.proof_mode.trim().is_empty() {
        return Err(LabPayError::EvidenceMissing("proof_mode empty".into()));
    }
    if evidence.burn.value == 0 {
        return Err(LabPayError::ValueZero);
    }
    if evidence.burn.nullifier == [0u8; 32] {
        return Err(LabPayError::EvidenceMissing(
            "nullifier all-zero (egress ν missing)".into(),
        ));
    }
    if evidence.burn.cm_spent == [0u8; 32] {
        return Err(LabPayError::EvidenceMissing("cm_spent all-zero".into()));
    }
    if evidence.burn.dest_commitment == [0u8; 32] {
        return Err(LabPayError::EvidenceMissing(
            "dest_commitment all-zero".into(),
        ));
    }

    let sealed_hex = sealed.owner_binding_hex.trim().to_lowercase();
    let evidence_hex = hex32(&evidence.burn.dest_commitment);
    if sealed_hex.is_empty() {
        return Err(LabPayError::EvidenceMissing(
            "sealed dest owner_binding empty".into(),
        ));
    }
    if evidence.burn.dest_commitment != sealed.owner_binding {
        return Err(LabPayError::DestMismatch {
            evidence: evidence_hex,
            sealed: sealed_hex,
        });
    }
    assert_dest_binding_equal(&evidence_hex, &sealed_hex, "lab_pay_dest").map_err(|e| {
        LabPayError::DestMismatch {
            evidence: evidence_hex.clone(),
            sealed: format!("{sealed_hex} ({e})"),
        }
    })?;
    if sealed.dest_display.trim().is_empty() {
        return Err(LabPayError::SoftValidateFailed("empty dest_display".into()));
    }
    if !soft_validate_dest_prefix(&sealed.dest_display) {
        return Err(LabPayError::SoftValidateFailed(sealed.dest_display.clone()));
    }
    if is_placeholder_owner_binding_hex(&sealed_hex) {
        return Err(LabPayError::PlaceholderDest(sealed_hex));
    }
    reject_placeholder_binding_hex(&sealed_hex)
        .map_err(|e| LabPayError::PlaceholderDest(e.to_string()))?;
    Ok(())
}

fn env_truthy_lab(key: &str) -> bool {
    matches!(
        std::env::var(key).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    )
}

/// Raw corridor profile string (empty if unset).
pub fn corridor_profile() -> String {
    std::env::var(ENV_CORRIDOR_PROFILE).unwrap_or_default()
}

/// Omni / product demo profiles (must not green on inventory-only).
pub fn is_product_profile() -> bool {
    let p = corridor_profile().to_ascii_lowercase();
    matches!(
        p.as_str(),
        "omni_production_shaped"
            | "omni"
            | "product"
            | "omni_e2e"
            | "production_shaped"
            | "omni-production-shaped"
    )
}

/// Explicit residual escape for inventory under product profile.
pub fn allow_lab_inventory_residual() -> bool {
    env_truthy_lab(ENV_CORRIDOR_ALLOW_LAB_INVENTORY_RESIDUAL)
}

/// Resolve Zcash release policy from env + profile defaults.
///
/// - Explicit `CORRIDOR_ZEC_RELEASE=threshold_escrow|lab_inventory` wins
/// - Product profiles default to `threshold_escrow`
/// - Else `lab_inventory` (legacy demos)
pub fn zec_release_policy() -> ZecReleasePolicy {
    match std::env::var(ENV_CORRIDOR_ZEC_RELEASE)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "threshold_escrow" | "threshold" | "escrow" | "threshold_escrow_release" => {
            ZecReleasePolicy::ThresholdEscrow
        }
        "lab_inventory" | "inventory" | "lab" => ZecReleasePolicy::LabInventory,
        "" if is_product_profile() => ZecReleasePolicy::ThresholdEscrow,
        _ if is_product_profile() => {
            // Unknown value under product → fail closed to product path.
            ZecReleasePolicy::ThresholdEscrow
        }
        _ => ZecReleasePolicy::LabInventory,
    }
}

/// Lab committee threshold (default 2-of-3).
pub fn lab_committee_params() -> (u16, u16) {
    let t = std::env::var(ENV_CORRIDOR_THRESHOLD_T)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    let n = std::env::var(ENV_CORRIDOR_THRESHOLD_N)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let t = t.max(1);
    let n = n.max(t);
    (t, n)
}

/// Metadata from the last threshold auth (live or in-process).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThresholdAuthMeta {
    pub auth_source: String,
    pub custody_label: String,
    pub committee_crypto: String,
    pub committee_pk_hex: String,
    pub frost: bool,
    pub hash_market_url: Option<String>,
}

impl Default for ThresholdAuthMeta {
    fn default() -> Self {
        Self {
            auth_source: AUTH_SOURCE_IN_PROCESS_LAB.into(),
            custody_label: "in-process-lab-committee".into(),
            committee_crypto: COMMITTEE_CRYPTO_LAB_T_OF_N.into(),
            committee_pk_hex: String::new(),
            frost: false,
            hash_market_url: None,
        }
    }
}

static LAST_THRESHOLD_AUTH_META: Mutex<Option<ThresholdAuthMeta>> = Mutex::new(None);

/// Last auth meta from [`lab_threshold_escrow_auth`] (receipt writers may attach).
pub fn take_last_threshold_auth_meta() -> Option<ThresholdAuthMeta> {
    LAST_THRESHOLD_AUTH_META
        .lock()
        .ok()
        .and_then(|mut g| g.take())
}

pub fn peek_last_threshold_auth_meta() -> Option<ThresholdAuthMeta> {
    LAST_THRESHOLD_AUTH_META.lock().ok().and_then(|g| g.clone())
}

fn store_threshold_auth_meta(meta: ThresholdAuthMeta) {
    if let Ok(mut g) = LAST_THRESHOLD_AUTH_META.lock() {
        *g = Some(meta);
    }
}

/// Resolve live hashmerchant base URL (empty if unset).
pub fn hash_market_base_url() -> Option<String> {
    for key in [ENV_CORRIDOR_HASH_MARKET_URL, ENV_HASH_MARKET_URL] {
        if let Ok(u) = std::env::var(key) {
            let t = u.trim().trim_end_matches('/').to_string();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    None
}

pub fn require_live_threshold() -> bool {
    env_truthy_lab(ENV_CORRIDOR_REQUIRE_LIVE_THRESHOLD)
}

/// Minimal HTTP POST JSON (no reqwest dep) — same style as Zakura RPC helper.
fn http_json_post(base: &str, path: &str, body: &Value) -> Result<Value, String> {
    let url = format!("{}{}", base.trim_end_matches('/'), path);
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("only http:// supported: {url}"))?;
    let (hostport, url_path) = match rest.split_once('/') {
        Some((hp, p)) => (hp, format!("/{p}")),
        None => (rest, "/".into()),
    };
    let (host, port) = if let Some((h, p)) = hostport.split_once(':') {
        (
            h.to_string(),
            p.parse::<u16>().map_err(|e| format!("port: {e}"))?,
        )
    } else {
        (hostport.to_string(), 80u16)
    };
    let body_s = body.to_string();
    let req = format!(
        "POST {url_path} HTTP/1.0\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body_s}",
        body_s.len()
    );
    let timeout = Duration::from_secs(3);
    let mut stream = TcpStream::connect_timeout(
        &format!("{host}:{port}")
            .parse()
            .map_err(|e| format!("parse addr: {e}"))?,
        timeout,
    )
    .map_err(|e| format!("connect {host}:{port}: {e}"))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    stream
        .write_all(req.as_bytes())
        .map_err(|e| e.to_string())?;
    let mut resp = String::new();
    stream
        .read_to_string(&mut resp)
        .map_err(|e| e.to_string())?;
    let http_body = resp
        .split("\r\n\r\n")
        .nth(1)
        .or_else(|| resp.split("\n\n").nth(1))
        .ok_or_else(|| "no HTTP body".to_string())?;
    // Status line
    let status_ok = resp.starts_with("HTTP/1.0 200")
        || resp.starts_with("HTTP/1.1 200")
        || resp.contains(" 200 OK");
    if !status_ok {
        return Err(format!(
            "HTTP non-200 from {path}: {}",
            resp.lines().next().unwrap_or("?")
        ));
    }
    serde_json::from_str(http_body).map_err(|e| format!("json: {e}; body={http_body}"))
}

fn http_json_get(base: &str, path: &str) -> Result<Value, String> {
    let url = format!("{}{}", base.trim_end_matches('/'), path);
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("only http:// supported: {url}"))?;
    let (hostport, url_path) = match rest.split_once('/') {
        Some((hp, p)) => (hp, format!("/{p}")),
        None => (rest, "/".into()),
    };
    let (host, port) = if let Some((h, p)) = hostport.split_once(':') {
        (
            h.to_string(),
            p.parse::<u16>().map_err(|e| format!("port: {e}"))?,
        )
    } else {
        (hostport.to_string(), 80u16)
    };
    let req =
        format!("GET {url_path} HTTP/1.0\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n");
    let timeout = Duration::from_secs(2);
    let mut stream = TcpStream::connect_timeout(
        &format!("{host}:{port}")
            .parse()
            .map_err(|e| format!("parse addr: {e}"))?,
        timeout,
    )
    .map_err(|e| format!("connect: {e}"))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    stream
        .write_all(req.as_bytes())
        .map_err(|e| e.to_string())?;
    let mut resp = String::new();
    stream
        .read_to_string(&mut resp)
        .map_err(|e| e.to_string())?;
    let http_body = resp
        .split("\r\n\r\n")
        .nth(1)
        .or_else(|| resp.split("\n\n").nth(1))
        .ok_or_else(|| "no HTTP body".to_string())?;
    let status_ok = resp.starts_with("HTTP/1.0 200")
        || resp.starts_with("HTTP/1.1 200")
        || resp.contains(" 200 OK");
    if !status_ok {
        return Err(format!(
            "HTTP non-200: {}",
            resp.lines().next().unwrap_or("?")
        ));
    }
    serde_json::from_str(http_body).map_err(|e| format!("json: {e}"))
}

/// Probe live hashmerchant custody (**FROST preferred**, then threshold).
pub fn live_hashmerchant_custody_ready() -> bool {
    let Some(base) = hash_market_base_url() else {
        return false;
    };
    for path in ["/custody", "/ve/custody"] {
        if let Ok(v) = http_json_get(&base, path) {
            let frost = v.get("frost").and_then(|x| x.as_bool()).unwrap_or(false)
                || v.get("label")
                    .and_then(|x| x.as_str())
                    .map(|s| s.contains("frost"))
                    .unwrap_or(false)
                || v.get("committee_crypto")
                    .and_then(|x| x.as_str())
                    .map(|s| s.contains("frost"))
                    .unwrap_or(false);
            let threshold = v
                .get("threshold_committee")
                .and_then(|x| x.as_bool())
                .unwrap_or(false)
                || v.get("label")
                    .and_then(|x| x.as_str())
                    .map(|s| s.contains("threshold"))
                    .unwrap_or(false);
            if frost || threshold {
                return true;
            }
        }
    }
    false
}

/// Obtain escrow-release auth from **live** hashmerchant process API.
pub fn live_threshold_escrow_auth(
    evidence: &EgressBurnEvidenceV0,
) -> Result<(ThresholdEscrowAuth, String, ThresholdAuthMeta), LabPayError> {
    let base = hash_market_base_url().ok_or_else(|| {
        LabPayError::ThresholdAuth("HASH_MARKET_URL / CORRIDOR_HASH_MARKET_URL unset".into())
    })?;
    let body = json!({
        "burn_nullifier_hex": hex::encode(evidence.burn.nullifier),
        "dest_commitment_hex": hex::encode(evidence.burn.dest_commitment),
        "amount_zat": evidence.burn.value,
        "asset_id_hex": hex::encode(evidence.burn.asset_id),
    });
    let mut last_err = String::new();
    let mut resp = None;
    for path in ["/escrow-release/sign", "/ve/escrow-release/sign"] {
        match http_json_post(&base, path, &body) {
            Ok(v) => {
                resp = Some(v);
                break;
            }
            Err(e) => last_err = e,
        }
    }
    let v = resp.ok_or_else(|| {
        LabPayError::ThresholdAuth(format!(
            "live hashmerchant escrow-release/sign failed: {last_err}"
        ))
    })?;
    let sig_hex = v
        .get("signature_hex")
        .and_then(|x| x.as_str())
        .ok_or_else(|| LabPayError::ThresholdAuth("response missing signature_hex".into()))?;
    let pk_hex = v
        .get("public_key_hex")
        .and_then(|x| x.as_str())
        .ok_or_else(|| LabPayError::ThresholdAuth("response missing public_key_hex".into()))?;
    let combined_sig = hex::decode(sig_hex.trim().trim_start_matches("0x"))
        .map_err(|e| LabPayError::ThresholdAuth(format!("signature_hex decode: {e}")))?;
    let committee_pk = hex::decode(pk_hex.trim().trim_start_matches("0x"))
        .map_err(|e| LabPayError::ThresholdAuth(format!("public_key_hex decode: {e}")))?;
    let frost_pk = frost_escrow::is_frost_public_key(&committee_pk);
    let tc_pk = threshold_committee::is_committee_public_key(&committee_pk);
    if !frost_pk && !tc_pk {
        return Err(LabPayError::ThresholdAuth(
            "live public_key is neither FE01 (FROST) nor TC01 (multi-sig committee)".into(),
        ));
    }
    let custody_label = v
        .get("custody_label")
        .and_then(|x| x.as_str())
        .unwrap_or(if frost_pk {
            "frost-escrow-ed25519-lab"
        } else {
            "threshold-committee-secp256k1"
        })
        .to_string();
    let committee_crypto = v
        .get("committee_crypto")
        .and_then(|x| x.as_str())
        .unwrap_or(if frost_pk {
            COMMITTEE_CRYPTO_FROST_LAB
        } else {
            COMMITTEE_CRYPTO_LAB_T_OF_N
        })
        .to_string();
    let frost = frost_pk
        || v.get("frost").and_then(|x| x.as_bool()).unwrap_or(false)
        || committee_crypto.contains("frost");
    // Verify locally before trusting wire (Object A digest).
    let digest = threshold_committee::escrow_release_digest(
        &evidence.burn.nullifier,
        &evidence.burn.dest_commitment,
        evidence.burn.value.into(),
        &evidence.burn.asset_id,
    );
    if frost_pk {
        frost_escrow::verify_encoded(&committee_pk, &digest, &combined_sig).map_err(|e| {
            LabPayError::ThresholdAuth(format!("live FROST sig verify failed: {e}"))
        })?;
    } else {
        threshold_committee::verify_encoded(&committee_pk, &digest, &combined_sig).map_err(
            |e| LabPayError::ThresholdAuth(format!("live multi-sig verify failed: {e}")),
        )?;
    }
    let meta = ThresholdAuthMeta {
        auth_source: AUTH_SOURCE_HASHMERCHANT_LIVE.into(),
        custody_label: custody_label.clone(),
        committee_crypto: committee_crypto.clone(),
        committee_pk_hex: hex::encode(&committee_pk),
        frost,
        hash_market_url: Some(base),
    };
    let label = format!(
        "{committee_crypto};auth_source={AUTH_SOURCE_HASHMERCHANT_LIVE};custody={custody_label};escrow_sign_path={ESCROW_SIGN_PATH_P_LAB};frost={frost}"
    );
    Ok((
        ThresholdEscrowAuth {
            committee_pk,
            combined_sig,
        },
        label,
        meta,
    ))
}

/// Prefer FROST in-process when product custody is frost (priority path).
pub fn prefer_frost_custody() -> bool {
    let backend = std::env::var("CORRIDOR_CUSTODY_BACKEND")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if backend.contains("frost") {
        return true;
    }
    if env_truthy_lab("CORRIDOR_FROST") {
        return true;
    }
    std::env::var("CORRIDOR_COMMITTEE_CRYPTO")
        .map(|s| s.contains("frost"))
        .unwrap_or(false)
}

/// In-process **FROST** lab committee (same Object A digests as FrostEscrowCustody).
pub fn in_process_frost_escrow_auth(
    evidence: &EgressBurnEvidenceV0,
) -> Result<(ThresholdEscrowAuth, String, ThresholdAuthMeta), LabPayError> {
    let (t, n) = lab_committee_params();
    let committee = frost_escrow::FrostLabCommittee::dkg(t, n)
        .map_err(|e| LabPayError::ThresholdAuth(format!("FROST DKG t={t} n={n}: {e}")))?;
    let digest = threshold_committee::escrow_release_digest(
        &evidence.burn.nullifier,
        &evidence.burn.dest_commitment,
        evidence.burn.value.into(),
        &evidence.burn.asset_id,
    );
    let sig = committee
        .sign_message(&digest)
        .map_err(|e| LabPayError::ThresholdAuth(format!("FROST sign: {e}")))?;
    let committee_pk = committee.encoded_public.clone();
    let meta = ThresholdAuthMeta {
        auth_source: AUTH_SOURCE_IN_PROCESS_LAB.into(),
        custody_label: format!("in-process-frost-lab;t={t};n={n}"),
        committee_crypto: COMMITTEE_CRYPTO_FROST_LAB.into(),
        committee_pk_hex: hex::encode(&committee_pk),
        frost: true,
        hash_market_url: None,
    };
    let label = format!(
        "{COMMITTEE_CRYPTO_FROST_LAB};t={t};n={n};auth_source={AUTH_SOURCE_IN_PROCESS_LAB};escrow_sign_path={ESCROW_SIGN_PATH_P_LAB};frost=true"
    );
    Ok((
        ThresholdEscrowAuth {
            committee_pk,
            combined_sig: sig,
        },
        label,
        meta,
    ))
}

/// In-process lab multi-sig committee (TC01 — residual, not FROST).
pub fn in_process_threshold_escrow_auth(
    evidence: &EgressBurnEvidenceV0,
) -> Result<(ThresholdEscrowAuth, String, ThresholdAuthMeta), LabPayError> {
    let (t, n) = lab_committee_params();
    let committee = LabCommittee::generate(t, n)
        .map_err(|e| LabPayError::ThresholdAuth(format!("lab DKG t={t} n={n}: {e}")))?;
    let digest = threshold_committee::escrow_release_digest(
        &evidence.burn.nullifier,
        &evidence.burn.dest_commitment,
        evidence.burn.value.into(),
        &evidence.burn.asset_id,
    );
    let combined = lab_sign_with_threshold(&committee, &digest)
        .map_err(|e| LabPayError::ThresholdAuth(format!("combine partials: {e}")))?;
    let committee_pk = committee.encoded_public.clone();
    let meta = ThresholdAuthMeta {
        auth_source: AUTH_SOURCE_IN_PROCESS_LAB.into(),
        custody_label: format!("in-process-lab-committee;t={t};n={n}"),
        committee_crypto: COMMITTEE_CRYPTO_LAB_T_OF_N.into(),
        committee_pk_hex: hex::encode(&committee_pk),
        frost: false,
        hash_market_url: None,
    };
    let label = format!(
        "{COMMITTEE_CRYPTO_LAB_T_OF_N};t={t};n={n};auth_source={AUTH_SOURCE_IN_PROCESS_LAB};escrow_sign_path={ESCROW_SIGN_PATH_P_LAB}"
    );
    Ok((
        ThresholdEscrowAuth {
            committee_pk,
            combined_sig: combined,
        },
        label,
        meta,
    ))
}

/// Sign escrow-release auth — prefer **live hashmerchant FROST**, else in-process.
///
/// Priority: live FE01/TC01 → in-process FROST (if prefer_frost) → in-process multi-sig.
/// When `CORRIDOR_REQUIRE_LIVE_THRESHOLD=1`, refuses pure in-process fallback.
pub fn lab_threshold_escrow_auth(
    evidence: &EgressBurnEvidenceV0,
) -> Result<(ThresholdEscrowAuth, String), LabPayError> {
    // 1) Live process/API (FROST or multi-sig on hashmerchant).
    if hash_market_base_url().is_some() {
        match live_threshold_escrow_auth(evidence) {
            Ok((auth, label, meta)) => {
                store_threshold_auth_meta(meta);
                return Ok((auth, label));
            }
            Err(e) if require_live_threshold() => {
                return Err(LabPayError::ThresholdAuth(format!(
                    "live custody required ({ENV_CORRIDOR_REQUIRE_LIVE_THRESHOLD}=1): {e}"
                )));
            }
            Err(_e) => {
                // fall through to in-process residual
            }
        }
    } else if require_live_threshold() {
        return Err(LabPayError::ThresholdAuth(format!(
            "live custody required but {ENV_HASH_MARKET_URL}/{ENV_CORRIDOR_HASH_MARKET_URL} unset"
        )));
    }

    // 2) In-process: FROST priority when product asks for frost custody.
    let (auth, label, meta) = if prefer_frost_custody() {
        in_process_frost_escrow_auth(evidence)?
    } else {
        in_process_threshold_escrow_auth(evidence)?
    };
    store_threshold_auth_meta(meta);
    Ok((auth, label))
}

/// Pure gate only: burn + dest + threshold → product mode receipt (no send).
pub fn authorize_threshold_escrow_for_evidence(
    evidence: &EgressBurnEvidenceV0,
    sealed: &SealedDestV0,
    auth: &ThresholdEscrowAuth,
) -> Result<ZecEgressReceiptV0, LabPayError> {
    authorize_escrow_release(evidence, &sealed.owner_binding, auth).map_err(|e| match e {
        EscrowReleaseError::MissingBurn => {
            LabPayError::EvidenceMissing("authorize_escrow_release: missing burn".into())
        }
        EscrowReleaseError::DestMismatch => LabPayError::DestMismatch {
            evidence: hex32(&evidence.burn.dest_commitment),
            sealed: sealed.owner_binding_hex.clone(),
        },
        EscrowReleaseError::BadThresholdAuth => {
            LabPayError::ThresholdAuth("authorize_escrow_release: bad threshold auth".into())
        }
        EscrowReleaseError::Schema => LabPayError::Pure("authorize_escrow_release: schema".into()),
    })
}

/// Product green check on a pay mode (after release).
///
/// Returns `Err` when product profile would claim green on funder-only inventory.
pub fn assert_product_release_mode(mode: &str) -> Result<(), LabPayError> {
    if !is_product_profile() {
        return Ok(());
    }
    if reject_funder_only_as_product(mode) {
        if allow_lab_inventory_residual() {
            return Ok(());
        }
        return Err(LabPayError::ProductRejectFunderOnly(format!(
            "mode={mode}; set {ENV_CORRIDOR_ZEC_RELEASE}=threshold_escrow or \
             {ENV_CORRIDOR_ALLOW_LAB_INVENTORY_RESIDUAL}=1 (product green FAILS on residual)"
        )));
    }
    Ok(())
}

fn synthetic_committee_txid(prefix: &str, nullifier: &[u8; 32], amount_zat: u64) -> String {
    let mut h = Sha256::new();
    h.update(prefix.as_bytes());
    h.update(b"|");
    h.update(nullifier);
    h.update(amount_zat.to_le_bytes());
    let dig = h.finalize();
    format!("{prefix}:{}", hex::encode(&dig[..16]))
}

fn synthetic_threshold_txid(nullifier: &[u8; 32], amount_zat: u64) -> String {
    synthetic_committee_txid(THRESHOLD_SIMULATED_TXID_PREFIX, nullifier, amount_zat)
}

/// Obtain Object B FROST spend signature (live hashmerchant preferred).
fn frost_object_b_spend_auth(
    evidence: &EgressBurnEvidenceV0,
    sealed: &SealedDestV0,
    object_a_sig: &[u8],
    committee_pk: &[u8],
) -> Result<(frost_escrow::ZcashSpendPackageV0, Vec<u8>), LabPayError> {
    let escrow_addr = std::env::var("CORRIDOR_FROST_ESCROW_ADDR")
        .or_else(|_| std::env::var("CORRIDOR_ZEC_ESCROW_ADDR"))
        .unwrap_or_else(|_| "frost-escrow".into());
    let pkg = frost_escrow::spend_package_from_release(
        escrow_addr,
        sealed.dest_display.clone(),
        sealed.owner_binding,
        evidence.burn.value,
        0,
        evidence.burn.nullifier,
        evidence.burn.asset_id,
        hex::encode(object_a_sig),
    );
    // SILENT ERROR
    pkg.validate_matches_burn(
        &evidence.burn.nullifier,
        &sealed.owner_binding,
        evidence.burn.value.into(),
        &evidence.burn.asset_id,
    )
    .map_err(|e| LabPayError::ThresholdAuth(format!("Object B package: {e}")))?;

    // Live API
    if let Some(base) = hash_market_base_url() {
        let body = json!({
            "escrow_addr": pkg.escrow_addr,
            "dest_display": pkg.dest_display,
            "dest_commitment_hex": hex::encode(pkg.dest_commitment),
            "amount_zat": pkg.amount_zat,
            "fee_zat": pkg.fee_zat,
            "burn_nullifier_hex": hex::encode(pkg.burn_nullifier),
            "asset_id_hex": hex::encode(pkg.asset_id),
            "object_a_sig_hex": pkg.object_a_sig_hex,
        });
        for path in ["/escrow-spend/sign", "/ve/escrow-spend/sign"] {
            if let Ok(v) = http_json_post(&base, path, &body) {
                let sig_hex = v
                    .get("signature_hex")
                    .and_then(|x| x.as_str())
                    .ok_or_else(|| {
                        LabPayError::ThresholdAuth("Object B missing signature".into())
                    })?;
                let sig = hex::decode(sig_hex.trim().trim_start_matches("0x"))
                    .map_err(|e| LabPayError::ThresholdAuth(format!("Object B sig hex: {e}")))?;
                frost_escrow::verify_spend_encoded(committee_pk, &pkg, &sig).map_err(|e| {
                    LabPayError::ThresholdAuth(format!("Object B live verify: {e}"))
                })?;
                return Ok((pkg, sig));
            }
        }
        if require_live_threshold() {
            return Err(LabPayError::ThresholdAuth(
                "Object B live /escrow-spend/sign required but failed".into(),
            ));
        }
    }

    // In-process FROST Object B residual: product e2e must use live FE01 custody for A+B.
    if require_live_threshold() {
        return Err(LabPayError::ThresholdAuth(
            "Object B requires live FROST hashmerchant /escrow-spend/sign (same custody as Object A)"
                .into(),
        ));
    }
    let (t, n) = lab_committee_params();
    let committee = frost_escrow::FrostLabCommittee::dkg(t, n)
        .map_err(|e| LabPayError::ThresholdAuth(format!("Object B in-process DKG: {e}")))?;
    // Offline residual: one in-process committee signs Object B (demo of spend crypto).
    let sig = committee
        .sign_spend_package(&pkg)
        .map_err(|e| LabPayError::ThresholdAuth(format!("Object B in-process sign: {e}")))?;
    frost_escrow::verify_spend_encoded(&committee.encoded_public, &pkg, &sig)
        .map_err(|e| LabPayError::ThresholdAuth(format!("Object B verify: {e}")))?;
    let _ = committee_pk; // live FE01 only when /escrow-spend/sign used above
    Ok((pkg, sig))
}

/// Product path: committee/FROST Object A, then FROST Object B spend, then remote ZEC send.
fn lab_pay_threshold_escrow(
    evidence: &EgressBurnEvidenceV0,
    sealed: &SealedDestV0,
    cfg: &ZakuraLocalConfig,
) -> Result<ZecEgressReceiptV0, LabPayError> {
    validate_lab_pay_gates(evidence, sealed)?;

    let (auth, _committee_label) = lab_threshold_escrow_auth(evidence)?;
    // Pure product gate — refuse missing burn / wrong dest / bad sig (Object A).
    let gated = authorize_threshold_escrow_for_evidence(evidence, sealed, &auth)?;
    let frost = frost_escrow::is_frost_public_key(&auth.committee_pk)
        || gated.mode == MODE_FROST_ESCROW_RELEASE;

    let amount_zat = evidence.burn.value;
    let force_sim = env_truthy_lab("CORRIDOR_LAB_ZEC_FORCE_SIMULATED");
    let bal_before = if rpc_ready(cfg) {
        try_getbalance_zat(cfg).ok().flatten()
    } else {
        None
    };

    // ── FROST path: Object B remote spend is required for product bridge-to-ZEC ──
    if frost {
        let (pkg, object_b_sig) =
            frost_object_b_spend_auth(evidence, sealed, &auth.combined_sig, &auth.committee_pk)?;
        // Record Object B side meta
        write_object_b_side_meta(evidence, sealed, &pkg, &object_b_sig, &auth.committee_pk)?;

        let mode_live = frost_escrow::MODE_FROST_ESCROW_SPEND;
        let mode_sim = frost_escrow::MODE_FROST_ESCROW_SPEND_SIMULATED;
        let sim_prefix = FROST_SPEND_SIMULATED_TXID_PREFIX;

        let receipt = if force_sim || !rpc_ready(cfg) {
            let txid = synthetic_committee_txid(sim_prefix, &evidence.burn.nullifier, amount_zat);
            build_receipt(evidence, sealed, Some(txid), mode_sim)?
        } else {
            let _ = validate_address(cfg, &sealed.dest_display);
            match attempt_lab_send(cfg, evidence, sealed, amount_zat) {
                Ok(txid)
                    if is_real_zec_txid(&txid)
                        && !txid.starts_with(FROST_SPEND_SIMULATED_TXID_PREFIX)
                        && !txid.starts_with(FROST_SIMULATED_TXID_PREFIX)
                        && !txid.starts_with(LAB_SIMULATED_TXID_PREFIX) =>
                {
                    // Remote ZEC landed after Object B FROST authorization.
                    build_receipt(evidence, sealed, Some(txid), mode_live)?
                }
                Ok(_) | Err(_) => {
                    let txid =
                        synthetic_committee_txid(sim_prefix, &evidence.burn.nullifier, amount_zat);
                    build_receipt(evidence, sealed, Some(txid), mode_sim)?
                }
            }
        };
        let bal_after = if rpc_ready(cfg) {
            try_getbalance_zat(cfg).ok().flatten()
        } else {
            None
        };
        write_threshold_release_side_meta(evidence, &receipt, bal_before, bal_after)?;
        return Ok(receipt);
    }

    // ── Multi-sig residual (Object A only) ──
    let mode_live = MODE_THRESHOLD_ESCROW_RELEASE;
    let mode_sim = MODE_THRESHOLD_ESCROW_RELEASE_SIMULATED;
    let sim_prefix = THRESHOLD_SIMULATED_TXID_PREFIX;
    debug_assert!(!reject_funder_only_as_product(&gated.mode));

    let receipt = if force_sim || !rpc_ready(cfg) {
        let txid = synthetic_committee_txid(sim_prefix, &evidence.burn.nullifier, amount_zat);
        build_receipt(evidence, sealed, Some(txid), mode_sim)?
    } else {
        let _ = validate_address(cfg, &sealed.dest_display);
        match attempt_lab_send(cfg, evidence, sealed, amount_zat) {
            Ok(txid)
                if is_real_zec_txid(&txid)
                    && !txid.starts_with(THRESHOLD_SIMULATED_TXID_PREFIX)
                    && !txid.starts_with(LAB_SIMULATED_TXID_PREFIX) =>
            {
                build_receipt(evidence, sealed, Some(txid), mode_live)?
            }
            Ok(txid) if txid.starts_with(LAB_SIMULATED_TXID_PREFIX) => {
                let txid =
                    synthetic_committee_txid(sim_prefix, &evidence.burn.nullifier, amount_zat);
                build_receipt(evidence, sealed, Some(txid), mode_sim)?
            }
            Ok(txid) => {
                if is_real_zec_txid(&txid) {
                    build_receipt(evidence, sealed, Some(txid), mode_live)?
                } else {
                    let sim =
                        synthetic_committee_txid(sim_prefix, &evidence.burn.nullifier, amount_zat);
                    build_receipt(evidence, sealed, Some(sim), mode_sim)?
                }
            }
            Err(_) => {
                let txid =
                    synthetic_committee_txid(sim_prefix, &evidence.burn.nullifier, amount_zat);
                build_receipt(evidence, sealed, Some(txid), mode_sim)?
            }
        }
    };

    let bal_after = if rpc_ready(cfg) {
        try_getbalance_zat(cfg).ok().flatten()
    } else {
        None
    };
    write_threshold_release_side_meta(evidence, &receipt, bal_before, bal_after)?;
    Ok(receipt)
}

fn write_object_b_side_meta(
    evidence: &EgressBurnEvidenceV0,
    sealed: &SealedDestV0,
    pkg: &frost_escrow::ZcashSpendPackageV0,
    object_b_sig: &[u8],
    committee_pk: &[u8],
) -> Result<(), LabPayError> {
    let path = std::env::var("CORRIDOR_FROST_SPEND_META_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let base = zec_egress_receipt_path();
            base.with_file_name("10b-frost-spend-object-b.json")
        });
    let doc = json!({
        "object": "B",
        "domain": String::from_utf8_lossy(frost_escrow::DOMAIN_ZEC_SPEND_V0),
        "escrow_sign_path": frost_escrow::ESCROW_SIGN_PATH_FROST_SPEND,
        "committee_crypto": COMMITTEE_CRYPTO_FROST_LAB,
        "committee_pk_hex": hex::encode(committee_pk),
        "object_b_sig_hex": hex::encode(object_b_sig),
        "sighash_hex": hex::encode(frost_escrow::spend_sighash(pkg)),
        "escrow_addr": pkg.escrow_addr,
        "dest_display": sealed.dest_display,
        "dest_commitment_hex": hex::encode(sealed.owner_binding),
        "amount_zat": evidence.burn.value,
        "burn_nullifier_hex": hex::encode(evidence.burn.nullifier),
        "mode_live": frost_escrow::MODE_FROST_ESCROW_SPEND,
        "mode_sim": frost_escrow::MODE_FROST_ESCROW_SPEND_SIMULATED,
        "note": "Object B FROST authorizes remote ZEC spend; wallet broadcast is separate residual until consensus-native FROST",
    });
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&path, serde_json::to_vec_pretty(&doc).unwrap_or_default())
        .map_err(|e| LabPayError::Io(format!("write Object B meta: {e}")))?;
    Ok(())
}

/// Side JSON next to ZEC receipt: auth_source + optional escrow balance Δ.
fn write_threshold_release_side_meta(
    evidence: &EgressBurnEvidenceV0,
    receipt: &ZecEgressReceiptV0,
    bal_before: Option<u64>,
    bal_after: Option<u64>,
) -> Result<(), LabPayError> {
    let meta = peek_last_threshold_auth_meta().unwrap_or_default();
    let path = std::env::var("CORRIDOR_THRESHOLD_AUTH_META_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let base = zec_egress_receipt_path();
            base.with_file_name("10-threshold-auth-meta.json")
        });
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| LabPayError::Io(e.to_string()))?;
    }
    let decreased = match (bal_before, bal_after) {
        (Some(b), Some(a)) => {
            Some(a < b || (b >= receipt.amount_zat && a + receipt.amount_zat <= b + 1))
        }
        _ => None,
    };
    let v = json!({
        "auth_source": meta.auth_source,
        "custody_label": meta.custody_label,
        "committee_crypto": meta.committee_crypto,
        "committee_pk_hex": meta.committee_pk_hex,
        "frost": false,
        "escrow_sign_path": ESCROW_SIGN_PATH_P_LAB,
        "hash_market_url": meta.hash_market_url,
        "mode": receipt.mode,
        "amount_zat": receipt.amount_zat,
        "burn_nullifier_hex": hex32(&evidence.burn.nullifier),
        "escrow_balance_before_zat": bal_before,
        "escrow_balance_after_zat": bal_after,
        "escrow_balance_decreased": decreased,
        "note": if meta.auth_source == AUTH_SOURCE_HASHMERCHANT_LIVE {
            "release sig from live hashmerchant ThresholdCommitteeCustody API"
        } else {
            "release sig from in-process lab residual (live server not required for this run)"
        },
    });
    let s = serde_json::to_string_pretty(&v).map_err(|e| LabPayError::Json(e.to_string()))?;
    fs::write(&path, s).map_err(|e| LabPayError::Io(e.to_string()))?;
    Ok(())
}

/// Lab residual inventory path (pre-threshold product).
fn lab_pay_inventory(
    evidence: &EgressBurnEvidenceV0,
    sealed: &SealedDestV0,
    cfg: &ZakuraLocalConfig,
) -> Result<ZecEgressReceiptV0, LabPayError> {
    // Product profile must not green on inventory without explicit residual flag.
    if is_product_profile() && !allow_lab_inventory_residual() {
        return Err(LabPayError::ProductRejectFunderOnly(format!(
            "profile={} refuses lab_inventory; set {ENV_CORRIDOR_ZEC_RELEASE}=threshold_escrow \
             or {ENV_CORRIDOR_ALLOW_LAB_INVENTORY_RESIDUAL}=1",
            corridor_profile()
        )));
    }

    let amount_zat = evidence.burn.value;
    let force_sim = env_truthy_lab("CORRIDOR_LAB_ZEC_FORCE_SIMULATED");

    if force_sim || !rpc_ready(cfg) {
        let txid = synthetic_lab_txid(&evidence.burn.nullifier, amount_zat);
        return build_receipt(
            evidence,
            sealed,
            Some(txid),
            MODE_LAB_INVENTORY_PAY_SIMULATED,
        );
    }

    // Soft-validate dest on chain when possible (does not replace burn gate).
    let _ = validate_address(cfg, &sealed.dest_display);

    // Prefer probing wallet; if getbalance missing, still attempt send once
    // (some residual stacks expose send without getbalance).
    let wallet = wallet_rpc_available(cfg);
    if !wallet {
        // One-shot send attempt still allowed — success upgrades to live mode.
        let send_result = attempt_lab_send(cfg, evidence, sealed, amount_zat);
        return match send_result {
            Ok(txid) if is_real_zec_txid(&txid) || !txid.starts_with(LAB_SIMULATED_TXID_PREFIX) => {
                build_receipt(evidence, sealed, Some(txid), MODE_LAB_INVENTORY_PAY)
            }
            Ok(_) | Err(_) => {
                let txid = synthetic_lab_txid(&evidence.burn.nullifier, amount_zat);
                build_receipt(
                    evidence,
                    sealed,
                    Some(txid),
                    MODE_LAB_INVENTORY_PAY_SIMULATED,
                )
            }
        };
    }

    // Wallet present: optional fundedness soft-check (do not refuse — residual
    // may fund via miner; send failure still degrades simulated).
    let _ = try_getbalance_zat(cfg);

    let send_result = attempt_lab_send(cfg, evidence, sealed, amount_zat);
    match send_result {
        Ok(txid) => {
            // Refuse to label simulated-looking strings as live.
            if txid.starts_with(LAB_SIMULATED_TXID_PREFIX) {
                build_receipt(
                    evidence,
                    sealed,
                    Some(txid),
                    MODE_LAB_INVENTORY_PAY_SIMULATED,
                )
            } else {
                build_receipt(evidence, sealed, Some(txid), MODE_LAB_INVENTORY_PAY)
            }
        }
        Err(_e) => {
            let txid = synthetic_lab_txid(&evidence.burn.nullifier, amount_zat);
            build_receipt(
                evidence,
                sealed,
                Some(txid),
                MODE_LAB_INVENTORY_PAY_SIMULATED,
            )
        }
    }
}

/// True if RPC error text indicates method not implemented / not found.
pub fn rpc_err_method_missing(err: &str) -> bool {
    let e = err.to_lowercase();
    e.contains("method not found")
        || e.contains("unknown method")
        || e.contains("method not available")
        || e.contains("not implemented")
        || e.contains("\"code\":-32601")
        || e.contains("code\": -32601")
        || e.contains("code\":-32601")
        || e.contains("-32601")
}

/// Probe zcashd-compat wallet surface: `getbalance` succeeds or fails for a
/// non-missing-method reason (auth, unlocked wallet, etc.).
///
/// Zakura core without wallet residual → false → lab pay stays simulated.
pub fn wallet_rpc_available(cfg: &ZakuraLocalConfig) -> bool {
    if !rpc_ready(cfg) {
        return false;
    }
    match json_rpc_call(cfg, "getbalance", json!([])) {
        Ok(_) => true,
        Err(e) => {
            let s = e.to_string();
            let low = s.to_lowercase();
            if low.contains("rpc down") || low.contains("io:") || low.contains("timed out") {
                return false;
            }
            // Method exists but e.g. wallet locked / empty — still "available".
            !rpc_err_method_missing(&s)
        }
    }
}

/// Optional inventory balance in zatoshis when `getbalance` works.
pub fn try_getbalance_zat(cfg: &ZakuraLocalConfig) -> Result<Option<u64>, ZakuraLocalError> {
    let r = json_rpc_call(cfg, "getbalance", json!([]))?;
    Ok(zec_amount_to_zat(&r))
}

/// Optional transparent received total via `getreceivedbyaddress` (zcashd-compat).
pub fn try_getreceivedbyaddress_zat(
    cfg: &ZakuraLocalConfig,
    addr: &str,
) -> Result<Option<u64>, ZakuraLocalError> {
    // minconf=0 so lab unconfirmed inventory still surfaces.
    let r = json_rpc_call(cfg, "getreceivedbyaddress", json!([addr, 0]))?;
    Ok(zec_amount_to_zat(&r))
}

fn zec_amount_to_zat(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                if f < 0.0 {
                    return None;
                }
                Some((f * 100_000_000.0).round() as u64)
            } else if let Some(i) = n.as_u64() {
                // Some nodes return sat/zat integer — treat large ints as zat.
                Some(i)
            } else if let Some(i) = n.as_i64() {
                if i < 0 { None } else { Some(i as u64) }
            } else {
                None
            }
        }
        Value::String(s) => s
            .parse::<f64>()
            .ok()
            .map(|f| (f * 100_000_000.0).round() as u64),
        _ => None,
    }
}

/// Attempt transparent `sendtoaddress` (amount in ZEC from zatoshis).
fn try_sendtoaddress(
    cfg: &ZakuraLocalConfig,
    dest: &str,
    amount_zat: u64,
) -> Result<String, ZakuraLocalError> {
    let zec = amount_zat as f64 / 100_000_000.0;
    let r = json_rpc_call(cfg, "sendtoaddress", json!([dest, zec]))?;
    match r {
        Value::String(txid) if !txid.is_empty() => Ok(txid),
        other => Err(ZakuraLocalError::Rpc(format!(
            "sendtoaddress unexpected result: {other}"
        ))),
    }
}

/// Attempt shielded `z_sendmany` (zcashd-compat residual; may be unavailable).
fn try_z_sendmany(
    cfg: &ZakuraLocalConfig,
    dest: &str,
    amount_zat: u64,
) -> Result<String, ZakuraLocalError> {
    let zec = amount_zat as f64 / 100_000_000.0;
    let params = json!(["", [{ "address": dest, "amount": zec }]]);
    let r = json_rpc_call(cfg, "z_sendmany", params)?;
    match r {
        Value::String(opid_or_txid) if !opid_or_txid.is_empty() => Ok(opid_or_txid),
        other => Err(ZakuraLocalError::Rpc(format!(
            "z_sendmany unexpected result: {other}"
        ))),
    }
}

/// True if a string looks like a real chain txid (64 hex), not lab synthetic.
pub fn is_real_zec_txid(txid: &str) -> bool {
    let t = txid.trim();
    if t.is_empty()
        || t.starts_with(LAB_SIMULATED_TXID_PREFIX)
        || t.starts_with(THRESHOLD_SIMULATED_TXID_PREFIX)
        || t.starts_with(FROST_SIMULATED_TXID_PREFIX)
        || t.starts_with(FROST_SPEND_SIMULATED_TXID_PREFIX)
        || t.starts_with(MODE_THRESHOLD_ESCROW_RELEASE)
        || t.starts_with(MODE_FROST_ESCROW_RELEASE)
        || t.starts_with(frost_escrow::MODE_FROST_ESCROW_SPEND)
    {
        return false;
    }
    // zcashd/zebra style: 64 hex chars; opids from z_sendmany may be opid-…
    if t.len() == 64 && t.chars().all(|c| c.is_ascii_hexdigit()) {
        return true;
    }
    t.starts_with("opid-") || t.starts_with("opid:")
}

fn build_receipt(
    evidence: &EgressBurnEvidenceV0,
    sealed: &SealedDestV0,
    zec_txid: Option<String>,
    mode: &str,
) -> Result<ZecEgressReceiptV0, LabPayError> {
    lab_receipt_after_burn(evidence, sealed.dest_display.clone(), zec_txid, mode)
        .map_err(|e| LabPayError::Pure(format!("{e:?}")))
}

/// Zcash pay gated on Terp burn evidence + G4 sealed dest.
///
/// Branches on [`zec_release_policy`]:
/// - **threshold_escrow** (product): committee auth via `authorize_escrow_release`,
///   then send when possible (P-lab). Modes `threshold_escrow_release[_simulated]`.
/// - **lab_inventory** (residual): host wallet inventory. Product profiles refuse
///   unless `CORRIDOR_ALLOW_LAB_INVENTORY_RESIDUAL=1`.
pub fn lab_pay_zec_after_burn(
    evidence: &EgressBurnEvidenceV0,
    sealed: &SealedDestV0,
) -> Result<ZecEgressReceiptV0, LabPayError> {
    lab_pay_zec_after_burn_with_cfg(evidence, sealed, &ZakuraLocalConfig::default())
}

/// Same as [`lab_pay_zec_after_burn`] with explicit RPC config (tests use closed port).
pub fn lab_pay_zec_after_burn_with_cfg(
    evidence: &EgressBurnEvidenceV0,
    sealed: &SealedDestV0,
    cfg: &ZakuraLocalConfig,
) -> Result<ZecEgressReceiptV0, LabPayError> {
    validate_lab_pay_gates(evidence, sealed)?;

    let receipt = match zec_release_policy() {
        ZecReleasePolicy::ThresholdEscrow => lab_pay_threshold_escrow(evidence, sealed, cfg)?,
        ZecReleasePolicy::LabInventory => lab_pay_inventory(evidence, sealed, cfg)?,
    };

    // Belt-and-suspenders: product green must not accept funder-only modes.
    assert_product_release_mode(&receipt.mode)?;
    Ok(receipt)
}

fn attempt_lab_send(
    cfg: &ZakuraLocalConfig,
    evidence: &EgressBurnEvidenceV0,
    sealed: &SealedDestV0,
    amount_zat: u64,
) -> Result<String, ZakuraLocalError> {
    match evidence.burn.dest_kind {
        DestKind::Transparent => try_sendtoaddress(cfg, &sealed.dest_display, amount_zat),
        DestKind::Shielded => try_z_sendmany(cfg, &sealed.dest_display, amount_zat)
            .or_else(|_| try_sendtoaddress(cfg, &sealed.dest_display, amount_zat)),
    }
}

// ── Open / confirm at sealed dest (P1) ─────────────────────────────────────

/// Result of post-pay (or pre-pay) open/confirm checks against sealed dest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenConfirmResult {
    pub dest_display: String,
    pub dest_owner_binding_hex: String,
    pub rpc_ready: bool,
    /// `validateaddress` isvalid when RPC answered.
    pub address_valid: Option<bool>,
    /// Wallet total balance in zat when `getbalance` available.
    pub wallet_balance_zat: Option<u64>,
    /// `getreceivedbyaddress` for transparent dest when available.
    pub received_zat: Option<u64>,
    /// Honest label: rpc_open_confirm | skip_clean_* | rpc_validate_only
    pub mode: String,
    pub note: String,
}

impl OpenConfirmResult {
    /// True when we got at least address validation on live RPC.
    pub fn confirmed_address(&self) -> bool {
        self.address_valid == Some(true)
    }

    /// True only when balance/received RPCs returned (not a funds claim alone).
    pub fn has_balance_probe(&self) -> bool {
        self.wallet_balance_zat.is_some() || self.received_zat.is_some()
    }
}

/// Open/confirm helper at sealed dest: `validateaddress` + optional balance.
///
/// **Skip-clean** when RPC down or wallet balance RPCs missing — never fails
/// the corridor for residual Zakura core. Soft-fails only when
/// `CORRIDOR_REQUIRE_ZAKURA_RPC=1` and RPC down (caller may ignore `Result` Err).
pub fn confirm_open_at_sealed_dest(
    sealed: &SealedDestV0,
) -> Result<OpenConfirmResult, LabPayError> {
    confirm_open_at_sealed_dest_with_cfg(sealed, &ZakuraLocalConfig::default())
}

/// Same as [`confirm_open_at_sealed_dest`] with explicit config.
pub fn confirm_open_at_sealed_dest_with_cfg(
    sealed: &SealedDestV0,
    cfg: &ZakuraLocalConfig,
) -> Result<OpenConfirmResult, LabPayError> {
    if sealed.dest_display.trim().is_empty() {
        return Err(LabPayError::SoftValidateFailed("empty dest_display".into()));
    }
    if !soft_validate_dest_prefix(&sealed.dest_display) {
        return Err(LabPayError::SoftValidateFailed(sealed.dest_display.clone()));
    }
    let require_rpc = env_truthy_lab("CORRIDOR_REQUIRE_ZAKURA_RPC");
    let ready = rpc_ready(cfg);
    if !ready {
        if require_rpc {
            return Err(LabPayError::Rpc(format!(
                "rpc required but down at {}",
                cfg.rpc_url
            )));
        }
        return Ok(OpenConfirmResult {
            dest_display: sealed.dest_display.clone(),
            dest_owner_binding_hex: sealed.owner_binding_hex.clone(),
            rpc_ready: false,
            address_valid: None,
            wallet_balance_zat: None,
            received_zat: None,
            mode: OPEN_CONFIRM_SKIP_NO_RPC.into(),
            note: format!(
                "Zakura RPC down at {} — open/confirm skip-clean",
                cfg.rpc_url
            ),
        });
    }

    let address_valid = match validate_address(cfg, &sealed.dest_display) {
        Ok(v) => Some(v),
        Err(e) => {
            // validate may be absent on broken stacks — skip-clean
            if require_rpc {
                return Err(LabPayError::Rpc(e.to_string()));
            }
            None
        }
    };

    let wallet_balance_zat = try_getbalance_zat(cfg).ok().flatten();
    let received_zat = if matches!(
        dest_kind_from_display(&sealed.dest_display),
        DestKind::Transparent
    ) {
        try_getreceivedbyaddress_zat(cfg, &sealed.dest_display)
            .ok()
            .flatten()
    } else {
        None
    };

    let (mode, note) = if wallet_balance_zat.is_some() || received_zat.is_some() {
        (
            OPEN_CONFIRM_RPC_OK.to_string(),
            format!(
                "validateaddress={:?} balance_zat={:?} received_zat={:?}",
                address_valid, wallet_balance_zat, received_zat
            ),
        )
    } else if address_valid.is_some() {
        (
            OPEN_CONFIRM_SKIP_VALIDATE_ONLY.to_string(),
            format!(
                "validateaddress={:?}; no getbalance/getreceivedbyaddress (Zakura core residual)",
                address_valid
            ),
        )
    } else {
        (
            OPEN_CONFIRM_SKIP_NO_WALLET.to_string(),
            "RPC up but open/confirm probes unavailable — skip-clean".into(),
        )
    };

    Ok(OpenConfirmResult {
        dest_display: sealed.dest_display.clone(),
        dest_owner_binding_hex: sealed.owner_binding_hex.clone(),
        rpc_ready: true,
        address_valid,
        wallet_balance_zat,
        received_zat,
        mode,
        note,
    })
}

/// Build minimal lab evidence matching a sealed dest (harness / unit helper).
///
/// Uses pure `egress_nullifier` domain via `terp_seams::dex::egress_nullifier`.
pub fn lab_evidence_for_sealed(
    sealed: &SealedDestV0,
    value: u64,
    proof_mode: impl Into<String>,
) -> EgressBurnEvidenceV0 {
    let mut cm = [0u8; 32];
    cm[0] = 0xce;
    cm[31] = 0x01;
    let mut rcm = [0u8; 32];
    rcm[0] = 0xac;
    rcm[1] = 0x01;
    let nf = terp_seams::dex::egress_nullifier(&cm, &rcm);

    let mut asset = [0u8; 32];
    asset[0] = b'Z';
    asset[1] = b'E';
    asset[2] = b'C';

    EgressBurnEvidenceV0 {
        terp_tx_hash: Some("lab_terp_burn_tx".into()),
        burn: EgressBurnPublic {
            asset_id: asset,
            value,
            cm_spent: cm,
            nullifier: nf,
            dest_commitment: sealed.owner_binding,
            dest_kind: dest_kind_from_display(&sealed.dest_display),
            root: [0x11; 32],
            source_pool_id: Some(1),
        },
        proof_mode: proof_mode.into(),
        settle_receipt_ref: Some("lab_settle_ref".into()),
    }
}

/// Write receipt as JSON artifact (pure types lack serde — host mirror).
pub fn write_zec_egress_receipt_json(
    receipt: &ZecEgressReceiptV0,
    path: &Path,
) -> Result<(), LabPayError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| LabPayError::Io(e.to_string()))?;
    }
    let meta = peek_last_threshold_auth_meta();
    let v = json!({
        "dest_display": receipt.dest_display,
        "dest_owner_binding_hex": receipt.dest_owner_binding_hex,
        "dest_kind": match receipt.dest_kind {
            DestKind::Transparent => "transparent",
            DestKind::Shielded => "shielded",
        },
        "zec_txid": receipt.zec_txid,
        "amount_zat": receipt.amount_zat,
        "mode": receipt.mode,
        "burn_nullifier_hex": receipt.burn_nullifier_hex,
        "auth_source": meta.as_ref().map(|m| m.auth_source.clone()),
        "committee_crypto": meta.as_ref().map(|m| m.committee_crypto.clone()),
        "custody_label": meta.as_ref().map(|m| m.custody_label.clone()),
        "frost": false,
    });
    let s = serde_json::to_string_pretty(&v).map_err(|e| LabPayError::Json(e.to_string()))?;
    fs::write(path, s).map_err(|e| LabPayError::Io(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::zakura_local::{
        REGTEST_MINER_DEST, REGTEST_MINER_OWNER_BINDING_HEX, SealedDestSource, seal_funded_dest,
    };
    use std::env;
    use std::time::Duration;

    fn offline_cfg() -> ZakuraLocalConfig {
        ZakuraLocalConfig {
            rpc_url: "http://127.0.0.1:1".into(),
            timeout: Duration::from_millis(150),
        }
    }

    fn golden_sealed() -> SealedDestV0 {
        SealedDestV0 {
            dest_display: REGTEST_MINER_DEST.into(),
            owner_binding: {
                let mut b = [0u8; 32];
                let raw = hex::decode(REGTEST_MINER_OWNER_BINDING_HEX).unwrap();
                b.copy_from_slice(&raw);
                b
            },
            owner_binding_hex: REGTEST_MINER_OWNER_BINDING_HEX.into(),
            source: SealedDestSource::GoldenPrimary,
            rpc_ready: false,
            rpc_validated: false,
        }
    }

    /// Serialize env-mutating tests (process env is global).
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Isolate env for product/threshold tests (restore previous values on drop).
    ///
    /// `# Safety`: test-only process env mutation; restored on drop (edition 2024).
    struct EnvGuard {
        keys: Vec<(&'static str, Option<String>)>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl EnvGuard {
        fn set(pairs: &[(&'static str, &str)]) -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let mut keys = Vec::new();
            for (k, v) in pairs {
                keys.push((*k, env::var(k).ok()));
                // # Safety: unit-test env pin only; Drop restores prior value.
                unsafe {
                    env::set_var(k, v);
                }
            }
            Self { keys, _lock: lock }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, prev) in self.keys.drain(..) {
                // # Safety: restore process env after test pin.
                unsafe {
                    match prev {
                        Some(v) => env::set_var(k, v),
                        None => env::remove_var(k),
                    }
                }
            }
        }
    }

    /// Pin legacy inventory residual so ambient omni profile env cannot flaky-fail unit tests.
    fn pin_legacy_inventory_env() -> EnvGuard {
        EnvGuard::set(&[
            (ENV_CORRIDOR_ZEC_RELEASE, "lab_inventory"),
            (ENV_CORRIDOR_PROFILE, ""),
            (ENV_CORRIDOR_ALLOW_LAB_INVENTORY_RESIDUAL, "1"),
            ("CORRIDOR_LAB_ZEC_FORCE_SIMULATED", "1"),
        ])
    }

    #[test]
    fn refuse_value_zero() {
        let _g = pin_legacy_inventory_env();
        let sealed = golden_sealed();
        let mut ev = lab_evidence_for_sealed(&sealed, 1, "mock_verify_lab");
        ev.burn.value = 0;
        let err = lab_pay_zec_after_burn_with_cfg(&ev, &sealed, &offline_cfg()).unwrap_err();
        assert!(matches!(err, LabPayError::ValueZero), "{err}");
    }

    #[test]
    fn refuse_dest_mismatch() {
        let _g = pin_legacy_inventory_env();
        let sealed = golden_sealed();
        let mut ev = lab_evidence_for_sealed(&sealed, 50_000, "mock_verify_lab");
        ev.burn.dest_commitment = [0x42; 32];
        let err = lab_pay_zec_after_burn_with_cfg(&ev, &sealed, &offline_cfg()).unwrap_err();
        assert!(matches!(err, LabPayError::DestMismatch { .. }), "{err}");
    }

    #[test]
    fn refuse_missing_proof_mode_and_zero_nullifier() {
        let _g = pin_legacy_inventory_env();
        let sealed = golden_sealed();
        let mut ev = lab_evidence_for_sealed(&sealed, 10, "mock_verify_lab");
        ev.proof_mode = String::new();
        let err = lab_pay_zec_after_burn_with_cfg(&ev, &sealed, &offline_cfg()).unwrap_err();
        assert!(matches!(err, LabPayError::EvidenceMissing(_)), "{err}");

        let mut ev2 = lab_evidence_for_sealed(&sealed, 10, "mock_verify_lab");
        ev2.burn.nullifier = [0u8; 32];
        let err2 = lab_pay_zec_after_burn_with_cfg(&ev2, &sealed, &offline_cfg()).unwrap_err();
        assert!(matches!(err2, LabPayError::EvidenceMissing(_)), "{err2}");
    }

    #[test]
    fn accept_mock_path_matching_seal() {
        let _g = pin_legacy_inventory_env();
        if env::var("ZAKURA_DEST_ADDR").is_ok() || env::var("CORRIDOR_DEST_OWNER_BINDING").is_ok() {
            eprintln!("skip accept_mock: dest env override present");
            return;
        }
        let sealed = seal_funded_dest(&offline_cfg()).expect("golden seal");
        assert_eq!(sealed.owner_binding_hex, REGTEST_MINER_OWNER_BINDING_HEX);
        let ev = lab_evidence_for_sealed(&sealed, 100_000, "mock_verify_lab");
        assert_eq!(ev.burn.dest_commitment, sealed.owner_binding);

        let receipt =
            lab_pay_zec_after_burn_with_cfg(&ev, &sealed, &offline_cfg()).expect("mock pay");
        assert_eq!(receipt.dest_display, REGTEST_MINER_DEST);
        assert_eq!(
            receipt.dest_owner_binding_hex,
            REGTEST_MINER_OWNER_BINDING_HEX
        );
        assert_eq!(receipt.amount_zat, 100_000);
        assert_eq!(receipt.mode, MODE_LAB_INVENTORY_PAY_SIMULATED);
        let txid = receipt.zec_txid.expect("synthetic txid");
        assert!(
            txid.starts_with(LAB_SIMULATED_TXID_PREFIX),
            "txid must be labeled simulated: {txid}"
        );
        assert_eq!(receipt.burn_nullifier_hex, hex32(&ev.burn.nullifier));
        assert_eq!(receipt.dest_kind, DestKind::Transparent);
        eprintln!(
            "lab_pay mock OK mode={} txid={} amount_zat={}",
            receipt.mode, txid, receipt.amount_zat
        );
    }

    #[test]
    fn refuse_without_evidence_helper_gates() {
        let sealed = golden_sealed();
        assert!(
            validate_lab_pay_gates(
                &EgressBurnEvidenceV0 {
                    terp_tx_hash: None,
                    burn: EgressBurnPublic {
                        asset_id: [0u8; 32],
                        value: 0,
                        cm_spent: [0u8; 32],
                        nullifier: [0u8; 32],
                        dest_commitment: sealed.owner_binding,
                        dest_kind: DestKind::Transparent,
                        root: [0u8; 32],
                        source_pool_id: None,
                    },
                    proof_mode: String::new(),
                    settle_receipt_ref: None,
                },
                &sealed
            )
            .is_err()
        );
    }

    /// Live: when RPC up, attempt send; degrade to simulated if wallet RPC missing.
    #[test]
    fn lab_pay_live_rpc_skip_clean_when_available() {
        let _g = EnvGuard::set(&[
            (ENV_CORRIDOR_ZEC_RELEASE, "lab_inventory"),
            (ENV_CORRIDOR_PROFILE, ""),
            (ENV_CORRIDOR_ALLOW_LAB_INVENTORY_RESIDUAL, "1"),
        ]);
        unsafe {
            env::remove_var("CORRIDOR_LAB_ZEC_FORCE_SIMULATED");
        }
        let cfg = ZakuraLocalConfig::default();
        if !rpc_ready(&cfg) {
            eprintln!(
                "skip live lab_pay: RPC not at {} (run zakura-local.sh up)",
                cfg.rpc_url
            );
            return;
        }
        let sealed = golden_sealed();
        let ev = lab_evidence_for_sealed(&sealed, 1, "mock_verify_lab");
        let receipt = lab_pay_zec_after_burn_with_cfg(&ev, &sealed, &cfg).expect("live or degrade");
        assert_eq!(
            receipt.dest_owner_binding_hex,
            REGTEST_MINER_OWNER_BINDING_HEX
        );
        assert!(receipt.zec_txid.is_some());
        assert!(
            receipt.mode == MODE_LAB_INVENTORY_PAY
                || receipt.mode == MODE_LAB_INVENTORY_PAY_SIMULATED
        );
        if receipt.mode == MODE_LAB_INVENTORY_PAY {
            let txid = receipt.zec_txid.as_deref().unwrap_or("");
            assert!(
                is_real_zec_txid(txid),
                "live mode must not use simulated prefix: {txid}"
            );
            assert!(!txid.starts_with(LAB_SIMULATED_TXID_PREFIX));
        } else {
            let txid = receipt.zec_txid.as_deref().unwrap_or("");
            assert!(
                txid.starts_with(LAB_SIMULATED_TXID_PREFIX),
                "simulated mode must label txid: {txid}"
            );
        }
        eprintln!(
            "lab_pay live path mode={} wallet_rpc={} txid={:?}",
            receipt.mode,
            wallet_rpc_available(&cfg),
            receipt.zec_txid
        );
    }

    #[test]
    fn open_confirm_skip_clean_when_rpc_down() {
        let sealed = golden_sealed();
        let r = confirm_open_at_sealed_dest_with_cfg(&sealed, &offline_cfg()).expect("skip-clean");
        assert!(!r.rpc_ready);
        assert_eq!(r.mode, OPEN_CONFIRM_SKIP_NO_RPC);
        assert!(r.address_valid.is_none());
        assert_eq!(r.dest_owner_binding_hex, REGTEST_MINER_OWNER_BINDING_HEX);
        eprintln!("open_confirm offline OK mode={} note={}", r.mode, r.note);
    }

    #[test]
    fn threshold_escrow_offline_product_mode() {
        let _g = EnvGuard::set(&[
            (ENV_CORRIDOR_ZEC_RELEASE, "threshold_escrow"),
            (ENV_CORRIDOR_PROFILE, "omni_production_shaped"),
            ("CORRIDOR_LAB_ZEC_FORCE_SIMULATED", "1"),
            (ENV_CORRIDOR_ALLOW_LAB_INVENTORY_RESIDUAL, ""),
            ("CORRIDOR_CUSTODY_BACKEND", "threshold"),
            ("CORRIDOR_FROST", "0"),
        ]);
        unsafe {
            env::remove_var(ENV_HASH_MARKET_URL);
            env::remove_var(ENV_CORRIDOR_HASH_MARKET_URL);
            env::remove_var(ENV_CORRIDOR_REQUIRE_LIVE_THRESHOLD);
            env::remove_var("CORRIDOR_COMMITTEE_CRYPTO");
        }

        assert_eq!(zec_release_policy(), ZecReleasePolicy::ThresholdEscrow);
        assert!(is_product_profile());
        assert!(!prefer_frost_custody());

        let sealed = golden_sealed();
        let ev = lab_evidence_for_sealed(&sealed, 42_000, "mock_verify_lab");
        let receipt =
            lab_pay_zec_after_burn_with_cfg(&ev, &sealed, &offline_cfg()).expect("threshold pay");
        assert_eq!(receipt.mode, MODE_THRESHOLD_ESCROW_RELEASE_SIMULATED);
        assert_eq!(receipt.amount_zat, 42_000);
        assert!(!reject_funder_only_as_product(&receipt.mode));
        assert_product_release_mode(&receipt.mode).expect("product green");
        let txid = receipt.zec_txid.expect("sim txid");
        assert!(
            txid.starts_with(THRESHOLD_SIMULATED_TXID_PREFIX),
            "txid={txid}"
        );
        assert!(!txid.starts_with(LAB_SIMULATED_TXID_PREFIX));
        assert_eq!(receipt.burn_nullifier_hex, hex32(&ev.burn.nullifier));
        eprintln!(
            "threshold product offline OK mode={} committee={} path={}",
            receipt.mode, COMMITTEE_CRYPTO_LAB_T_OF_N, ESCROW_SIGN_PATH_P_LAB
        );
    }

    #[test]
    fn frost_object_b_spend_offline_product_mode() {
        let _g = EnvGuard::set(&[
            (ENV_CORRIDOR_ZEC_RELEASE, "threshold_escrow"),
            (ENV_CORRIDOR_PROFILE, "omni_production_shaped"),
            ("CORRIDOR_LAB_ZEC_FORCE_SIMULATED", "1"),
            (ENV_CORRIDOR_ALLOW_LAB_INVENTORY_RESIDUAL, ""),
            ("CORRIDOR_CUSTODY_BACKEND", "frost"),
            ("CORRIDOR_FROST", "1"),
        ]);
        unsafe {
            env::remove_var(ENV_HASH_MARKET_URL);
            env::remove_var(ENV_CORRIDOR_HASH_MARKET_URL);
            env::remove_var(ENV_CORRIDOR_REQUIRE_LIVE_THRESHOLD);
        }
        assert!(prefer_frost_custody());
        let sealed = golden_sealed();
        let ev = lab_evidence_for_sealed(&sealed, 12_000, "mock_verify_lab");
        let receipt =
            lab_pay_zec_after_burn_with_cfg(&ev, &sealed, &offline_cfg()).expect("frost spend");
        assert_eq!(
            receipt.mode,
            frost_escrow::MODE_FROST_ESCROW_SPEND_SIMULATED
        );
        assert_product_release_mode(&receipt.mode).unwrap();
        let txid = receipt.zec_txid.expect("txid");
        assert!(
            txid.starts_with(FROST_SPEND_SIMULATED_TXID_PREFIX),
            "txid={txid}"
        );
        eprintln!(
            "frost Object B spend offline OK mode={} path={}",
            receipt.mode,
            frost_escrow::ESCROW_SIGN_PATH_FROST_SPEND
        );
    }

    #[test]
    fn threshold_auth_via_authorize_escrow_release_shipped() {
        // Direct pure gate on real shipped function (not a parallel stub).
        let sealed = golden_sealed();
        let ev = lab_evidence_for_sealed(&sealed, 7_000, "mock_verify_lab");
        // Force in-process path (no live URL).
        let _g = EnvGuard::set(&[
            (ENV_HASH_MARKET_URL, ""),
            (ENV_CORRIDOR_HASH_MARKET_URL, ""),
            (ENV_CORRIDOR_REQUIRE_LIVE_THRESHOLD, ""),
        ]);
        let (auth, label) = lab_threshold_escrow_auth(&ev).expect("sign");
        assert!(label.contains(COMMITTEE_CRYPTO_LAB_T_OF_N));
        assert!(label.contains(AUTH_SOURCE_IN_PROCESS_LAB));
        let meta = peek_last_threshold_auth_meta().expect("meta stored");
        assert_eq!(meta.auth_source, AUTH_SOURCE_IN_PROCESS_LAB);
        assert!(!meta.frost);
        let gated = authorize_threshold_escrow_for_evidence(&ev, &sealed, &auth).expect("gate");
        assert_eq!(gated.mode, MODE_THRESHOLD_ESCROW_RELEASE);
        assert_eq!(gated.amount_zat, 7_000);

        // Wrong dest fails closed.
        let mut wrong = sealed.clone();
        wrong.owner_binding = [0xEE; 32];
        wrong.owner_binding_hex = hex::encode(wrong.owner_binding);
        let err = authorize_threshold_escrow_for_evidence(&ev, &wrong, &auth).unwrap_err();
        assert!(matches!(err, LabPayError::DestMismatch { .. }), "{err}");

        // Empty sig fails.
        let bad = ThresholdEscrowAuth {
            committee_pk: auth.committee_pk.clone(),
            combined_sig: vec![],
        };
        let err = authorize_threshold_escrow_for_evidence(&ev, &sealed, &bad).unwrap_err();
        assert!(matches!(err, LabPayError::ThresholdAuth(_)), "{err}");
    }

    #[test]
    fn require_live_threshold_fails_closed_without_url() {
        let _g = EnvGuard::set(&[
            (ENV_HASH_MARKET_URL, ""),
            (ENV_CORRIDOR_HASH_MARKET_URL, ""),
            (ENV_CORRIDOR_REQUIRE_LIVE_THRESHOLD, "1"),
            (ENV_CORRIDOR_ZEC_RELEASE, "threshold_escrow"),
            (ENV_CORRIDOR_PROFILE, "omni_production_shaped"),
        ]);
        let sealed = golden_sealed();
        let ev = lab_evidence_for_sealed(&sealed, 1_000, "mock_verify_lab");
        let err = lab_threshold_escrow_auth(&ev).unwrap_err();
        assert!(matches!(err, LabPayError::ThresholdAuth(_)), "{err}");
        assert!(
            err.to_string().contains("live custody")
                || err.to_string().contains("live threshold")
                || err.to_string().contains("HASH_MARKET"),
            "err={err}"
        );
    }

    #[test]
    fn product_profile_refuses_lab_inventory_without_flag() {
        // Empty residual value → env_truthy_lab is false (same as unset for our matcher).
        let _g = EnvGuard::set(&[
            (ENV_CORRIDOR_PROFILE, "omni_production_shaped"),
            (ENV_CORRIDOR_ZEC_RELEASE, "lab_inventory"),
            (ENV_CORRIDOR_ALLOW_LAB_INVENTORY_RESIDUAL, ""),
        ]);

        assert!(is_product_profile());
        assert_eq!(zec_release_policy(), ZecReleasePolicy::LabInventory);
        assert!(!allow_lab_inventory_residual());

        let sealed = golden_sealed();
        let ev = lab_evidence_for_sealed(&sealed, 1_000, "mock_verify_lab");
        let err = lab_pay_zec_after_burn_with_cfg(&ev, &sealed, &offline_cfg()).unwrap_err();
        assert!(
            matches!(err, LabPayError::ProductRejectFunderOnly(_)),
            "expected product reject, got {err}"
        );
        assert!(reject_funder_only_as_product(
            MODE_LAB_INVENTORY_PAY_SIMULATED
        ));
    }

    #[test]
    fn product_profile_inventory_residual_flag_allows_labeled() {
        let _g = EnvGuard::set(&[
            (ENV_CORRIDOR_PROFILE, "product"),
            (ENV_CORRIDOR_ZEC_RELEASE, "lab_inventory"),
            (ENV_CORRIDOR_ALLOW_LAB_INVENTORY_RESIDUAL, "1"),
            ("CORRIDOR_LAB_ZEC_FORCE_SIMULATED", "1"),
        ]);
        let sealed = golden_sealed();
        let ev = lab_evidence_for_sealed(&sealed, 500, "mock_verify_lab");
        let receipt =
            lab_pay_zec_after_burn_with_cfg(&ev, &sealed, &offline_cfg()).expect("residual ok");
        assert_eq!(receipt.mode, MODE_LAB_INVENTORY_PAY_SIMULATED);
        // Residual flag: assert_product_release_mode allows inventory.
        assert_product_release_mode(&receipt.mode).expect("residual allowed");
    }

    #[test]
    fn reject_funder_only_as_product_ssot() {
        assert!(reject_funder_only_as_product(MODE_LAB_INVENTORY_PAY));
        assert!(reject_funder_only_as_product(
            MODE_LAB_INVENTORY_PAY_SIMULATED
        ));
        assert!(!reject_funder_only_as_product(
            MODE_THRESHOLD_ESCROW_RELEASE
        ));
        assert!(!reject_funder_only_as_product(
            MODE_THRESHOLD_ESCROW_RELEASE_SIMULATED
        ));
    }

    #[test]
    fn open_confirm_live_skip_clean_when_available() {
        let cfg = ZakuraLocalConfig::default();
        if !rpc_ready(&cfg) {
            eprintln!("skip live open_confirm: RPC not at {}", cfg.rpc_url);
            return;
        }
        let sealed = golden_sealed();
        let r = confirm_open_at_sealed_dest_with_cfg(&sealed, &cfg).expect("confirm");
        assert!(r.rpc_ready);
        assert!(
            r.mode == OPEN_CONFIRM_RPC_OK
                || r.mode == OPEN_CONFIRM_SKIP_VALIDATE_ONLY
                || r.mode == OPEN_CONFIRM_SKIP_NO_WALLET,
            "unexpected mode {}",
            r.mode
        );
        // Regtest miner dest should usually validate on Zakura regtest.
        if r.address_valid == Some(false) {
            eprintln!(
                "warn: validateaddress false for {} (network residual)",
                sealed.dest_display
            );
        }
        eprintln!(
            "open_confirm live mode={} valid={:?} bal={:?} recv={:?} note={}",
            r.mode, r.address_valid, r.wallet_balance_zat, r.received_zat, r.note
        );
    }

    #[test]
    fn real_txid_classifier() {
        assert!(!is_real_zec_txid(&synthetic_lab_txid(&[1u8; 32], 1)));
        assert!(!is_real_zec_txid(""));
        assert!(is_real_zec_txid(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        ));
        assert!(is_real_zec_txid("opid-deadbeef"));
        assert!(!rpc_err_method_missing("insufficient funds"));
        assert!(rpc_err_method_missing(
            r#"{"code":-32601,"message":"Method not found"}"#
        ));
    }

    #[test]
    fn wallet_probe_offline_false() {
        assert!(!wallet_rpc_available(&offline_cfg()));
    }
}
