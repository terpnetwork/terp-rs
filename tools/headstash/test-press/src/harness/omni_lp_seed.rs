//! OmniBridge-shaped liquidity bootstrap (GUIDE Phases 1–3).
//!
//! Dual-home prep → bridge-mint LP BTC/ZEC notes → seed pool reserves from those
//! notes with receipt linkage (`lp_seed.source=bridged_notes`).
//!
//! Under `CORRIDOR_POOL_SEED=bridged_lp` / `CORRIDOR_PROFILE=omni_production_shaped`,
//! product path **refuses** magic-only CreatePool (LAB_R_* without bridged source).
//!
//! SSOT pure: `terp_seams::dex::lp_seed`.
//! SSOT guide: `GUIDE-OMNI-E2E-PRODUCTION-SHAPED-2026-07-22.md`.

use std::fs;
use std::path::{Path, PathBuf};

use terp_seams::dex::{
    asset_id_b, asset_id_hub, authorize_create_pool_reserves, bootstrap_omni_liquidity_pure,
    reject_magic_pool_under_bridged_policy, DualHomePrepV0, EscrowLiabilityV0, LpBridgeMintV0,
    LpSeedError, LpSeedReceiptV0, PoolSeedPolicy, LP_SEED_SOURCE_BRIDGED_NOTES,
    LP_SEED_SOURCE_MAGIC_LAB, OMNI_LP_BTC, OMNI_LP_ZEC, OMNI_USER_BTC,
};
use serde::{Deserialize, Serialize};

use super::lab_pay_zec::is_product_profile;
use super::mint_evidence::{hex32, PROOF_MODE_MOCK_VERIFY_LAB};
use super::swap_statement_cw::{LAB_GAMMA, LAB_GAMMA_DEN, LAB_POOL_ID, LAB_R_IN, LAB_R_OUT};

// ---------------------------------------------------------------------------
// Env
// ---------------------------------------------------------------------------

/// `CORRIDOR_POOL_SEED=bridged_lp` forbids magic CreatePool alone.
pub const ENV_CORRIDOR_POOL_SEED: &str = "CORRIDOR_POOL_SEED";
pub const POOL_SEED_BRIDGED_LP: &str = "bridged_lp";
pub const POOL_SEED_MAGIC_LAB: &str = "magic_lab";

/// Resolve pool-seed policy from env (product / omni profile forces bridged_lp).
pub fn corridor_pool_seed_policy() -> PoolSeedPolicy {
    if is_product_profile() {
        return PoolSeedPolicy::BridgedLp;
    }
    match std::env::var(ENV_CORRIDOR_POOL_SEED) {
        Ok(v) => PoolSeedPolicy::parse(&v),
        Err(_) => PoolSeedPolicy::MagicLab,
    }
}

/// Alias: product/omni profile requires bridged LP (same set as release product profile).
pub fn corridor_profile_is_omni() -> bool {
    is_product_profile()
}

/// True when product path requires bridged LP seed (not magic reserves alone).
pub fn require_bridged_lp_seed() -> bool {
    corridor_pool_seed_policy() == PoolSeedPolicy::BridgedLp
}

// ---------------------------------------------------------------------------
// Artifacts
// ---------------------------------------------------------------------------

