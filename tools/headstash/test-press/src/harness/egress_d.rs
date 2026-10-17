//! Option D continuous corridor: settle → egress burn → lab Zcash pay → dual receipts.
//!
//! SSOT: `DESIGN-ZEC-EGRESS-D.md`, PROMPT-IMPL-HARNESS-D.
//!
//! Env:
//! - `CORRIDOR_ZEC_EGRESS_D=1` — enable after G3 settle (binary / shell)
//! - `CORRIDOR_EGRESS_BURN_EVIDENCE_PATH` — default `/tmp/corridor-egress-burn-evidence.json`
//! - `CORRIDOR_ZEC_EGRESS_RECEIPT_PATH` — default `/tmp/corridor-zec-egress-receipt.json`
//!
//! When CW `BridgeEgressBurn` is not yet on chain (CW-EGRESS residual), Terp burn is
//! recorded via pure `apply_egress_burn` and labeled `pure_record_lab` / `mock_verify_lab`.
//! Dest seal equality is **always** fail-closed.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use terp_seams::dex::{
    DestKind as PureDestKind, EGRESS_NF_LABEL, EgressBurnEvidenceV0 as PureEvidence,
    EgressBurnPublic, EgressError, EgressSeamState, SettleLikeOpening, SwapActionV0,
    ZecEgressReceiptV0 as PureZecReceipt, apply_egress_burn, build_egress_burn_from_settle,
    egress_nullifier,
};

use super::lab_pay_zec::{
    LabPayError, OpenConfirmResult, confirm_open_at_sealed_dest_with_cfg,
    egress_burn_evidence_path, lab_pay_zec_after_burn as lab_pay_zec_host, zec_egress_receipt_path,
};
// Pay mode labels + dest_kind SSOT: lab_pay_zec (ZAKURA-PAY).
pub use super::lab_pay_zec::{
    MODE_LAB_INVENTORY_PAY, MODE_LAB_INVENTORY_PAY_SIMULATED, confirm_open_at_sealed_dest,
    dest_kind_from_display, wallet_rpc_available,
};
use super::mint_evidence::{MintEvidenceError, SettleReceiptV0, decode_hex32, hex32};
use super::swap_statement_cw::{SwapSpendHandoffV0, lab_asset_out_zec};
use super::zakura_local::{
    SealedDestSource, SealedDestV0, ZakuraLocalConfig, assert_dest_binding_equal,
    owner_binding_from_dest_display, rpc_ready, seal_funded_dest,
};

/// Env gate for Option D stage after settle.
pub const ENV_CORRIDOR_ZEC_EGRESS_D: &str = "CORRIDOR_ZEC_EGRESS_D";

/// Labeled residual: pure burn record (CW BridgeEgressBurn not on wasm yet).
pub const BURN_SURFACE_PURE_RECORD_LAB: &str = "pure_record_lab";
/// Future: CW BridgeEgressBurn on headstash / private-dex.
pub const BURN_SURFACE_CW_BRIDGE_EGRESS: &str = "cw_bridge_egress_burn";

