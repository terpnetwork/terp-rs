//! Multi-net dual-home prep (GUIDE Phase 1 / WAVE-2 MN-DUAL-HOME).
//!
//! Plane **H** evidence for omni product path:
//! - bitcoind regtest: separate **LP** + **user** UTXO buckets
//! - Zakura escrow: native prefund (wallet RPC) with I1 coverage of planned LP ZEC
//!
//! Shell hook [`e2e/hooks/omni-dual-home-prep.sh`] writes the artifact consumed here.
//! Pure film fixtures remain available via [`DualHomePrepV0::omni_fixture`] when
//! multi-net is unavailable — **not** product green under
//! `CORRIDOR_OMNI_REQUIRE_PRODUCT_GREEN=1`.
//!
//! SSOT: `GUIDE-OMNI-E2E-PRODUCTION-SHAPED-2026-07-22.md` §5,
//! `WAVE-2-FULL-MULTINET-FROST-PLAN.md` G1.

use std::fs;
use std::path::{Path, PathBuf};

use terp_seams::dex::{DualHomePrepV0, OMNI_LP_BTC, OMNI_LP_ZEC, OMNI_USER_BTC};
use serde::{Deserialize, Serialize};

use super::lab_pay_zec::{try_getbalance_zat, try_getreceivedbyaddress_zat, wallet_rpc_available};
use super::omni_lp_seed::OmniLpError;
use super::zakura_local::{json_rpc_call, rpc_ready, ZakuraLocalConfig, ZakuraLocalError};

// ---------------------------------------------------------------------------
// Env
// ---------------------------------------------------------------------------

/// Path to multi-net dual-home prep JSON (written by shell hook or harness).
pub const ENV_CORRIDOR_DUAL_HOME_PREP_PATH: &str = "CORRIDOR_DUAL_HOME_PREP_PATH";
/// When `1`/`true`, dual-home must be multi-net funded (BTC LP+user + escrow cover).
pub const ENV_CORRIDOR_OMNI_REQUIRE_PRODUCT_GREEN: &str = "CORRIDOR_OMNI_REQUIRE_PRODUCT_GREEN";
/// Escrow transparent address for prefund / balance probes (defaults to corridor golden dest).
pub const ENV_CORRIDOR_ESCROW_ADDR: &str = "CORRIDOR_ESCROW_ADDR";

// ---------------------------------------------------------------------------
// Artifact schema (GUIDE §12: 01-home-btc / 02-home-zec + dual-home-prep.json)
// ---------------------------------------------------------------------------

/// Multi-net dual-home prep document (run-dir SSOT for Phases 1).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DualHomeMultiNetPrepV0 {
    pub phase: u32,
    pub status: String,
    /// True only when native BTC LP+user UTXOs were funded on bitcoind regtest.
    pub btc_funded: bool,
    /// True only when Zakura wallet path shows escrow ≥ required zats (or explicit prefund tx).
    pub escrow_prefunded: bool,
    /// Combined: both homes real → multi-net dual-home product shape for Phase 1.
    pub multi_net: bool,
    pub lp_btc_sats: u64,
    pub user_btc_sats: u64,
    pub lp_zec_zats: u64,
    /// Minimum escrow native (zats) required for I1 start (LP ZEC + buffer).
    pub escrow_prefund_min_zats: u64,
    /// Observed / claimed escrow balance after prep.
    pub escrow_balance_zats: u64,
    pub escrow_addr: String,
    #[serde(default)]
    pub escrow_prefund_txid: Option<String>,
    #[serde(default)]
    pub lp_btc_addr: Option<String>,
    #[serde(default)]
    pub user_btc_addr: Option<String>,
    #[serde(default)]
    pub lp_btc_txid: Option<String>,
    #[serde(default)]
    pub user_btc_txid: Option<String>,
    #[serde(default)]
    pub lp_btc_vout: Option<u32>,
    #[serde(default)]
    pub user_btc_vout: Option<u32>,
    #[serde(default)]
    pub lp_btc_outpoint: Option<String>,
    #[serde(default)]
    pub user_btc_outpoint: Option<String>,
    #[serde(default)]
    pub layer: String,
    #[serde(default)]
    pub residuals: Vec<String>,
    #[serde(default)]
    pub note: String,
}