/// JSON receipt written for product green / CI (GUIDE §7.2 + §12 `05-lp-seed-pool.json`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LpSeedReceiptJson {
    pub profile: String,
    pub stage: String,
    pub status: String,
    pub lp_seed: LpSeedInnerJson,
    pub dual_home: DualHomePrepJson,
    pub proof_mode: String,
    pub mock_verify_lab: bool,
    pub policy: String,
    /// When true, dual-home native prep was multi-net (bitcoind + escrow).
    #[serde(default)]
    pub multi_net: bool,
    /// Honesty layer label (e.g. L2_pure_film, L3_dual_home+L2_lp_mint).
    #[serde(default)]
    pub layer: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LpSeedInnerJson {
    pub btc_mint_nf: String,
    pub zec_mint_nf: String,
    pub lp_btc_value: u64,
    pub lp_zec_value: u64,
    pub r_btc: u64,
    pub r_zec: u64,
    pub source: String,
    pub pool_id: u64,
    pub lp_nullifiers: Vec<String>,
    pub conservation_btc: String,
    pub conservation_zec: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DualHomePrepJson {
    pub lp_btc_sats: u64,
    pub user_btc_sats: u64,
    pub lp_zec_zats: u64,
    pub escrow_balance_zats: u64,
    pub escrow_addr_label: String,
    pub lp_btc_outpoint_label: String,
    pub user_btc_outpoint_label: String,
    pub outstanding_zec_claims: u64,
}

/// In-process bootstrap outcome (Phases 1–3).
#[derive(Clone, Debug)]
pub struct OmniLiquidityBootstrap {
    pub prep: DualHomePrepV0,
    pub liability: EscrowLiabilityV0,
    pub lp_btc_mint: LpBridgeMintV0,
    pub lp_zec_mint: LpBridgeMintV0,
    pub receipt: LpSeedReceiptV0,
    /// Reserves for CreatePool / handoff quote (BTC in, ZEC out).
    pub r_btc: u128,
    pub r_zec: u128,
    pub pool_id: u64,
    pub gamma: u64,
    pub gamma_den: u64,
    pub policy: PoolSeedPolicy,
    pub proof_mode: String,
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum OmniLpError {
    #[error("lp seed pure: {0:?}")]
    Pure(LpSeedError),
    #[error("product path: magic CreatePool forbidden under CORRIDOR_POOL_SEED=bridged_lp")]
    MagicPoolForbidden,
    #[error("missing bridged LP seed receipt under product policy")]
    MissingReceipt,
    #[error("reserves mismatch: expected r_btc={exp_btc} r_zec={exp_zec} got {got_btc}/{got_zec}")]
    ReservesMismatch {
        exp_btc: u128,
        exp_zec: u128,
        got_btc: u128,
        got_zec: u128,
    },
    #[error("io: {0}")]
    Io(String),
    #[error("json: {0}")]
    Json(String),
    #[error("{0}")]
    Other(String),
}

impl From<LpSeedError> for OmniLpError {
    fn from(e: LpSeedError) -> Self {
        match e {
            LpSeedError::ErrMagicPoolForbidden => OmniLpError::MagicPoolForbidden,
            LpSeedError::ErrMissingBridgedNotes => OmniLpError::MissingReceipt,
            other => OmniLpError::Pure(other),
        }
    }
}

/// Default LP seed receipt path.
pub fn lp_seed_receipt_path() -> PathBuf {
    std::env::var("CORRIDOR_LP_SEED_RECEIPT_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/corridor-lp-seed-receipt.json"))
}

/// Bootstrap Phases 1–3 (pure / lab film, labeled mock_verify).
///
/// Records LP BTC + LP ZEC mint evidence, checks escrow liability on ZEC mint,
/// seeds pool reserve values equal to bridged LP note values with receipt linkage.
///
/// Dual-home: when `CORRIDOR_DUAL_HOME_PREP_PATH` / `CORRIDOR_OMNI_RUN_DIR` has a
/// multi-net prep artifact, amounts and outpoint labels come from that prep
/// (fail-closed under `CORRIDOR_OMNI_REQUIRE_PRODUCT_GREEN=1` if underfunded).
pub fn bootstrap_omni_liquidity() -> Result<OmniLiquidityBootstrap, OmniLpError> {
    let prep = crate::harness::omni_dual_home::resolve_dual_home_for_bootstrap()
        .map_err(OmniLpError::from)?;
    bootstrap_omni_liquidity_with(prep, LAB_POOL_ID)
}

/// Bootstrap using explicit multi-net dual-home prep (amounts + liability).
pub fn bootstrap_omni_liquidity_from_multinet(
    multi: &crate::harness::omni_dual_home::DualHomeMultiNetPrepV0,
) -> Result<OmniLiquidityBootstrap, OmniLpError> {
    multi
        .assert_product_green_or_soft()
        .map_err(OmniLpError::from)?;
    let prep = multi.to_pure_prep();
    bootstrap_omni_liquidity_with(prep, LAB_POOL_ID)
}

/// Bootstrap with explicit prep / pool_id (tests).
pub fn bootstrap_omni_liquidity_with(
    prep: DualHomePrepV0,
    pool_id: u64,
) -> Result<OmniLiquidityBootstrap, OmniLpError> {
    let lp_owner = [0x4c; 32];
    let btc_rcm = {
        let mut r = [0u8; 32];
        r[0] = 0xB1;
        r[1] = 0x01;
        r
    };
    let zec_rcm = {
        let mut r = [0u8; 32];
        r[0] = 0xB2;
        r[1] = 0x02;
        r
    };

    let (pool, _state, liability, receipt, lp_btc_mint, lp_zec_mint) =
        bootstrap_omni_liquidity_pure(
            asset_id_hub(),
            asset_id_b(),
            &prep,
            lp_owner,
            btc_rcm,
            zec_rcm,
            pool_id,
            LAB_GAMMA,
            LAB_GAMMA_DEN,
            PROOF_MODE_MOCK_VERIFY_LAB,
        )
        .map_err(OmniLpError::from)?;

    // Product policy: must not be magic
    reject_magic_pool_under_bridged_policy(PoolSeedPolicy::BridgedLp, &receipt.source)
        .map_err(OmniLpError::from)?;
    authorize_create_pool_reserves(
        PoolSeedPolicy::BridgedLp,
        pool.r_a,
        pool.r_b,
        Some(&receipt),
    )
    .map_err(OmniLpError::from)?;

    Ok(OmniLiquidityBootstrap {
        prep,
        liability,
        lp_btc_mint,
        lp_zec_mint,
        receipt,
        r_btc: pool.r_a,
        r_zec: pool.r_b,
        pool_id,
        gamma: LAB_GAMMA,
        gamma_den: LAB_GAMMA_DEN,
        policy: PoolSeedPolicy::BridgedLp,
        proof_mode: PROOF_MODE_MOCK_VERIFY_LAB.into(),
    })
}

impl OmniLiquidityBootstrap {
    pub fn to_json_receipt(&self, profile: &str) -> LpSeedReceiptJson {
        self.to_json_receipt_with_multinet(profile, false, "L2_pure_film")
    }

    /// Receipt with multi-net dual-home honesty labels (GUIDE §12 / WAVE-2).
    pub fn to_json_receipt_with_multinet(
        &self,
        profile: &str,
        multi_net: bool,
        layer: &str,
    ) -> LpSeedReceiptJson {
        LpSeedReceiptJson {
            profile: profile.into(),
            stage: "lp_seed_pool".into(),
            status: "complete".into(),
            lp_seed: LpSeedInnerJson {
                btc_mint_nf: self.receipt.btc_mint_nf_hex.clone(),
                zec_mint_nf: self.receipt.zec_mint_nf_hex.clone(),
                lp_btc_value: self.receipt.lp_btc_value as u64,
                lp_zec_value: self.receipt.lp_zec_value as u64,
                r_btc: self.receipt.r_btc as u64,
                r_zec: self.receipt.r_zec as u64,
                source: self.receipt.source.clone(),
                pool_id: self.receipt.pool_id,
                lp_nullifiers: self.receipt.lp_nullifiers_hex.clone(),
                conservation_btc: self.receipt.conservation_btc.clone(),
                conservation_zec: self.receipt.conservation_zec.clone(),
            },
            dual_home: DualHomePrepJson {
                lp_btc_sats: self.prep.lp_btc_sats as u64,
                user_btc_sats: self.prep.user_btc_sats as u64,
                lp_zec_zats: self.prep.lp_zec_zats as u64,
                escrow_balance_zats: self.prep.escrow_balance_zats as u64,
                escrow_addr_label: self.prep.escrow_addr_label.clone(),
                lp_btc_outpoint_label: self.prep.lp_btc_outpoint_label.clone(),
                user_btc_outpoint_label: self.prep.user_btc_outpoint_label.clone(),
                outstanding_zec_claims: self.liability.outstanding_zec_claims as u64,
            },
            proof_mode: self.proof_mode.clone(),
            mock_verify_lab: self.proof_mode == PROOF_MODE_MOCK_VERIFY_LAB,
            policy: self.policy.as_str().into(),
            multi_net,
            layer: layer.into(),
        }
    }

    pub fn write_json(&self, path: &Path) -> Result<(), OmniLpError> {
        let multi_net = crate::harness::omni_dual_home::load_dual_home_prep(
            &crate::harness::omni_dual_home::dual_home_prep_path(),
        )
        .ok()
        .map(|m| m.is_product_dual_home_green())
        .unwrap_or(false);
        self.write_json_labeled(path, multi_net)
    }

    pub fn write_json_labeled(&self, path: &Path, multi_net: bool) -> Result<(), OmniLpError> {
        let layer = if multi_net {
            "L3_dual_home_native+L2_lp_mint_seed"
        } else {
            "L2_pure_film"
        };
        let doc = self.to_json_receipt_with_multinet("omni_production_shaped", multi_net, layer);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| OmniLpError::Io(e.to_string()))?;
        }
        let s = serde_json::to_string_pretty(&doc).map_err(|e| OmniLpError::Json(e.to_string()))?;
        fs::write(path, s).map_err(|e| OmniLpError::Io(e.to_string()))
    }

    /// Write GUIDE §12 LP mint + seed receipts into omni run dir (Phases 2–3).
    ///
    /// `multi_net_dual_home` is true only when dual-home native prep is product green;
    /// LP mints remain harness/pure (`mock_verify_lab`) until chain BridgeMint LP lands.
    pub fn write_omni_run_pack(
        &self,
        run_dir: &Path,
        multi_net_dual_home: bool,
    ) -> Result<(), OmniLpError> {
        fs::create_dir_all(run_dir).map_err(|e| OmniLpError::Io(e.to_string()))?;
        let layer = if multi_net_dual_home {
            "L3_dual_home_native+L2_lp_mint_seed"
        } else {
            "L2_pure_film"
        };
        let doc = self.to_json_receipt_with_multinet(
            "omni_production_shaped",
            multi_net_dual_home,
            layer,
        );

        self.write_json_labeled(&lp_seed_receipt_path(), multi_net_dual_home)?;
        // Also pin under run dir for receipt pack consumers.
        self.write_json_labeled(
            &run_dir.join("corridor-lp-seed-receipt.json"),
            multi_net_dual_home,
        )?;

        let seed_doc = serde_json::json!({
            "phase": 3,
            "status": "complete",
            "layer": layer,
            "multi_net": multi_net_dual_home,
            "lp_seed": doc.lp_seed,
            "dual_home": doc.dual_home,
            "proof_mode": doc.proof_mode,
            "mock_verify_lab": doc.mock_verify_lab,
            "policy": doc.policy,
            "note": if multi_net_dual_home {
                "Pool R from bridged LP notes after multi-net dual-home prep; LP mint layer still mock_verify_lab until chain LP BridgeMint"
            } else {
                "Pure/harness bootstrap_omni_liquidity; multi-net dual-home residual"
            },
        });
        fs::write(
            run_dir.join("05-lp-seed-pool.json"),
            serde_json::to_string_pretty(&seed_doc)
                .map_err(|e| OmniLpError::Json(e.to_string()))?,
        )
        .map_err(|e| OmniLpError::Io(e.to_string()))?;

        let btc_mint = serde_json::json!({
            "phase": 2,
            "status": "complete",
            "layer": layer,
            "multi_net_dual_home": multi_net_dual_home,
            "asset": "BTC",
            "value_sats": self.lp_btc_mint.value,
            "conservation_class": self.lp_btc_mint.conservation_class,
            "bridge_nullifier_hex": hex32(&self.lp_btc_mint.bridge_nullifier),
            "cm_hex": hex32(&self.lp_btc_mint.cm_public),
            "proof_mode": self.proof_mode,
            "lp_btc_outpoint": self.prep.lp_btc_outpoint_label,
        });
        fs::write(
            run_dir.join("03-lp-mint-btc.json"),
            serde_json::to_string_pretty(&btc_mint)
                .map_err(|e| OmniLpError::Json(e.to_string()))?,
        )
        .map_err(|e| OmniLpError::Io(e.to_string()))?;

        let zec_mint = serde_json::json!({
            "phase": 2,
            "status": "complete",
            "layer": layer,
            "multi_net_dual_home": multi_net_dual_home,
            "asset": "ZEC",
            "value_zats": self.lp_zec_mint.value,
            "conservation_class": self.lp_zec_mint.conservation_class,
            "bridge_nullifier_hex": hex32(&self.lp_zec_mint.bridge_nullifier),
            "cm_hex": hex32(&self.lp_zec_mint.cm_public),
            "proof_mode": self.proof_mode,
            "escrow_balance_zats": self.prep.escrow_balance_zats,
            "outstanding_zec_claims": self.liability.outstanding_zec_claims,
            "liability_ok": self.prep.escrow_balance_zats >= self.liability.outstanding_zec_claims,
        });
        fs::write(
            run_dir.join("04-lp-mint-zec.json"),
            serde_json::to_string_pretty(&zec_mint)
                .map_err(|e| OmniLpError::Json(e.to_string()))?,
        )
        .map_err(|e| OmniLpError::Io(e.to_string()))?;

        Ok(())
    }

    /// Assert product-green shape: source bridged_notes, R == LP values.
    pub fn assert_product_green(&self) -> Result<(), OmniLpError> {
        if self.receipt.source != LP_SEED_SOURCE_BRIDGED_NOTES {
            return Err(OmniLpError::MagicPoolForbidden);
        }
        self.receipt
            .assert_reserves_match_notes()
            .map_err(OmniLpError::from)?;
        if self.r_btc != self.lp_btc_mint.value || self.r_zec != self.lp_zec_mint.value {
            return Err(OmniLpError::ReservesMismatch {
                exp_btc: self.lp_btc_mint.value,
                exp_zec: self.lp_zec_mint.value,
                got_btc: self.r_btc,
                got_zec: self.r_zec,
            });
        }
        Ok(())
    }
}