// ── Serde artifacts (JSON on disk; mirror pure normative fields as hex) ─────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EgressBurnPublicJson {
    pub asset_id_hex: String,
    pub value: u64,
    pub cm_spent_hex: String,
    pub nullifier_hex: String,
    pub dest_commitment_hex: String,
    pub dest_kind: String,
    pub root_hex: String,
    pub source_pool_id: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EgressBurnEvidenceJson {
    pub schema: String,
    pub profile: String,
    pub stage: String,
    pub status: String,
    /// `pure_record_lab` | `cw_bridge_egress_burn`
    pub burn_surface: String,
    pub terp_tx_hash: Option<String>,
    pub burn: EgressBurnPublicJson,
    /// `mock_verify_lab` | `zk_api`
    pub proof_mode: String,
    pub settle_receipt_ref: Option<String>,
    pub settle_receipt_path: Option<String>,
    pub chain_id: Option<String>,
    pub headstash_contract: Option<String>,
    pub private_dex_contract: Option<String>,
    pub intent_id: Option<String>,
    /// Continuity: egress ν domain label (must be egress-nf-v0).
    pub egress_nf_label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ZecEgressReceiptJson {
    pub schema: String,
    pub profile: String,
    pub stage: String,
    pub status: String,
    pub dest_display: String,
    pub dest_owner_binding_hex: String,
    pub dest_kind: String,
    pub zec_txid: Option<String>,
    pub amount_zat: u64,
    /// `lab_inventory_pay` | `lab_inventory_pay_simulated` | `lc_mint` | ...
    pub mode: String,
    pub burn_nullifier_hex: String,
    pub burn_evidence_path: Option<String>,
    pub sealed_source: Option<String>,
    /// `hashmerchant_live` | `in_process_lab` when threshold path ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committee_crypto: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custody_label: Option<String>,
    /// Always false for lab multi-sig path (FROST not claimed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frost: Option<bool>,
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum EgressDError {
    #[error("egress d: {0}")]
    Msg(String),
    #[error("dest seal mismatch: {0}")]
    DestMismatch(String),
    #[error("pure egress: {0:?}")]
    Pure(EgressError),
    #[error("mint evidence: {0}")]
    Mint(#[from] MintEvidenceError),
    #[error("io: {0}")]
    Io(String),
    #[error("json: {0}")]
    Json(String),
}

impl From<EgressError> for EgressDError {
    fn from(e: EgressError) -> Self {
        Self::Pure(e)
    }
}

impl From<LabPayError> for EgressDError {
    fn from(e: LabPayError) -> Self {
        match e {
            LabPayError::DestMismatch { evidence, sealed } => {
                Self::DestMismatch(format!("evidence={evidence} sealed={sealed}"))
            }
            other => Self::Msg(other.to_string()),
        }
    }
}

fn env_truthy(key: &str) -> bool {
    matches!(
        std::env::var(key).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    )
}

/// Whether Option D stage is enabled.
pub fn zec_egress_d_enabled() -> bool {
    env_truthy(ENV_CORRIDOR_ZEC_EGRESS_D)
}

fn dest_kind_wire(k: PureDestKind) -> &'static str {
    match k {
        PureDestKind::Transparent => "transparent",
        PureDestKind::Shielded => "shielded",
    }
}

fn public_to_json(p: &EgressBurnPublic) -> EgressBurnPublicJson {
    EgressBurnPublicJson {
        asset_id_hex: hex32(&p.asset_id),
        value: p.value,
        cm_spent_hex: hex32(&p.cm_spent),
        nullifier_hex: hex32(&p.nullifier),
        dest_commitment_hex: hex32(&p.dest_commitment),
        dest_kind: dest_kind_wire(p.dest_kind).into(),
        root_hex: hex32(&p.root),
        source_pool_id: p.source_pool_id,
    }
}

fn pure_evidence_to_json(
    evidence: &PureEvidence,
    burn_surface: &str,
    settle: Option<&SettleReceiptV0>,
    settle_path: Option<&Path>,
    profile: &str,
) -> EgressBurnEvidenceJson {
    EgressBurnEvidenceJson {
        schema: "EgressBurnEvidenceV0".into(),
        profile: profile.into(),
        stage: "option_d_egress_burn".into(),
        status: "complete".into(),
        burn_surface: burn_surface.into(),
        terp_tx_hash: evidence.terp_tx_hash.clone(),
        burn: public_to_json(&evidence.burn),
        proof_mode: evidence.proof_mode.clone(),
        settle_receipt_ref: evidence
            .settle_receipt_ref
            .clone()
            .or_else(|| settle.map(|s| format!("pool_id={};status={}", s.pool_id, s.status))),
        settle_receipt_path: settle_path.map(|p| p.display().to_string()),
        chain_id: settle.map(|s| s.chain_id.clone()),
        headstash_contract: settle.map(|s| s.headstash_contract.clone()),
        private_dex_contract: settle.map(|s| s.private_dex_contract.clone()),
        intent_id: settle.and_then(|s| s.intent_id.clone()),
        egress_nf_label: String::from_utf8_lossy(EGRESS_NF_LABEL).into_owned(),
    }
}

fn pure_zec_to_json(
    r: &PureZecReceipt,
    profile: &str,
    evidence_path: Option<&Path>,
    sealed_source: Option<&str>,
) -> ZecEgressReceiptJson {
    let meta = super::lab_pay_zec::peek_last_threshold_auth_meta();
    ZecEgressReceiptJson {
        schema: "ZecEgressReceiptV0".into(),
        profile: profile.into(),
        stage: "option_d_zec_lab_pay".into(),
        status: "complete".into(),
        dest_display: r.dest_display.clone(),
        dest_owner_binding_hex: r.dest_owner_binding_hex.clone(),
        dest_kind: dest_kind_wire(r.dest_kind).into(),
        zec_txid: r.zec_txid.clone(),
        amount_zat: r.amount_zat,
        mode: r.mode.clone(),
        burn_nullifier_hex: r.burn_nullifier_hex.clone(),
        burn_evidence_path: evidence_path.map(|p| p.display().to_string()),
        sealed_source: sealed_source.map(|s| s.to_string()),
        auth_source: meta.as_ref().map(|m| m.auth_source.clone()),
        committee_crypto: meta.as_ref().map(|m| m.committee_crypto.clone()),
        custody_label: meta.as_ref().map(|m| m.custody_label.clone()),
        frost: Some(false),
    }
}

impl EgressBurnEvidenceJson {
    pub fn write_json(&self, path: &Path) -> Result<(), EgressDError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| EgressDError::Io(e.to_string()))?;
        }
        let s =
            serde_json::to_string_pretty(self).map_err(|e| EgressDError::Json(e.to_string()))?;
        fs::write(path, s).map_err(|e| EgressDError::Io(e.to_string()))
    }

    pub fn read_json(path: &Path) -> Result<Self, EgressDError> {
        let s = fs::read_to_string(path).map_err(|e| EgressDError::Io(e.to_string()))?;
        serde_json::from_str(&s).map_err(|e| EgressDError::Json(e.to_string()))
    }
}

impl ZecEgressReceiptJson {
    pub fn write_json(&self, path: &Path) -> Result<(), EgressDError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| EgressDError::Io(e.to_string()))?;
        }
        let s =
            serde_json::to_string_pretty(self).map_err(|e| EgressDError::Json(e.to_string()))?;
        fs::write(path, s).map_err(|e| EgressDError::Io(e.to_string()))
    }

    pub fn read_json(path: &Path) -> Result<Self, EgressDError> {
        let s = fs::read_to_string(path).map_err(|e| EgressDError::Io(e.to_string()))?;
        serde_json::from_str(&s).map_err(|e| EgressDError::Json(e.to_string()))
    }
}