impl DualHomeMultiNetPrepV0 {
    /// Planned fixture amounts (not yet multi-net funded).
    pub fn planned_fixture() -> Self {
        let lp_zec = OMNI_LP_ZEC as u64;
        let min = lp_zec + lp_zec / 10;
        Self {
            phase: 1,
            status: "planned".into(),
            btc_funded: false,
            escrow_prefunded: false,
            multi_net: false,
            lp_btc_sats: OMNI_LP_BTC as u64,
            user_btc_sats: OMNI_USER_BTC as u64,
            lp_zec_zats: lp_zec,
            escrow_prefund_min_zats: min,
            escrow_balance_zats: 0,
            escrow_addr: super::zakura_local::default_dest_display(),
            escrow_prefund_txid: None,
            lp_btc_addr: None,
            user_btc_addr: None,
            lp_btc_txid: None,
            user_btc_txid: None,
            lp_btc_vout: None,
            user_btc_vout: None,
            lp_btc_outpoint: None,
            user_btc_outpoint: None,
            layer: "planned".into(),
            residuals: vec!["dual_home_not_funded".into()],
            note: "Fixture plan only — not multi-net product green".into(),
        }
    }

    /// Map to pure [`DualHomePrepV0`] for LP bootstrap (amounts + labels).
    ///
    /// Uses **observed** `escrow_balance_zats` only (never invents cover). Callers must
    /// fail closed when underfunded under product green.
    pub fn to_pure_prep(&self) -> DualHomePrepV0 {
        DualHomePrepV0 {
            lp_btc_sats: self.lp_btc_sats as u128,
            user_btc_sats: self.user_btc_sats as u128,
            lp_zec_zats: self.lp_zec_zats as u128,
            escrow_balance_zats: self.escrow_balance_zats as u128,
            escrow_addr_label: self.escrow_addr.clone(),
            lp_btc_outpoint_label: self
                .lp_btc_outpoint
                .clone()
                .or_else(|| {
                    self.lp_btc_txid
                        .as_ref()
                        .map(|t| format!("{}:{}", t, self.lp_btc_vout.unwrap_or(0)))
                })
                .unwrap_or_else(|| "lp-btc-utxo-pending".into()),
            user_btc_outpoint_label: self
                .user_btc_outpoint
                .clone()
                .or_else(|| {
                    self.user_btc_txid
                        .as_ref()
                        .map(|t| format!("{}:{}", t, self.user_btc_vout.unwrap_or(0)))
                })
                .unwrap_or_else(|| "user-btc-utxo-pending".into()),
        }
    }

    /// Product dual-home green: both homes funded with separable BTC claims + escrow cover.
    pub fn is_product_dual_home_green(&self) -> bool {
        self.multi_net
            && self.btc_funded
            && self.escrow_prefunded
            && self.escrow_balance_zats >= self.escrow_prefund_min_zats
            && self.escrow_balance_zats >= self.lp_zec_zats
            && self.lp_btc_sats > 0
            && self.user_btc_sats > 0
            && self.status == "complete"
    }

    /// Fail closed under product green requirement.
    pub fn assert_product_green_or_soft(&self) -> Result<(), OmniDualHomeError> {
        if !require_omni_product_green() {
            return Ok(());
        }
        if self.is_product_dual_home_green() {
            return Ok(());
        }
        Err(OmniDualHomeError::ProductGreenRequired(format!(
            "dual-home not multi-net product green (btc_funded={} escrow_prefunded={} multi_net={} balance={} min={} residuals={:?})",
            self.btc_funded,
            self.escrow_prefunded,
            self.multi_net,
            self.escrow_balance_zats,
            self.escrow_prefund_min_zats,
            self.residuals
        )))
    }

    pub fn write_json(&self, path: &Path) -> Result<(), OmniDualHomeError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| OmniDualHomeError::Io(e.to_string()))?;
        }
        let s = serde_json::to_string_pretty(self)
            .map_err(|e| OmniDualHomeError::Json(e.to_string()))?;
        fs::write(path, s).map_err(|e| OmniDualHomeError::Io(e.to_string()))
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum OmniDualHomeError {
    #[error("io: {0}")]
    Io(String),
    #[error("json: {0}")]
    Json(String),
    #[error("product green required: {0}")]
    ProductGreenRequired(String),
    #[error("escrow underfunded: balance={balance} need={need}")]
    EscrowUnderfunded { balance: u64, need: u64 },
    #[error("zakura: {0}")]
    Zakura(String),
    #[error("{0}")]
    Other(String),
}