/// Reserves to use for CreatePool / settle handoff under current policy.
///
/// - `bridged_lp`: must pass `bootstrap` receipt; returns LP note values
/// - `magic_lab`: legacy LAB_R_IN / LAB_R_OUT
pub fn resolve_pool_reserves(
    bootstrap: Option<&OmniLiquidityBootstrap>,
) -> Result<(u128, u128, Option<&LpSeedReceiptV0>), OmniLpError> {
    let policy = corridor_pool_seed_policy();
    match policy {
        PoolSeedPolicy::BridgedLp => {
            let boot = bootstrap.ok_or(OmniLpError::MissingReceipt)?;
            boot.assert_product_green()?;
            authorize_create_pool_reserves(
                PoolSeedPolicy::BridgedLp,
                boot.r_btc,
                boot.r_zec,
                Some(&boot.receipt),
            )
            .map_err(OmniLpError::from)?;
            Ok((boot.r_btc, boot.r_zec, Some(&boot.receipt)))
        }
        PoolSeedPolicy::MagicLab => {
            // Explicit refuse if someone marks source as required bridged but skipped
            if require_bridged_lp_seed() {
                return Err(OmniLpError::MagicPoolForbidden);
            }
            Ok((LAB_R_IN, LAB_R_OUT, None))
        }
    }
}