/// Resolve G4 sealed dest for Option D after settle.
///
/// 1. Prefer `seal_funded_dest` when it equals settle `note_out.owner_binding` (continuous G4).
/// 2. Env `ZAKURA_DEST_ADDR` / `ZAKURA_DEST_DISPLAY` when digest equals openings.
/// 3. Env `CORRIDOR_DEST_OWNER_BINDING` pin equal to openings (display from env if present).
/// 4. **Fail-closed** when G4/env golden is available but openings differ — do **not** invent
///    `demo_zec_mint_seal_*` with a divergent binding story on the product funded path.
/// 5. Residual invent display only when no G4/env seal is resolvable (unit offline edge).
///
/// Fail-closed if note_out.owner_binding is placeholder / zero.
pub fn sealed_dest_for_option_d(
    handoff: &SwapSpendHandoffV0,
    cfg: &ZakuraLocalConfig,
) -> Result<SealedDestV0, EgressDError> {
    let out_owner = handoff.action.witness.note_out.owner_binding;
    if out_owner == [0u8; 32] {
        return Err(EgressDError::Msg(
            "note_out.owner_binding zero — refuse Option D seal".into(),
        ));
    }
    let out_hex = hex32(&out_owner);
    if crate::harness::zakura_local::is_placeholder_owner_binding_hex(&out_hex) {
        return Err(EgressDError::Msg(format!(
            "placeholder note_out dest seal rejected: {out_hex}"
        )));
    }

    // Product / funded: G4 golden or env pin must match settle openings (single seal e2e).
    let g4_result = seal_funded_dest(cfg);
    match &g4_result {
        Ok(g4) if g4.owner_binding == out_owner => {
            return Ok(g4.clone());
        }
        Ok(g4) => {
            // G4 resolvable but openings diverge → refuse invent residual (P0 single seal).
            // Happy-fixture residual must pin mint dest to G4 before settle (see corridor_ict_funded).
            return Err(EgressDError::DestMismatch(format!(
                "settle note_out.owner_binding={out_hex} ≠ G4 seal {} (source={}) — refuse demo_zec_mint_seal invent; fix mint dest continuity",
                g4.owner_binding_hex,
                g4.source.as_wire_str()
            )));
        }
        Err(e) => {
            eprintln!("  WARN: seal_funded_dest: {e} — trying env display / pin continuity");
        }
    }

    // Env dest display whose digest equals openings (operator continuous path).
    if let Ok(addr) = std::env::var("ZAKURA_DEST_ADDR") {
        let addr = addr.trim().to_string();
        if !addr.is_empty() && owner_binding_from_dest_display(&addr) == out_owner {
            return Ok(SealedDestV0 {
                dest_display: addr,
                owner_binding: out_owner,
                owner_binding_hex: out_hex.clone(),
                source: SealedDestSource::EnvOverride,
                rpc_ready: rpc_ready(cfg),
                rpc_validated: false,
            });
        }
    }
    if let Ok(addr) = std::env::var("ZAKURA_DEST_DISPLAY") {
        let addr = addr.trim().to_string();
        if !addr.is_empty() && owner_binding_from_dest_display(&addr) == out_owner {
            return Ok(SealedDestV0 {
                dest_display: addr,
                owner_binding: out_owner,
                owner_binding_hex: out_hex.clone(),
                source: SealedDestSource::EnvOverride,
                rpc_ready: rpc_ready(cfg),
                rpc_validated: false,
            });
        }
    }

    // Explicit pin matches openings (shell exported G4 without seal_funded_dest path).
    if let Ok(pin) = std::env::var("CORRIDOR_DEST_OWNER_BINDING") {
        let pin = pin.trim().to_ascii_lowercase();
        if !pin.is_empty() {
            if crate::harness::zakura_local::is_placeholder_owner_binding_hex(&pin) {
                return Err(EgressDError::Msg(
                    "CORRIDOR_DEST_OWNER_BINDING is placeholder — refuse Option D".into(),
                ));
            }
            if pin != out_hex {
                return Err(EgressDError::DestMismatch(format!(
                    "CORRIDOR_DEST_OWNER_BINDING={pin} ≠ note_out.owner_binding={out_hex}"
                )));
            }
            let dest_display = std::env::var("ZAKURA_DEST_DISPLAY")
                .or_else(|_| std::env::var("ZAKURA_DEST_ADDR"))
                .unwrap_or_else(|_| format!("bound_dest_{}", &out_hex[..16.min(out_hex.len())]));
            return Ok(SealedDestV0 {
                dest_display,
                owner_binding: out_owner,
                owner_binding_hex: out_hex,
                source: SealedDestSource::EnvOverride,
                rpc_ready: rpc_ready(cfg),
                rpc_validated: false,
            });
        }
    }

    // No G4/env available — last-ditch residual (unit offline only). Labeled invent display.
    let dest_display = format!("demo_zec_mint_seal_{}", &out_hex[..16.min(out_hex.len())]);
    eprintln!(
        "  WARN: sealed_dest residual invent display={} (no G4/env) binding={}",
        dest_display, out_hex
    );
    Ok(SealedDestV0 {
        dest_display,
        owner_binding: out_owner,
        owner_binding_hex: out_hex,
        source: SealedDestSource::UiPaste,
        rpc_ready: rpc_ready(cfg),
        rpc_validated: false,
    })
}