impl From<ZakuraLocalError> for OmniDualHomeError {
    fn from(e: ZakuraLocalError) -> Self {
        OmniDualHomeError::Zakura(e.to_string())
    }
}

impl From<OmniDualHomeError> for OmniLpError {
    fn from(e: OmniDualHomeError) -> Self {
        match e {
            OmniDualHomeError::EscrowUnderfunded { balance, need } => {
                OmniLpError::Other(format!("escrow underfunded balance={balance} need={need}"))
            }
            other => OmniLpError::Other(other.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Path / env helpers
// ---------------------------------------------------------------------------

pub fn require_omni_product_green() -> bool {
    matches!(
        std::env::var(ENV_CORRIDOR_OMNI_REQUIRE_PRODUCT_GREEN).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    )
}

pub fn dual_home_prep_path() -> PathBuf {
    if let Ok(p) = std::env::var(ENV_CORRIDOR_DUAL_HOME_PREP_PATH) {
        return PathBuf::from(p);
    }
    if let Ok(run) = std::env::var("CORRIDOR_OMNI_RUN_DIR") {
        return PathBuf::from(run).join("dual-home-prep.json");
    }
    PathBuf::from("/tmp/corridor-dual-home-prep.json")
}

pub fn load_dual_home_prep(path: &Path) -> Result<DualHomeMultiNetPrepV0, OmniDualHomeError> {
    let s = fs::read_to_string(path).map_err(|e| OmniDualHomeError::Io(e.to_string()))?;
    serde_json::from_str(&s).map_err(|e| OmniDualHomeError::Json(e.to_string()))
}

/// Load prep from env path if present; else planned fixture (soft) or hard fail.
pub fn load_dual_home_prep_or_fixture() -> Result<DualHomeMultiNetPrepV0, OmniDualHomeError> {
    let path = dual_home_prep_path();
    if path.is_file() {
        let prep = load_dual_home_prep(&path)?;
        prep.assert_product_green_or_soft()?;
        return Ok(prep);
    }
    let planned = DualHomeMultiNetPrepV0::planned_fixture();
    planned.assert_product_green_or_soft()?;
    Ok(planned)
}

/// Resolve pure DualHomePrep for bootstrap: multi-net artifact → pure; else fixture.
///
/// Product green (`CORRIDOR_OMNI_REQUIRE_PRODUCT_GREEN=1`) requires multi-net
/// dual-home complete with escrow ≥ LP ZEC. Soft path falls back to omni fixture
/// amounts while preserving any real BTC outpoint labels when BTC was funded.
pub fn resolve_dual_home_for_bootstrap() -> Result<DualHomePrepV0, OmniDualHomeError> {
    let multi = load_dual_home_prep_or_fixture()?;

    if require_omni_product_green() {
        multi.assert_product_green_or_soft()?;
        if multi.escrow_balance_zats < multi.lp_zec_zats {
            return Err(OmniDualHomeError::EscrowUnderfunded {
                balance: multi.escrow_balance_zats,
                need: multi.lp_zec_zats,
            });
        }
        let pure = multi.to_pure_prep();
        pure.assert_escrow_covers_lp_zec().map_err(|_| {
            OmniDualHomeError::EscrowUnderfunded {
                balance: multi.escrow_balance_zats,
                need: multi.lp_zec_zats,
            }
        })?;
        return Ok(pure);
    }

    // Soft packaging: use multi-net amounts when escrow_prefunded; else fixture
    // economics with optional real BTC outpoint labels.
    if multi.escrow_prefunded && multi.escrow_balance_zats >= multi.lp_zec_zats {
        let pure = multi.to_pure_prep();
        let _ = pure.assert_escrow_covers_lp_zec();
        return Ok(pure);
    }

    let mut pure = DualHomePrepV0::omni_fixture();
    if multi.btc_funded {
        pure.lp_btc_sats = multi.lp_btc_sats as u128;
        pure.user_btc_sats = multi.user_btc_sats as u128;
        if let Some(ref op) = multi.lp_btc_outpoint {
            pure.lp_btc_outpoint_label = op.clone();
        }
        if let Some(ref op) = multi.user_btc_outpoint {
            pure.user_btc_outpoint_label = op.clone();
        }
    }
    Ok(pure)
}

// ---------------------------------------------------------------------------
// Zakura escrow observe / prefund (wallet RPC)
// ---------------------------------------------------------------------------

/// Probe escrow coverage via wallet RPC. Does **not** mint; observe plane only.
pub fn observe_escrow_balance(
    cfg: &ZakuraLocalConfig,
    escrow_addr: &str,
) -> Result<EscrowObserveV0, OmniDualHomeError> {
    if !rpc_ready(cfg) {
        return Ok(EscrowObserveV0 {
            rpc_ready: false,
            wallet_rpc: false,
            wallet_balance_zat: None,
            received_by_addr_zat: None,
            escrow_addr: escrow_addr.into(),
            covers_lp_zec: false,
            note: "zakura rpc down".into(),
        });
    }
    let wallet = wallet_rpc_available(cfg);
    let wallet_balance_zat = if wallet {
        try_getbalance_zat(cfg).ok().flatten()
    } else {
        None
    };
    let received_by_addr_zat = try_getreceivedbyaddress_zat(cfg, escrow_addr)
        .ok()
        .flatten();
    let observed = wallet_balance_zat
        .or(received_by_addr_zat)
        .unwrap_or(0);
    let need = OMNI_LP_ZEC as u64;
    Ok(EscrowObserveV0 {
        rpc_ready: true,
        wallet_rpc: wallet,
        wallet_balance_zat,
        received_by_addr_zat,
        escrow_addr: escrow_addr.into(),
        covers_lp_zec: observed >= need,
        note: if !wallet {
            "zakura core without wallet RPC — escrow prefund residual".into()
        } else if observed >= need {
            "escrow balance covers planned LP ZEC".into()
        } else {
            format!("escrow underfunded observed={observed} need={need}")
        },
    })
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EscrowObserveV0 {
    pub rpc_ready: bool,
    pub wallet_rpc: bool,
    pub wallet_balance_zat: Option<u64>,
    pub received_by_addr_zat: Option<u64>,
    pub escrow_addr: String,
    pub covers_lp_zec: bool,
    pub note: String,
}

/// Attempt to prefund escrow via transparent `sendtoaddress` (wallet residual).
///
/// Returns txid when send succeeds. Fail closed under product green if underfunded
/// after attempt.
pub fn try_prefund_escrow_wallet(
    cfg: &ZakuraLocalConfig,
    escrow_addr: &str,
    amount_zat: u64,
) -> Result<PrefundEscrowResult, OmniDualHomeError> {
    let before = observe_escrow_balance(cfg, escrow_addr)?;
    if before.covers_lp_zec {
        let bal = before
            .wallet_balance_zat
            .or(before.received_by_addr_zat)
            .unwrap_or(amount_zat);
        return Ok(PrefundEscrowResult {
            status: "already_funded".into(),
            txid: None,
            balance_after_zat: bal,
            amount_sent_zat: 0,
            wallet_rpc: before.wallet_rpc,
            note: "escrow already covers LP ZEC".into(),
        });
    }
    if !before.wallet_rpc {
        if require_omni_product_green() {
            return Err(OmniDualHomeError::ProductGreenRequired(
                "Zakura wallet RPC unavailable; cannot prefund escrow under product green".into(),
            ));
        }
        return Ok(PrefundEscrowResult {
            status: "residual_no_wallet".into(),
            txid: None,
            balance_after_zat: 0,
            amount_sent_zat: 0,
            wallet_rpc: false,
            note: before.note,
        });
    }
    // sendtoaddress amount in ZEC
    let zec = amount_zat as f64 / 100_000_000.0;
    match json_rpc_call(cfg, "sendtoaddress", serde_json::json!([escrow_addr, zec])) {
        Ok(serde_json::Value::String(txid)) if !txid.is_empty() => {
            let after = observe_escrow_balance(cfg, escrow_addr)?;
            let bal = after
                .wallet_balance_zat
                .or(after.received_by_addr_zat)
                .unwrap_or(amount_zat);
            if require_omni_product_green() && bal < amount_zat && !after.covers_lp_zec {
                return Err(OmniDualHomeError::EscrowUnderfunded {
                    balance: bal,
                    need: amount_zat,
                });
            }
            Ok(PrefundEscrowResult {
                status: "prefunded".into(),
                txid: Some(txid),
                balance_after_zat: bal.max(amount_zat),
                amount_sent_zat: amount_zat,
                wallet_rpc: true,
                note: "sendtoaddress escrow prefund".into(),
            })
        }
        Ok(other) => {
            if require_omni_product_green() {
                return Err(OmniDualHomeError::Zakura(format!(
                    "sendtoaddress unexpected: {other}"
                )));
            }
            Ok(PrefundEscrowResult {
                status: "residual_send_failed".into(),
                txid: None,
                balance_after_zat: 0,
                amount_sent_zat: 0,
                wallet_rpc: true,
                note: format!("sendtoaddress unexpected: {other}"),
            })
        }
        Err(e) => {
            if require_omni_product_green() {
                return Err(OmniDualHomeError::from(e));
            }
            Ok(PrefundEscrowResult {
                status: "residual_send_error".into(),
                txid: None,
                balance_after_zat: 0,
                amount_sent_zat: 0,
                wallet_rpc: true,
                note: e.to_string(),
            })
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrefundEscrowResult {
    pub status: String,
    pub txid: Option<String>,
    pub balance_after_zat: u64,
    pub amount_sent_zat: u64,
    pub wallet_rpc: bool,
    pub note: String,
}

// ---------------------------------------------------------------------------
// Run-dir receipt writers (GUIDE §12)
// ---------------------------------------------------------------------------

/// Write `01-home-btc.json` and `02-home-zec-escrow.json` (+ dual-home-prep.json).
pub fn write_dual_home_run_pack(
    run_dir: &Path,
    prep: &DualHomeMultiNetPrepV0,
) -> Result<(), OmniDualHomeError> {
    fs::create_dir_all(run_dir).map_err(|e| OmniDualHomeError::Io(e.to_string()))?;
    prep.write_json(&run_dir.join("dual-home-prep.json"))?;

    let btc = serde_json::json!({
        "phase": 1,
        "status": if prep.btc_funded { "complete" } else { prep.status.as_str() },
        "multi_net": prep.btc_funded,
        "lp_btc_sats": prep.lp_btc_sats,
        "user_btc_sats": prep.user_btc_sats,
        "lp_btc_addr": prep.lp_btc_addr,
        "user_btc_addr": prep.user_btc_addr,
        "lp_btc_txid": prep.lp_btc_txid,
        "user_btc_txid": prep.user_btc_txid,
        "lp_btc_vout": prep.lp_btc_vout,
        "user_btc_vout": prep.user_btc_vout,
        "lp_btc_outpoint": prep.lp_btc_outpoint,
        "user_btc_outpoint": prep.user_btc_outpoint,
        "layer": prep.layer,
        "note": "LP and user UTXOs must be separable claims (different outpoints / intent_ids)",
        "residuals": prep.residuals,
    });
    fs::write(
        run_dir.join("01-home-btc.json"),
        serde_json::to_string_pretty(&btc).map_err(|e| OmniDualHomeError::Json(e.to_string()))?,
    )
    .map_err(|e| OmniDualHomeError::Io(e.to_string()))?;

    let zec = serde_json::json!({
        "phase": 1,
        "status": if prep.escrow_prefunded { "complete" } else { prep.status.as_str() },
        "multi_net": prep.escrow_prefunded,
        "lp_zec_zats": prep.lp_zec_zats,
        "escrow_prefund_min_zats": prep.escrow_prefund_min_zats,
        "escrow_balance_zats": prep.escrow_balance_zats,
        "escrow_addr": prep.escrow_addr,
        "escrow_prefund_txid": prep.escrow_prefund_txid,
        "conservation": "I1: outstanding ZEC-SEAM ≤ escrow native",
        "layer": prep.layer,
        "note": if prep.escrow_prefunded {
            "Escrow prefunded under wallet RPC path"
        } else {
            "Prefund escrow under committee key material before LP ZEC mint"
        },
        "residuals": prep.residuals,
    });
    fs::write(
        run_dir.join("02-home-zec-escrow.json"),
        serde_json::to_string_pretty(&zec).map_err(|e| OmniDualHomeError::Json(e.to_string()))?,
    )
    .map_err(|e| OmniDualHomeError::Io(e.to_string()))
}

/// Merge dual-home multi-net flags into classification helpers used by bootstrap scripts.
pub fn dual_home_supports_multinet_lp(prep: &DualHomeMultiNetPrepV0) -> bool {
    prep.is_product_dual_home_green()
}

/// Default amounts for shell/rust (keep aligned with pure fixtures).
pub fn fixture_amounts() -> (u64, u64, u64) {
    (
        OMNI_LP_BTC as u64,
        OMNI_USER_BTC as u64,
        OMNI_LP_ZEC as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        keys: Vec<(&'static str, Option<String>)>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl EnvGuard {
        fn clear(keys: &[&'static str]) -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let mut saved = Vec::new();
            for k in keys {
                saved.push((*k, std::env::var(k).ok()));
                unsafe {
                    std::env::remove_var(k);
                }
            }
            Self {
                keys: saved,
                _lock: lock,
            }
        }
        fn set(pairs: &[(&'static str, &str)]) -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let mut keys = Vec::new();
            for (k, v) in pairs {
                keys.push((*k, std::env::var(k).ok()));
                unsafe {
                    std::env::set_var(k, v);
                }
            }
            Self { keys, _lock: lock }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, prev) in self.keys.drain(..) {
                unsafe {
                    match prev {
                        Some(v) => std::env::set_var(k, v),
                        None => std::env::remove_var(k),
                    }
                }
            }
        }
    }

    #[test]
    fn planned_fixture_not_product_green() {
        let p = DualHomeMultiNetPrepV0::planned_fixture();
        assert!(!p.is_product_dual_home_green());
        assert!(!p.multi_net);
        assert_eq!(p.lp_btc_sats, OMNI_LP_BTC as u64);
    }

    #[test]
    fn product_green_requires_both_homes() {
        let mut p = DualHomeMultiNetPrepV0::planned_fixture();
        p.status = "complete".into();
        p.btc_funded = true;
        p.escrow_prefunded = true;
        p.multi_net = true;
        p.escrow_balance_zats = p.escrow_prefund_min_zats;
        p.lp_btc_outpoint = Some("aa:0".into());
        p.user_btc_outpoint = Some("bb:0".into());
        assert!(p.is_product_dual_home_green());
        let pure = p.to_pure_prep();
        assert_eq!(pure.lp_btc_sats, OMNI_LP_BTC);
        assert_eq!(pure.escrow_balance_zats, p.escrow_balance_zats as u128);
        pure.assert_escrow_covers_lp_zec().unwrap();
    }

    #[test]
    fn require_product_green_fails_planned() {
        let _g = EnvGuard::set(&[(ENV_CORRIDOR_OMNI_REQUIRE_PRODUCT_GREEN, "1")]);
        let p = DualHomeMultiNetPrepV0::planned_fixture();
        assert!(p.assert_product_green_or_soft().is_err());
    }

    #[test]
    fn soft_path_allows_planned() {
        let _g = EnvGuard::clear(&[ENV_CORRIDOR_OMNI_REQUIRE_PRODUCT_GREEN]);
        let p = DualHomeMultiNetPrepV0::planned_fixture();
        p.assert_product_green_or_soft().unwrap();
    }

    #[test]
    fn write_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("omni-dh-{}", std::process::id()));
        let mut p = DualHomeMultiNetPrepV0::planned_fixture();
        p.status = "complete".into();
        p.btc_funded = true;
        p.escrow_prefunded = true;
        p.multi_net = true;
        p.escrow_balance_zats = p.escrow_prefund_min_zats;
        p.lp_btc_txid = Some("ab".repeat(32));
        p.user_btc_txid = Some("cd".repeat(32));
        p.lp_btc_outpoint = Some(format!("{}:0", p.lp_btc_txid.as_ref().unwrap()));
        p.user_btc_outpoint = Some(format!("{}:0", p.user_btc_txid.as_ref().unwrap()));
        write_dual_home_run_pack(&dir, &p).unwrap();
        let loaded = load_dual_home_prep(&dir.join("dual-home-prep.json")).unwrap();
        assert!(loaded.is_product_dual_home_green());
        assert!(dir.join("01-home-btc.json").is_file());
        assert!(dir.join("02-home-zec-escrow.json").is_file());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn underfunded_escrow_not_green() {
        let mut p = DualHomeMultiNetPrepV0::planned_fixture();
        p.status = "complete".into();
        p.btc_funded = true;
        p.escrow_prefunded = true;
        p.multi_net = true;
        p.escrow_balance_zats = 1;
        assert!(!p.is_product_dual_home_green());
    }
}