/// Fail-closed gate used by corridor settle: product path cannot invent R.
pub fn refuse_magic_pool_if_bridged_policy(
    r_btc: u128,
    r_zec: u128,
    bootstrap: Option<&OmniLiquidityBootstrap>,
) -> Result<(), OmniLpError> {
    let policy = corridor_pool_seed_policy();
    if policy != PoolSeedPolicy::BridgedLp {
        return Ok(());
    }
    let Some(boot) = bootstrap else {
        return Err(OmniLpError::MissingReceipt);
    };
    // Detect magic lab constants used as if they were product reserves
    if r_btc == LAB_R_IN && r_zec == LAB_R_OUT && (boot.r_btc != LAB_R_IN || boot.r_zec != LAB_R_OUT)
    {
        return Err(OmniLpError::MagicPoolForbidden);
    }
    if r_btc != boot.r_btc || r_zec != boot.r_zec {
        return Err(OmniLpError::ReservesMismatch {
            exp_btc: boot.r_btc,
            exp_zec: boot.r_zec,
            got_btc: r_btc,
            got_zec: r_zec,
        });
    }
    reject_magic_pool_under_bridged_policy(policy, &boot.receipt.source).map_err(OmniLpError::from)
}

/// Convenience: source label for receipts when magic lab path is used.
pub fn magic_lab_source_label() -> &'static str {
    LP_SEED_SOURCE_MAGIC_LAB
}