/// Build settle-like opening from post-settle swap handoff (ZEC `note_out`).
///
/// Fail-closed if:
/// - sealed dest ≠ note_out.owner_binding
/// - cm_out[0] continuity broken
/// - value 0
pub fn settle_opening_from_handoff(
    handoff: &SwapSpendHandoffV0,
    sealed: &SealedDestV0,
) -> Result<SettleLikeOpening, EgressDError> {
    let note_out = &handoff.action.witness.note_out;
    if note_out.value == 0 {
        return Err(EgressDError::Msg("note_out value=0 — cannot egress".into()));
    }

    // Fail-closed: G4 seal must equal SEAM out owner (product corridor).
    let out_hex = hex32(&note_out.owner_binding);
    assert_dest_binding_equal(&sealed.owner_binding_hex, &out_hex, "settle.note_out.owner")
        .map_err(|e| EgressDError::DestMismatch(e.to_string()))?;

    // cm continuity: public cm_out[0] == note_out.cm_public
    let cm0 = handoff
        .action
        .public
        .cm_out
        .first()
        .copied()
        .ok_or_else(|| EgressDError::Msg("cm_out empty after settle".into()))?;
    if cm0 != note_out.cm_public {
        return Err(EgressDError::Msg(
            "cm_out[0] ≠ note_out.cm_public (settle continuity)".into(),
        ));
    }

    // Asset out should be lab ZEC stand-in (sim-ZEC); soft check.
    let expected_out = lab_asset_out_zec();
    if note_out.asset_id != expected_out && note_out.asset_id != handoff.action.public.asset_out {
        return Err(EgressDError::Msg(
            "note_out asset_id ≠ statement asset_out".into(),
        ));
    }

    let dest_kind = dest_kind_from_display(&sealed.dest_display);
    Ok(SettleLikeOpening {
        asset_id: note_out.asset_id,
        value: note_out.value,
        cm: note_out.cm_public,
        rcm: note_out.rcm,
        dest_seal: sealed.owner_binding,
        dest_kind,
        root: handoff.action.public.root,
        source_pool_id: Some(handoff.statement.pool_id),
        path_position: 0,
    })
}

/// Map settle opening → CW `EgressBurnStatement` (headstash BridgeEgressBurn).
///
/// Available when `interface` feature pulls `cw-headstash`. Callers execute on suite.
#[cfg(feature = "interface")]
pub fn cw_egress_statement_from_opening(
    opening: &SettleLikeOpening,
) -> Result<cw_headstash::egress::EgressBurnStatement, EgressDError> {
    let burn = build_egress_burn_from_settle(opening).map_err(EgressDError::from)?;
    let dest_kind = match burn.public.dest_kind {
        PureDestKind::Transparent => cw_headstash::egress::DestKind::Transparent,
        PureDestKind::Shielded => cw_headstash::egress::DestKind::Shielded,
    };
    Ok(cw_headstash::egress::EgressBurnStatement {
        asset_id: cosmwasm_std::Binary::from(burn.public.asset_id.to_vec()),
        value: burn.public.value,
        cm_spent: cosmwasm_std::Binary::from(burn.public.cm_spent.to_vec()),
        nullifier: cosmwasm_std::Binary::from(burn.public.nullifier.to_vec()),
        dest_commitment: cosmwasm_std::Binary::from(burn.public.dest_commitment.to_vec()),
        dest_kind,
        root: cosmwasm_std::Binary::from(burn.public.root.to_vec()),
        source_pool_id: burn.public.source_pool_id,
        rcm: cosmwasm_std::Binary::from(burn.witness.rcm.to_vec()),
        owner_binding: cosmwasm_std::Binary::from(burn.witness.owner_binding.to_vec()),
    })
}

/// Lab mock egress proof (non-empty; matches contract mock_verify gate).
#[cfg(feature = "interface")]
pub fn lab_mock_egress_proof() -> cosmwasm_std::Binary {
    // Prefer contract fixture when linked; keep identical spirit.
    cw_headstash::egress::mock_egress_proof_bytes()
}

/// Apply pure egress burn (labeled residual when CW msg unavailable).
///
/// Always fail-closed on dest ≠ G4 seal.
pub fn apply_pure_egress_burn_labeled(
    opening: &SettleLikeOpening,
    sealed: &SealedDestV0,
    settle_receipt_ref: Option<String>,
) -> Result<(PureEvidence, String), EgressDError> {
    // Explicit dest equality before pure apply (belt + suspenders).
    if opening.dest_seal != sealed.owner_binding {
        return Err(EgressDError::DestMismatch(format!(
            "opening.dest_seal {} ≠ sealed {}",
            hex32(&opening.dest_seal),
            sealed.owner_binding_hex
        )));
    }

    let burn = build_egress_burn_from_settle(opening).map_err(EgressDError::from)?;
    // Domain assert
    let derived = egress_nullifier(&burn.public.cm_spent, &burn.witness.rcm);
    if derived != burn.public.nullifier {
        return Err(EgressDError::Msg("egress ν re-derive failed".into()));
    }

    let mut state = EgressSeamState {
        allowed_root: Some(opening.root),
        ..Default::default()
    };
    let mut evidence = apply_egress_burn(
        &mut state,
        &burn.public,
        &burn.witness,
        Some(&sealed.owner_binding),
    )
    .map_err(EgressDError::from)?;
    evidence.settle_receipt_ref = settle_receipt_ref;
    // Labeled residual: not on-chain CW
    Ok((evidence, BURN_SURFACE_PURE_RECORD_LAB.to_string()))
}

/// Lab Zcash pay **only after** burn evidence; fail-closed on dest mismatch / empty evidence.
///
/// Delegates to ZAKURA-PAY host attach (`lab_pay_zec`): offline synthetic
/// `lab_inventory_pay_simulated`, live `sendtoaddress`/`z_sendmany` with skip-clean degrade.
pub fn lab_pay_zec_after_burn(
    evidence: &PureEvidence,
    sealed: &SealedDestV0,
) -> Result<PureZecReceipt, EgressDError> {
    lab_pay_zec_host(evidence, sealed).map_err(EgressDError::from)
}

/// Full Option D stage after successful settle: burn → lab pay → write dual artifacts.
#[derive(Clone, Debug)]
pub struct EgressDStageOutcome {
    pub evidence: EgressBurnEvidenceJson,
    pub zec_receipt: ZecEgressReceiptJson,
    pub evidence_path: PathBuf,
    pub zec_receipt_path: PathBuf,
    pub burn_surface: String,
    pub pure_evidence: PureEvidence,
    /// Open/confirm at sealed dest when RPC/wallet allow; None if probe hard-failed.
    pub open_confirm: Option<OpenConfirmResult>,
}

/// Inputs for continuous corridor stage.
pub struct EgressDStageInputs<'a> {
    pub handoff: &'a SwapSpendHandoffV0,
    pub settle: &'a SettleReceiptV0,
    pub sealed: &'a SealedDestV0,
    pub settle_receipt_path: Option<&'a Path>,
    pub profile: &'a str,
}

pub fn run_option_d_after_settle(
    inputs: EgressDStageInputs<'_>,
) -> Result<EgressDStageOutcome, EgressDError> {
    if inputs.settle.status != "complete" {
        return Err(EgressDError::Msg(format!(
            "settle status={} — refuse Option D (need complete)",
            inputs.settle.status
        )));
    }
    if inputs.settle.cm_out_hex.is_empty() {
        return Err(EgressDError::Msg(
            "settle cm_out_hex empty — refuse Option D".into(),
        ));
    }

    // Continuity: handoff cm_out[0] hex matches settle receipt.
    let cm0_hex = hex32(
        inputs
            .handoff
            .action
            .public
            .cm_out
            .first()
            .ok_or_else(|| EgressDError::Msg("handoff cm_out empty".into()))?,
    );
    let settle_cm0 = inputs
        .settle
        .cm_out_hex
        .first()
        .ok_or_else(|| EgressDError::Msg("settle cm_out_hex empty".into()))?;
    if cm0_hex != settle_cm0.trim().to_lowercase()
        && cm0_hex != settle_cm0.trim().to_lowercase().trim_start_matches("0x")
    {
        // Allow settle hex with or without 0x; normalize both.
        let a = cm0_hex.trim().trim_start_matches("0x").to_lowercase();
        let b = settle_cm0.trim().trim_start_matches("0x").to_lowercase();
        if a != b {
            return Err(EgressDError::Msg(format!(
                "handoff cm_out[0]={a} ≠ settle cm_out[0]={b}"
            )));
        }
    }

    let opening = settle_opening_from_handoff(inputs.handoff, inputs.sealed)?;
    let settle_ref = Some(format!(
        "pool_id={};delta_out={};cm0={}",
        inputs.settle.pool_id, inputs.settle.delta_r_out, settle_cm0
    ));

    // CW path residual: no BridgeEgressBurn on wasm → pure labeled record.
    let (pure_evidence, burn_surface) =
        apply_pure_egress_burn_labeled(&opening, inputs.sealed, settle_ref)?;

    // Double-check receipt cm was the one burned
    let burned_cm = hex32(&pure_evidence.burn.cm_spent);
    let b = settle_cm0.trim().trim_start_matches("0x").to_lowercase();
    if burned_cm != b {
        return Err(EgressDError::Msg(format!(
            "burned cm {burned_cm} ≠ settle cm_out[0] {b}"
        )));
    }

    let zec = lab_pay_zec_after_burn(&pure_evidence, inputs.sealed)?;

    // Open/confirm at sealed dest (skip-clean when RPC/wallet residual).
    let zcfg = ZakuraLocalConfig::default();
    let open_confirm: Option<OpenConfirmResult> =
        match confirm_open_at_sealed_dest_with_cfg(inputs.sealed, &zcfg) {
            Ok(oc) => Some(oc),
            Err(_) => None,
        };

    let evidence_path = egress_burn_evidence_path();
    let zec_path = zec_egress_receipt_path();

    let evidence_json = pure_evidence_to_json(
        &pure_evidence,
        &burn_surface,
        Some(inputs.settle),
        inputs.settle_receipt_path,
        inputs.profile,
    );
    evidence_json.write_json(&evidence_path)?;

    let zec_json = pure_zec_to_json(
        &zec,
        inputs.profile,
        Some(&evidence_path),
        Some(inputs.sealed.source.as_wire_str()),
    );
    zec_json.write_json(&zec_path)?;

    Ok(EgressDStageOutcome {
        evidence: evidence_json,
        zec_receipt: zec_json,
        evidence_path,
        zec_receipt_path: zec_path,
        burn_surface,
        pure_evidence,
        open_confirm,
    })
}