pub fn bridged_notes_source_label() -> &'static str {
    LP_SEED_SOURCE_BRIDGED_NOTES
}

/// LP mint evidence hex fields (for logs / optional MintEvidence-shaped artifacts).
pub fn lp_mint_log_line(kind: &str, mint: &LpBridgeMintV0) -> String {
    format!(
        "lp_{kind}_mint value={} cm={} bridge_nf={} conservation={} proof_mode={}",
        mint.value,
        hex32(&mint.cm_public),
        hex32(&mint.bridge_nullifier),
        mint.conservation_class,
        mint.proof_mode
    )
}

// Re-export fixture constants for suite/binary.
pub use terp_seams::dex::{OMNI_LP_BTC as FIXTURE_LP_BTC, OMNI_LP_ZEC as FIXTURE_LP_ZEC};

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialize env-mutating tests (process env is global).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        keys: Vec<(&'static str, Option<String>)>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl EnvGuard {
        fn set(pairs: &[(&'static str, &str)]) -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let mut keys = Vec::new();
            for (k, v) in pairs {
                keys.push((*k, std::env::var(k).ok()));
                // # Safety: test-only env pin.
                unsafe {
                    std::env::set_var(k, v);
                }
            }
            Self { keys, _lock: lock }
        }
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
        /// Clear listed keys then set pairs (single mutex hold — avoid nested EnvGuard deadlock).
        fn clear_then_set(clear: &[&'static str], pairs: &[(&'static str, &str)]) -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let mut keys = Vec::new();
            for k in clear {
                keys.push((*k, std::env::var(k).ok()));
                unsafe {
                    std::env::remove_var(k);
                }
            }
            for (k, v) in pairs {
                if !keys.iter().any(|(kk, _)| kk == k) {
                    keys.push((*k, std::env::var(k).ok()));
                }
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
    fn bootstrap_reserves_match_lp_notes() {
        let _g = EnvGuard::clear(&[
            "CORRIDOR_PROFILE",
            ENV_CORRIDOR_POOL_SEED,
            "CORRIDOR_OMNI_REQUIRE_PRODUCT_GREEN",
            "CORRIDOR_DUAL_HOME_PREP_PATH",
        ]);
        let boot = bootstrap_omni_liquidity().expect("bootstrap");
        boot.assert_product_green().unwrap();
        assert_eq!(boot.r_btc, OMNI_LP_BTC);
        assert_eq!(boot.r_zec, OMNI_LP_ZEC);
        assert_eq!(boot.receipt.source, LP_SEED_SOURCE_BRIDGED_NOTES);
        assert_eq!(
            boot.liability.outstanding_zec_claims,
            boot.prep.lp_zec_zats
        );
        assert!(boot.prep.escrow_balance_zats >= boot.liability.outstanding_zec_claims);
    }

    #[test]
    fn refuse_magic_reserves_under_bridged_policy() {
        let _g = EnvGuard::clear_then_set(
            &[
                "CORRIDOR_OMNI_REQUIRE_PRODUCT_GREEN",
                "CORRIDOR_DUAL_HOME_PREP_PATH",
                "CORRIDOR_PROFILE",
            ],
            &[(ENV_CORRIDOR_POOL_SEED, POOL_SEED_BRIDGED_LP)],
        );
        let boot = bootstrap_omni_liquidity().unwrap();
        assert_ne!(
            (boot.r_btc, boot.r_zec),
            (LAB_R_IN, LAB_R_OUT),
            "fixture LP reserves must differ from magic lab constants"
        );
        let err = refuse_magic_pool_if_bridged_policy(LAB_R_IN, LAB_R_OUT, Some(&boot)).unwrap_err();
        match err {
            OmniLpError::MagicPoolForbidden | OmniLpError::ReservesMismatch { .. } => {}
            other => panic!("unexpected {other:?}"),
        }
        // Correct reserves from bootstrap OK
        refuse_magic_pool_if_bridged_policy(boot.r_btc, boot.r_zec, Some(&boot)).unwrap();
        // Missing bootstrap
        let err = refuse_magic_pool_if_bridged_policy(boot.r_btc, boot.r_zec, None).unwrap_err();
        assert!(matches!(err, OmniLpError::MissingReceipt));
    }

    #[test]
    fn resolve_pool_reserves_bridged() {
        let _g = EnvGuard::clear_then_set(
            &[
                "CORRIDOR_OMNI_REQUIRE_PRODUCT_GREEN",
                "CORRIDOR_DUAL_HOME_PREP_PATH",
            ],
            &[("CORRIDOR_PROFILE", "omni_production_shaped")],
        );
        let boot = bootstrap_omni_liquidity().unwrap();
        let (r_a, r_b, rec) = resolve_pool_reserves(Some(&boot)).unwrap();
        assert_eq!(r_a, OMNI_LP_BTC);
        assert_eq!(r_b, OMNI_LP_ZEC);
        assert!(rec.unwrap().is_bridged_notes());
        assert!(resolve_pool_reserves(None).is_err());
    }

    #[test]
    fn resolve_pool_reserves_magic_lab_default() {
        let _g = EnvGuard::clear(&["CORRIDOR_PROFILE", ENV_CORRIDOR_POOL_SEED]);
        let (r_a, r_b, rec) = resolve_pool_reserves(None).unwrap();
        assert_eq!((r_a, r_b), (LAB_R_IN, LAB_R_OUT));
        assert!(rec.is_none());
    }

    #[test]
    fn receipt_json_roundtrip() {
        let boot = bootstrap_omni_liquidity().unwrap();
        let path = std::env::temp_dir().join("corridor-lp-seed-test.json");
        boot.write_json(&path).unwrap();
        let s = fs::read_to_string(&path).unwrap();
        let doc: LpSeedReceiptJson = serde_json::from_str(&s).unwrap();
        assert_eq!(doc.lp_seed.source, LP_SEED_SOURCE_BRIDGED_NOTES);
        assert_eq!(doc.lp_seed.r_btc as u128, OMNI_LP_BTC);
        assert_eq!(doc.lp_seed.r_zec as u128, OMNI_LP_ZEC);
        assert_eq!(doc.dual_home.user_btc_sats as u128, OMNI_USER_BTC);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn escrow_underfund_fixture_fails_bootstrap() {
        let mut prep = DualHomePrepV0::omni_fixture();
        prep.escrow_balance_zats = 1; // underfund
        let err = bootstrap_omni_liquidity_with(prep, 1).unwrap_err();
        assert!(
            matches!(err, OmniLpError::Pure(LpSeedError::ErrEscrowUnderfunded))
                || err.to_string().contains("Escrow")
                || matches!(err, OmniLpError::Pure(_))
        );
    }

    /// Hook entry: bootstrap from dual-home prep env + write GUIDE §12 03–05 pack.
    ///
    /// Used by `e2e/hooks/omni-lp-seed.sh`. Soft path allows planned dual-home;
    /// product green env fails closed via resolve_dual_home_for_bootstrap.
    #[test]
    fn bootstrap_and_write_run_pack() {
        // Soft bootstrap inside the test; shell hook enforces product green after dual-home.
        let _g = EnvGuard::clear_then_set(
            &["CORRIDOR_OMNI_REQUIRE_PRODUCT_GREEN"],
            &[
                ("CORRIDOR_PROFILE", "omni_production_shaped"),
                (ENV_CORRIDOR_POOL_SEED, POOL_SEED_BRIDGED_LP),
            ],
        );
        let run_dir = std::env::var("CORRIDOR_OMNI_RUN_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::temp_dir().join(format!("omni-lp-pack-{}", std::process::id()))
            });
        let _ = fs::create_dir_all(&run_dir);

        // Ensure dual-home prep path points at run dir when shell set RUN_DIR only.
        // Paths may already be set by omni-lp-seed.sh — do not clobber.
        if std::env::var("CORRIDOR_DUAL_HOME_PREP_PATH").is_err() {
            unsafe {
                std::env::set_var(
                    "CORRIDOR_DUAL_HOME_PREP_PATH",
                    run_dir.join("dual-home-prep.json"),
                );
            }
        }
        if std::env::var("CORRIDOR_LP_SEED_RECEIPT_PATH").is_err() {
            unsafe {
                std::env::set_var(
                    "CORRIDOR_LP_SEED_RECEIPT_PATH",
                    run_dir.join("corridor-lp-seed-receipt.json"),
                );
            }
        }

        let multi_net = crate::harness::omni_dual_home::load_dual_home_prep(
            &crate::harness::omni_dual_home::dual_home_prep_path(),
        )
        .ok()
        .map(|m| m.is_product_dual_home_green())
        .unwrap_or(false);

        let boot = bootstrap_omni_liquidity().expect("bootstrap_omni_liquidity");
        boot.assert_product_green().expect("bridged_notes product shape");
        boot.write_omni_run_pack(&run_dir, multi_net)
            .expect("write_omni_run_pack");

        assert!(run_dir.join("05-lp-seed-pool.json").is_file());
        assert!(run_dir.join("03-lp-mint-btc.json").is_file());
        assert!(run_dir.join("04-lp-mint-zec.json").is_file());
        let seed: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(run_dir.join("05-lp-seed-pool.json")).unwrap())
                .unwrap();
        assert_eq!(seed["lp_seed"]["source"], "bridged_notes");
        assert_eq!(
            seed["lp_seed"]["r_btc"].as_u64().unwrap(),
            boot.r_btc as u64
        );
        assert_eq!(
            seed["lp_seed"]["r_zec"].as_u64().unwrap(),
            boot.r_zec as u64
        );
        assert_eq!(seed["multi_net"], multi_net);
        println!(
            "  bootstrap_and_write_run_pack OK multi_net={} run_dir={}",
            multi_net,
            run_dir.display()
        );
    }
}