/// RECOVERY-5 / lab degrade SSOT: pure_record_lab + lab_inventory_pay_simulated
/// are **labeled residual**, never product chain/ZEC consensus success.
///
/// Returns `Ok(())` when labels are honest. Errors if mislabeled as CW/product
/// success aliases or empty.
pub fn assert_lab_degrade_labels(burn_surface: &str, pay_mode: &str) -> Result<(), EgressDError> {
    if burn_surface.trim().is_empty() || pay_mode.trim().is_empty() {
        return Err(EgressDError::Msg(
            "lab degrade labels empty — refuse silent product success".into(),
        ));
    }
    // Forbidden product-success aliases (UI copy / status strings).
    let forbidden = [
        "chain_egress_complete",
        "zec_sent",
        "mainnet_complete",
        "egress complete (chain)",
        "ZEC sent",
    ];
    for f in forbidden {
        if burn_surface.eq_ignore_ascii_case(f) || pay_mode.eq_ignore_ascii_case(f) {
            return Err(EgressDError::Msg(format!(
                "product success alias forbidden for lab residual: {f}"
            )));
        }
    }
    // SSOT label width checks: residual modes must keep their exact strings.
    if burn_surface == BURN_SURFACE_PURE_RECORD_LAB {
        debug_assert_ne!(BURN_SURFACE_PURE_RECORD_LAB, BURN_SURFACE_CW_BRIDGE_EGRESS);
    }
    if pay_mode == MODE_LAB_INVENTORY_PAY_SIMULATED {
        debug_assert_ne!(MODE_LAB_INVENTORY_PAY_SIMULATED, MODE_LAB_INVENTORY_PAY);
        if !pay_mode.contains("simulated") {
            return Err(EgressDError::Msg(
                "lab_inventory_pay_simulated must contain 'simulated'".into(),
            ));
        }
    }
    Ok(())
}

/// True when residual labels forbid product UI celebration (R3/R5).
///
/// Threshold product modes (`threshold_escrow_release[_simulated]`) are allowed
/// to celebrate architecture (still honesty-labeled for mock proof / P-lab).
/// Inventory funder modes always forbid product celebration.
pub fn product_celebrate_forbidden(burn_surface: &str, pay_mode: &str) -> bool {
    burn_surface == BURN_SURFACE_PURE_RECORD_LAB
        || pay_mode == MODE_LAB_INVENTORY_PAY_SIMULATED
        || reject_funder_only_as_product(pay_mode)
}

/// Refuse pay without burn evidence (product bar) — unit-test helper.
pub fn refuse_pay_without_evidence(sealed: &SealedDestV0) -> Result<(), EgressDError> {
    let empty = PureEvidence {
        terp_tx_hash: None,
        burn: EgressBurnPublic {
            asset_id: [1u8; 32],
            value: 0,
            cm_spent: [0u8; 32],
            nullifier: [0u8; 32],
            dest_commitment: sealed.owner_binding,
            dest_kind: PureDestKind::Transparent,
            root: [7u8; 32],
            source_pool_id: None,
        },
        proof_mode: "mock_verify_lab".into(),
        settle_receipt_ref: None,
    };
    match lab_pay_zec_after_burn(&empty, sealed) {
        Ok(_) => Err(EgressDError::Msg(
            "lab_pay accepted empty evidence — fail-closed broken".into(),
        )),
        Err(_) => Ok(()),
    }
}

// Silence unused import when SwapActionV0 only used in docs.
#[allow(dead_code)]
fn _type_check_action(_a: &SwapActionV0) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::mint_evidence::mint_evidence_from_claim_fields;
    use crate::harness::swap_statement_cw::{ProofModeLabel, build_swap_spend_handoff_from_mint};
    use crate::harness::zakura_local::{
        REGTEST_MINER_DEST, REGTEST_MINER_OWNER_BINDING_HEX, SealedDestSource,
        owner_binding_from_dest_display, primary_golden_dest,
    };

    fn fixture_mint() -> crate::harness::MintEvidenceV0 {
        let mut asset_in = [0u8; 32];
        asset_in[0] = b'H';
        asset_in[1] = b'U';
        asset_in[2] = b'B';
        let dest = owner_binding_from_dest_display(REGTEST_MINER_DEST);
        mint_evidence_from_claim_fields(
            "ict_local_funded",
            "test-1",
            "terp1hs",
            &[0x11; 32],
            &[0x22; 32],
            5_000,
            &asset_in,
            Some(&[0x33; 32]),
            Some(&dest),
            true,
            None,
            Some("intent-d".into()),
        )
    }

    fn sealed_golden() -> SealedDestV0 {
        let g = primary_golden_dest().expect("golden");
        SealedDestV0 {
            dest_display: g.dest_display,
            owner_binding: g.owner_binding,
            owner_binding_hex: g.owner_binding_hex,
            source: SealedDestSource::GoldenPrimary,
            rpc_ready: false,
            rpc_validated: false,
        }
    }

    fn sample_settle(handoff: &SwapSpendHandoffV0) -> SettleReceiptV0 {
        SettleReceiptV0 {
            profile: "ict_local_funded".into(),
            stage: "chain_settle_swap".into(),
            status: "complete".into(),
            chain_id: "test-1".into(),
            headstash_contract: "terp1hs".into(),
            private_dex_contract: "terp1dex".into(),
            pool_id: handoff.statement.pool_id,
            delta_r_in: handoff.statement.delta_r_in.to_string(),
            delta_r_out: handoff.statement.delta_r_out.to_string(),
            r_in_after: "0".into(),
            r_out_after: "0".into(),
            nullifiers_hex: handoff.pool_nullifiers_hex.clone(),
            cm_out_hex: handoff
                .action
                .public
                .cm_out
                .iter()
                .map(|c| hex::encode(c))
                .collect(),
            bridge_nullifier_hex: handoff.mint.bridge_nullifier_hex.clone(),
            mint_cm_public_hex: handoff.mint.cm_public_hex.clone(),
            proof_mode: "mock_verify_lab".into(),
            mock_verify_dex: true,
            intent_id: handoff.mint.intent_id.clone(),
            settle_tx_hash: None,
            halo2_swap: false,
            skip_ibc_post_swap: false,
        }
    }

    #[test]
    fn option_d_after_settle_happy_pure_labeled() {
        // Pin residual inventory so ambient omni/product profile cannot flaky-fail.
        use crate::harness::lab_pay_zec::{
            ENV_CORRIDOR_ALLOW_LAB_INVENTORY_RESIDUAL, ENV_CORRIDOR_PROFILE,
            ENV_CORRIDOR_ZEC_RELEASE,
        };
        let prev_release = std::env::var(ENV_CORRIDOR_ZEC_RELEASE).ok();
        let prev_profile = std::env::var(ENV_CORRIDOR_PROFILE).ok();
        let prev_residual = std::env::var(ENV_CORRIDOR_ALLOW_LAB_INVENTORY_RESIDUAL).ok();
        // # Safety: test-only process env pin; restored below.
        unsafe {
            std::env::set_var(ENV_CORRIDOR_ZEC_RELEASE, "lab_inventory");
            std::env::set_var(ENV_CORRIDOR_PROFILE, "");
            std::env::set_var(ENV_CORRIDOR_ALLOW_LAB_INVENTORY_RESIDUAL, "1");
            std::env::set_var("CORRIDOR_LAB_ZEC_FORCE_SIMULATED", "1");
        }

        let mint = fixture_mint();
        let handoff =
            build_swap_spend_handoff_from_mint(&mint, ProofModeLabel::MockVerifyLab).unwrap();
        let sealed = sealed_golden();
        // Ensure mint dest matches golden (fixture uses REGTEST_MINER_DEST)
        assert_eq!(
            hex32(&handoff.action.witness.note_out.owner_binding),
            sealed.owner_binding_hex
        );
        let settle = sample_settle(&handoff);

        let dir = std::env::temp_dir().join(format!("corridor-egress-d-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // # Safety: test-only env pin for artifact paths; cleaned in same test.
        unsafe {
            std::env::set_var(
                "CORRIDOR_EGRESS_BURN_EVIDENCE_PATH",
                dir.join("evidence.json").to_str().unwrap(),
            );
            std::env::set_var(
                "CORRIDOR_ZEC_EGRESS_RECEIPT_PATH",
                dir.join("zec.json").to_str().unwrap(),
            );
        }

        let out = run_option_d_after_settle(EgressDStageInputs {
            handoff: &handoff,
            settle: &settle,
            sealed: &sealed,
            settle_receipt_path: None,
            profile: "ict_local_funded",
        })
        .expect("option d");

        assert_eq!(out.burn_surface, BURN_SURFACE_PURE_RECORD_LAB);
        assert_eq!(out.evidence.status, "complete");
        assert_eq!(out.evidence.burn_surface, BURN_SURFACE_PURE_RECORD_LAB);
        assert_eq!(out.evidence.egress_nf_label, "egress-nf-v0");
        assert_eq!(out.evidence.proof_mode, "mock_verify_lab");
        assert_eq!(out.zec_receipt.status, "complete");
        assert_eq!(out.zec_receipt.mode, MODE_LAB_INVENTORY_PAY_SIMULATED);
        // RECOVERY-5: labels are residual, not product celebrate
        assert_lab_degrade_labels(&out.burn_surface, &out.zec_receipt.mode).unwrap();
        assert!(product_celebrate_forbidden(
            &out.burn_surface,
            &out.zec_receipt.mode
        ));
        assert_eq!(
            out.zec_receipt.dest_owner_binding_hex,
            sealed.owner_binding_hex
        );
        assert!(out.zec_receipt.zec_txid.is_some());
        assert_eq!(
            out.zec_receipt.amount_zat,
            handoff.action.witness.note_out.value
        );
        // Open/confirm is skip-clean when RPC down (default offline CI).
        if let Some(ref oc) = out.open_confirm {
            assert_eq!(oc.dest_owner_binding_hex, sealed.owner_binding_hex);
            eprintln!("option_d open_confirm mode={} note={}", oc.mode, oc.note);
        }
        // Roundtrip
        let e2 = EgressBurnEvidenceJson::read_json(&out.evidence_path).unwrap();
        assert_eq!(e2.burn.nullifier_hex, out.evidence.burn.nullifier_hex);
        let z2 = ZecEgressReceiptJson::read_json(&out.zec_receipt_path).unwrap();
        assert_eq!(z2.burn_nullifier_hex, out.zec_receipt.burn_nullifier_hex);

        let _ = fs::remove_dir_all(&dir);
        // # Safety: restore process env after test pin.
        unsafe {
            std::env::remove_var("CORRIDOR_EGRESS_BURN_EVIDENCE_PATH");
            std::env::remove_var("CORRIDOR_ZEC_EGRESS_RECEIPT_PATH");
            match prev_release {
                Some(v) => std::env::set_var(ENV_CORRIDOR_ZEC_RELEASE, v),
                None => std::env::remove_var(ENV_CORRIDOR_ZEC_RELEASE),
            }
            match prev_profile {
                Some(v) => std::env::set_var(ENV_CORRIDOR_PROFILE, v),
                None => std::env::remove_var(ENV_CORRIDOR_PROFILE),
            }
            match prev_residual {
                Some(v) => std::env::set_var(ENV_CORRIDOR_ALLOW_LAB_INVENTORY_RESIDUAL, v),
                None => std::env::remove_var(ENV_CORRIDOR_ALLOW_LAB_INVENTORY_RESIDUAL),
            }
        }
    }

    #[test]
    fn dest_mismatch_fail_closed() {
        let mint = fixture_mint();
        let handoff =
            build_swap_spend_handoff_from_mint(&mint, ProofModeLabel::MockVerifyLab).unwrap();
        let mut sealed = sealed_golden();
        // Corrupt seal
        sealed.owner_binding = [0xEE; 32];
        sealed.owner_binding_hex = hex::encode(sealed.owner_binding);
        let _settle = sample_settle(&handoff);
        let err = run_option_d_after_settle(EgressDStageInputs {
            handoff: &handoff,
            settle: &_settle,
            sealed: &sealed,
            settle_receipt_path: None,
            profile: "ict_local_funded",
        })
        .unwrap_err();
        let s = err.to_string().to_lowercase();
        assert!(
            s.contains("dest") || s.contains("mismatch") || s.contains("binding"),
            "err={err}"
        );
    }

    #[test]
    fn sealed_dest_for_option_d_matches_g4_golden() {
        let mint = fixture_mint();
        let handoff =
            build_swap_spend_handoff_from_mint(&mint, ProofModeLabel::MockVerifyLab).unwrap();
        let cfg = ZakuraLocalConfig::default();
        let sealed = sealed_dest_for_option_d(&handoff, &cfg).expect("g4 match");
        assert_eq!(sealed.owner_binding_hex, REGTEST_MINER_OWNER_BINDING_HEX);
        assert!(!sealed.dest_display.starts_with("demo_zec_mint_seal_"));
        assert_eq!(sealed.dest_display, REGTEST_MINER_DEST);
    }

    #[test]
    fn sealed_dest_refuse_invent_when_g4_diverges() {
        // Non-golden dest on mint → G4 resolvable but openings diverge → fail closed.
        let mut asset_in = [0u8; 32];
        asset_in[0] = b'H';
        let foreign = [0xABu8; 32];
        let mint = mint_evidence_from_claim_fields(
            "ict_local_funded",
            "test-1",
            "terp1hs",
            &[0x11; 32],
            &[0x22; 32],
            5_000,
            &asset_in,
            Some(&[0x33; 32]),
            Some(&foreign),
            true,
            None,
            Some("intent-x".into()),
        );
        let handoff =
            build_swap_spend_handoff_from_mint(&mint, ProofModeLabel::MockVerifyLab).unwrap();
        let cfg = ZakuraLocalConfig::default();
        let err = sealed_dest_for_option_d(&handoff, &cfg).unwrap_err();
        match &err {
            EgressDError::DestMismatch(msg) => {
                assert!(
                    msg.contains("refuse") || msg.contains("G4") || msg.contains("≠"),
                    "msg={msg}"
                );
            }
            other => panic!("expected DestMismatch, got {other}"),
        }
    }

    #[test]
    fn lab_pay_refuses_empty_evidence() {
        let sealed = sealed_golden();
        refuse_pay_without_evidence(&sealed).unwrap();
    }

    #[test]
    fn lab_pay_dest_mismatch_refuse() {
        let mint = fixture_mint();
        let handoff =
            build_swap_spend_handoff_from_mint(&mint, ProofModeLabel::MockVerifyLab).unwrap();
        let sealed = sealed_golden();
        let settle = sample_settle(&handoff);
        let opening = settle_opening_from_handoff(&handoff, &sealed).unwrap();
        let (evidence, _) = apply_pure_egress_burn_labeled(&opening, &sealed, None).unwrap();
        let mut wrong = sealed.clone();
        wrong.owner_binding = [0xAB; 32];
        wrong.owner_binding_hex = hex::encode(wrong.owner_binding);
        let err = lab_pay_zec_after_burn(&evidence, &wrong).unwrap_err();
        assert!(matches!(err, EgressDError::DestMismatch(_)));
    }

    #[test]
    fn egress_nf_domain_label() {
        assert_eq!(EGRESS_NF_LABEL, b"egress-nf-v0");
    }

    #[test]
    fn lab_degrade_labels_ssot() {
        assert_lab_degrade_labels(
            BURN_SURFACE_PURE_RECORD_LAB,
            MODE_LAB_INVENTORY_PAY_SIMULATED,
        )
        .unwrap();
        assert!(product_celebrate_forbidden(
            BURN_SURFACE_PURE_RECORD_LAB,
            MODE_LAB_INVENTORY_PAY_SIMULATED
        ));
        assert!(!product_celebrate_forbidden(
            BURN_SURFACE_CW_BRIDGE_EGRESS,
            MODE_LAB_INVENTORY_PAY
        ));
        assert!(assert_lab_degrade_labels("zec_sent", MODE_LAB_INVENTORY_PAY).is_err());
        assert!(assert_lab_degrade_labels(BURN_SURFACE_PURE_RECORD_LAB, "").is_err());
    }

    #[test]
    fn decode_hex32_settle_cm() {
        let h = hex::encode([9u8; 32]);
        let b = decode_hex32(&h, "cm").unwrap();
        assert_eq!(b, [9u8; 32]);
    }
}
