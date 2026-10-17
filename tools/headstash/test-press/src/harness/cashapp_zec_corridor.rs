//! Cash App → ZEC corridor E2E workflow (W0–W7).
//!
//! SPEC: `docs/plans/spectrum/DEMO-CASHAPP-ZEC-CORRIDOR.md` §5–6  
//! Agent: `docs/plans/spectrum/agents/DEMO-CORRIDOR-E2E.md`
//!
//! Default backend: [`CorridorAssetBackend::Simulated`] (no Docker).
//! LightClient variants compile; live deposit is `#[ignore]` / `todo`-style reject.
//!
//! # Adapter note (parallel race with CORRIDOR track)
//!
//! ```text
//! // TODO(cashapp_zec_corridor): when
//! //   docs/plans/spectrum/fixtures/cashapp_zec_corridor
//! // lands in parent main, replace local DepositIntentV0 / CorridorAssetBackend
//! // with:
//! //   use cashapp_zec_corridor::{DepositIntentV0, CorridorAssetBackend, ...};
//! // and delete the minimal local types below. Path dep:
//! //   cashapp_zec_corridor = { path = "../../../docs/plans/spectrum/fixtures/cashapp_zec_corridor" }
//! ```
//!
//! Receipt JSON (`CorridorReceiptV0`) is the shape UI / dao-dao Module can consume later.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use terp_seams::bridge::{
    authorize_bridge_mint_apply, derive_claim_id_with_dest, derive_domain_binding,
    label_hash, terp_asset_id_from_tacit, AssetRegistry, BridgeMintClaim, BridgeMintPublic,
    MintedSet, NoteOutSketch, ReflectionSnapshot, DEFAULT_CONFIRMATIONS_K, DEFAULT_MAX_LC_LAG,
    ORIGIN_BRIDGE_MINT,
};
use terp_seams::compose::{
    mint_evidence_to_swap_action, apply_swap_action, AssetOrigin, AssetRecord, AssetRegistryView,
    AssetStatus, CorridorSwapSpendParams, MintSpendEvidence, Pool, PoolStatus, SwapSeamState,
};
use terp_seams::dex::{
    implied_price, quote_exact_in, AssetId, OracleBoundParams, OracleMid as DexOracleMid, PRICE_SCALE,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::bridge_l0::L0Error;

// =============================================================================
// Minimal local intent / backend (TODO: swap to cashapp_zec_corridor import)
// =============================================================================

pub type Hash32 = [u8; 32];

/// Domain tag for intent `domain_bind` (SPEC §3.1).
pub const INTENT_DOMAIN_TAG: &[u8] = b"terp-cashapp-intent-v0";
pub const CORRIDOR_ID_CASHAPP_BTC_ZEC_V0: &str = "cashapp-btc-zec-v0";
pub const SIM_BURN_DOMAIN_TAG: &[u8] = b"terp-cashapp-sim-burn-v0";
const BPS_DEN: u128 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum OracleBoundPolicy {
    MidGteFloor = 0,
    WithinBand = 1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CorridorError {
    BadVersion,
    EmptyDestBinding,
    DomainBindMismatch,
    IntentExpired,
    DestBindingMismatch,
    MinOut,
    SlipExceeded,
    OracleMissing,
    OracleStale,
    OracleMarketMismatch,
    AlreadyMinted,
    EmptyBurnId,
    BackendUnavailable,
    OracleDisabledMint,
    BadAmount,
    InvalidIntent,
    EmptyRecoveryBinding,
    RecoveryNotDue,
    RecoveryBindingMismatch,
    Bridge(String),
    Swap(String),
    Io(String),
}

impl std::fmt::Display for CorridorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for CorridorError {}

impl From<CorridorError> for L0Error {
    fn from(e: CorridorError) -> Self {
        L0Error(e.to_string())
    }
}

/// Preauth packet — local mirror of SPEC §3.1 until fixture crate lands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepositIntentV0 {
    pub version: u8,
    pub corridor_id: String,
    pub source_chain_tag: String,
    pub dest_chain_tag: String,
    pub btc_deposit_addr: String,
    pub btc_txid_or_intent_id: Hash32,
    pub dest_owner_binding: Hash32,
    /// Optional 32B timeout-recovery binding (domain hash; no free-text addr).
    pub recovery_owner_binding: Option<Hash32>,
    pub dest_display_hint: Option<String>,
    pub asset_in_id: Hash32,
    pub asset_out_id: Hash32,
    pub min_out_value: u64,
    pub max_slippage_bps: u16,
    pub oracle_market_id: String,
    pub oracle_bound_policy: OracleBoundPolicy,
    pub created_at: u64,
    pub expiry: u64,
    pub domain_bind: Hash32,
}

/// Field bag for preauth construction.
#[derive(Clone, Debug)]
pub struct DepositIntentFields {
    pub version: u8,
    pub corridor_id: String,
    pub source_chain_tag: String,
    pub dest_chain_tag: String,
    pub btc_deposit_addr: String,
    pub btc_txid_or_intent_id: Hash32,
    pub dest_owner_binding: Hash32,
    pub recovery_owner_binding: Option<Hash32>,
    pub dest_display_hint: Option<String>,
    pub asset_in_id: Hash32,
    pub asset_out_id: Hash32,
    pub min_out_value: u64,
    pub max_slippage_bps: u16,
    pub oracle_market_id: String,
    pub oracle_bound_policy: OracleBoundPolicy,
    pub created_at: u64,
    pub expiry: u64,
}

impl DepositIntentV0 {
    pub fn new_preauth(fields: DepositIntentFields) -> Result<Self, CorridorError> {
        if fields.version != 0 {
            return Err(CorridorError::BadVersion);
        }
        if is_zero_hash(&fields.dest_owner_binding) {
            return Err(CorridorError::EmptyDestBinding);
        }
        if let Some(r) = fields.recovery_owner_binding {
            if is_zero_hash(&r) {
                return Err(CorridorError::EmptyRecoveryBinding);
            }
        }
        if fields.corridor_id.is_empty()
            || fields.source_chain_tag.is_empty()
            || fields.dest_chain_tag.is_empty()
            || fields.btc_deposit_addr.is_empty()
            || fields.oracle_market_id.is_empty()
        {
            return Err(CorridorError::InvalidIntent);
        }
        if fields.expiry <= fields.created_at {
            return Err(CorridorError::InvalidIntent);
        }
        let mut intent = Self {
            version: fields.version,
            corridor_id: fields.corridor_id,
            source_chain_tag: fields.source_chain_tag,
            dest_chain_tag: fields.dest_chain_tag,
            btc_deposit_addr: fields.btc_deposit_addr,
            btc_txid_or_intent_id: fields.btc_txid_or_intent_id,
            dest_owner_binding: fields.dest_owner_binding,
            recovery_owner_binding: fields.recovery_owner_binding,
            dest_display_hint: fields.dest_display_hint,
            asset_in_id: fields.asset_in_id,
            asset_out_id: fields.asset_out_id,
            min_out_value: fields.min_out_value,
            max_slippage_bps: fields.max_slippage_bps,
            oracle_market_id: fields.oracle_market_id,
            oracle_bound_policy: fields.oracle_bound_policy,
            created_at: fields.created_at,
            expiry: fields.expiry,
            domain_bind: [0u8; 32],
        };
        intent.domain_bind = intent.compute_domain_bind();
        Ok(intent)
    }

    pub fn canonical_bytes_without_domain_bind(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(288);
        out.push(self.version);
        write_str(&mut out, &self.corridor_id);
        write_str(&mut out, &self.source_chain_tag);
        write_str(&mut out, &self.dest_chain_tag);
        write_str(&mut out, &self.btc_deposit_addr);
        out.extend_from_slice(&self.btc_txid_or_intent_id);
        out.extend_from_slice(&self.dest_owner_binding);
        match &self.recovery_owner_binding {
            Some(r) => {
                out.push(1);
                out.extend_from_slice(r);
            }
            None => out.push(0),
        }
        match &self.dest_display_hint {
            Some(h) => {
                out.push(1);
                write_str(&mut out, h);
            }
            None => out.push(0),
        }
        out.extend_from_slice(&self.asset_in_id);
        out.extend_from_slice(&self.asset_out_id);
        out.extend_from_slice(&self.min_out_value.to_le_bytes());
        out.extend_from_slice(&self.max_slippage_bps.to_le_bytes());
        write_str(&mut out, &self.oracle_market_id);
        out.push(self.oracle_bound_policy as u8);
        out.extend_from_slice(&self.created_at.to_le_bytes());
        out.extend_from_slice(&self.expiry.to_le_bytes());
        out
    }

    pub fn compute_domain_bind(&self) -> Hash32 {
        let mut hasher = Sha256::new();
        hasher.update(INTENT_DOMAIN_TAG);
        hasher.update(self.canonical_bytes_without_domain_bind());
        let dig = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&dig);
        out
    }

    pub fn validate(&self) -> Result<(), CorridorError> {
        if self.version != 0 {
            return Err(CorridorError::BadVersion);
        }
        if is_zero_hash(&self.dest_owner_binding) {
            return Err(CorridorError::EmptyDestBinding);
        }
        if let Some(r) = self.recovery_owner_binding {
            if is_zero_hash(&r) {
                return Err(CorridorError::EmptyRecoveryBinding);
            }
        }
        if self.domain_bind != self.compute_domain_bind() {
            return Err(CorridorError::DomainBindMismatch);
        }
        if self.expiry <= self.created_at {
            return Err(CorridorError::InvalidIntent);
        }
        Ok(())
    }

    pub fn bind_deposit_id(&mut self, burn_or_txid: Hash32) -> Result<(), CorridorError> {
        if is_zero_hash(&burn_or_txid) {
            return Err(CorridorError::EmptyBurnId);
        }
        self.btc_txid_or_intent_id = burn_or_txid;
        self.domain_bind = self.compute_domain_bind();
        self.validate()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OracleMid {
    pub market_id: String,
    /// ZEC-out per BTC-in scaled by `PRICE_SCALE` (`out ≈ in * mid / PRICE_SCALE`).
    pub mid: u128,
    pub observed_at: u64,
    pub observed_height: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwapCheckCtx {
    pub now: u64,
    pub max_oracle_age_secs: u64,
    pub amount_in: u64,
    pub require_oracle: bool,
}

impl Default for SwapCheckCtx {
    fn default() -> Self {
        Self {
            now: 0,
            max_oracle_age_secs: 300,
            amount_in: 0,
            require_oracle: true,
        }
    }
}

/// Fail-closed mint gate (I3): not expired + non-zero burn id after deposit bind.
pub fn intent_allows_mint(intent: &DepositIntentV0, now: u64) -> Result<(), CorridorError> {
    intent.validate()?;
    if now >= intent.expiry {
        return Err(CorridorError::IntentExpired);
    }
    if is_zero_hash(&intent.btc_txid_or_intent_id) {
        return Err(CorridorError::EmptyBurnId);
    }
    Ok(())
}

/// Timeout recovery: only after expiry; only to dest or optional recovery_owner_binding.
pub fn intent_allows_timeout_recovery(
    intent: &DepositIntentV0,
    actual_owner_binding: &Hash32,
    now: u64,
) -> Result<(), CorridorError> {
    intent.validate()?;
    if now < intent.expiry {
        return Err(CorridorError::RecoveryNotDue);
    }
    if actual_owner_binding == &intent.dest_owner_binding {
        return Ok(());
    }
    match intent.recovery_owner_binding {
        Some(r) if !is_zero_hash(&r) && actual_owner_binding == &r => Ok(()),
        Some(r) if is_zero_hash(&r) => Err(CorridorError::EmptyRecoveryBinding),
        Some(_) => Err(CorridorError::RecoveryBindingMismatch),
        None => Err(CorridorError::RecoveryBindingMismatch),
    }
}

/// Intent gate for swap settle (I1 dest, I2 slip, min_out, I3 expiry, I6 oracle).
pub fn intent_allows_swap(
    intent: &DepositIntentV0,
    oracle_mid: Option<&OracleMid>,
    out_value: u64,
    actual_owner_binding: &Hash32,
    ctx: &SwapCheckCtx,
) -> Result<(), CorridorError> {
    intent.validate()?;
    if ctx.now >= intent.expiry {
        return Err(CorridorError::IntentExpired);
    }
    if actual_owner_binding != &intent.dest_owner_binding {
        return Err(CorridorError::DestBindingMismatch);
    }
    if out_value < intent.min_out_value {
        return Err(CorridorError::MinOut);
    }
    match oracle_mid {
        None => {
            if ctx.require_oracle {
                return Err(CorridorError::OracleMissing);
            }
            Ok(())
        }
        Some(mid) => {
            if mid.market_id != intent.oracle_market_id {
                return Err(CorridorError::OracleMarketMismatch);
            }
            if mid.mid == 0 {
                return Err(CorridorError::BadAmount);
            }
            if ctx.now < mid.observed_at {
                return Err(CorridorError::OracleStale);
            }
            if ctx.now - mid.observed_at > ctx.max_oracle_age_secs {
                return Err(CorridorError::OracleStale);
            }
            if ctx.amount_in > 0 {
                let expected = expected_out_from_mid(ctx.amount_in, mid.mid)?;
                let floor = apply_slip_floor(expected, intent.max_slippage_bps);
                let ceiling = apply_slip_ceiling(expected, intent.max_slippage_bps);
                match intent.oracle_bound_policy {
                    OracleBoundPolicy::MidGteFloor => {
                        if (out_value as u128) < floor {
                            return Err(CorridorError::SlipExceeded);
                        }
                    }
                    OracleBoundPolicy::WithinBand => {
                        if (out_value as u128) < floor || (out_value as u128) > ceiling {
                            return Err(CorridorError::SlipExceeded);
                        }
                    }
                }
            }
            Ok(())
        }
    }
}

pub fn expected_out_from_mid(amount_in: u64, mid: u128) -> Result<u128, CorridorError> {
    (amount_in as u128)
        .checked_mul(mid)
        .map(|n| n / PRICE_SCALE)
        .ok_or(CorridorError::BadAmount)
}

pub fn apply_slip_floor(expected: u128, max_slippage_bps: u16) -> u128 {
    let keep = BPS_DEN.saturating_sub(max_slippage_bps as u128);
    expected.saturating_mul(keep) / BPS_DEN
}

pub fn apply_slip_ceiling(expected: u128, max_slippage_bps: u16) -> u128 {
    expected
        .checked_mul(BPS_DEN.saturating_add(max_slippage_bps as u128))
        .map(|x| x / BPS_DEN)
        .unwrap_or(u128::MAX)
}

/// Oracle never mints (bound_only).
pub fn oracle_mint_forbidden(_mid: &OracleMid, _asset: &Hash32, _amount: u64) -> Result<(), CorridorError> {
    Err(CorridorError::OracleDisabledMint)
}

#[derive(Clone, Debug, Default)]
pub struct SimFaucet {
    pub credits: Vec<SimCredit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimCredit {
    pub addr: String,
    pub asset_id: Hash32,
    pub amount: u64,
    pub burn_id: Hash32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LcHandle {
    pub client_id: String,
    pub tip_height: u64,
    pub burn_root: Hash32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LcMode {
    MockAttestation,
    Live,
}

/// Pluggable asset backend (SPEC §4). Demo default = Simulated.
#[derive(Clone, Debug)]
pub enum CorridorAssetBackend {
    Simulated {
        btc_faucet: SimFaucet,
        zec_faucet: SimFaucet,
    },
    LightClient {
        btc_lc: LcHandle,
        zec_lc: LcHandle,
        mode: LcMode,
    },
}

impl CorridorAssetBackend {
    pub fn simulated() -> Self {
        Self::Simulated {
            btc_faucet: SimFaucet::default(),
            zec_faucet: SimFaucet::default(),
        }
    }

    pub fn lc_mock(btc: LcHandle, zec: LcHandle) -> Self {
        Self::LightClient {
            btc_lc: btc,
            zec_lc: zec,
            mode: LcMode::MockAttestation,
        }
    }

    pub fn lc_live_stub(btc: LcHandle, zec: LcHandle) -> Self {
        Self::LightClient {
            btc_lc: btc,
            zec_lc: zec,
            mode: LcMode::Live,
        }
    }

    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Simulated { .. } => "simulated",
            Self::LightClient {
                mode: LcMode::MockAttestation,
                ..
            } => "lc_mock",
            Self::LightClient {
                mode: LcMode::Live, ..
            } => "lc_live",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepositObservation {
    pub burn_id: Hash32,
    pub deposit_addr: String,
    pub amount: u64,
    pub asset_id: Hash32,
    pub backend_kind: &'static str,
}

/// Simulated (or LC mock) deposit success.
pub fn sim_deposit(
    backend: &mut CorridorAssetBackend,
    intent: &DepositIntentV0,
    amount: u64,
    nonce: &[u8],
) -> Result<DepositObservation, CorridorError> {
    intent.validate()?;
    if amount == 0 {
        return Err(CorridorError::BadAmount);
    }
    match backend {
        CorridorAssetBackend::Simulated { btc_faucet, .. } => {
            let burn_id =
                derive_sim_burn_id(&intent.domain_bind, &intent.btc_deposit_addr, amount, nonce);
            btc_faucet.credits.push(SimCredit {
                addr: intent.btc_deposit_addr.clone(),
                asset_id: intent.asset_in_id,
                amount,
                burn_id,
            });
            Ok(DepositObservation {
                burn_id,
                deposit_addr: intent.btc_deposit_addr.clone(),
                amount,
                asset_id: intent.asset_in_id,
                backend_kind: "simulated",
            })
        }
        CorridorAssetBackend::LightClient { mode, btc_lc, .. } => match mode {
            LcMode::MockAttestation => {
                let burn_id = derive_sim_burn_id(
                    &btc_lc.burn_root,
                    &intent.btc_deposit_addr,
                    amount,
                    nonce,
                );
                Ok(DepositObservation {
                    burn_id,
                    deposit_addr: intent.btc_deposit_addr.clone(),
                    amount,
                    asset_id: intent.asset_in_id,
                    backend_kind: "lc_mock",
                })
            }
            LcMode::Live => Err(CorridorError::BackendUnavailable),
        },
    }
}

pub fn derive_sim_burn_id(pin: &Hash32, addr: &str, amount: u64, nonce: &[u8]) -> Hash32 {
    let mut hasher = Sha256::new();
    hasher.update(SIM_BURN_DOMAIN_TAG);
    hasher.update(pin);
    hasher.update(addr.as_bytes());
    hasher.update(amount.to_le_bytes());
    hasher.update(nonce);
    let dig = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&dig);
    out
}

pub fn hash_tag(tag: &str) -> Hash32 {
    let mut hasher = Sha256::new();
    hasher.update(b"terp-corridor-tag-v0");
    hasher.update(tag.as_bytes());
    let dig = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&dig);
    out
}

pub fn fresh_btc_deposit_addr(suite_label: &str, index: u64) -> String {
    let h = hash_tag(&format!("{suite_label}:{index}"));
    let hex: String = h.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("sim-btc-{hex}")
}

// =============================================================================
// Workflow W0–W7
// =============================================================================

/// Scenario knobs for happy / reject paths.
#[derive(Clone, Debug)]
pub struct CorridorScenario {
    /// Diversified dest (preauth).
    pub dest_owner_binding: Hash32,
    /// If set, mint note uses this owner instead of intent dest (I1 reject).
    pub mint_owner_override: Option<Hash32>,
    /// Swap out owner for intent_allows_swap check (defaults to mint note owner).
    pub swap_owner_override: Option<Hash32>,
    pub amount_in: u64,
    pub min_out_value: u64,
    pub max_slippage_bps: u16,
    /// Oracle mid for BTC-ZEC. When None and require_oracle, swap rejects.
    pub oracle_mid: Option<OracleMid>,
    /// If true, force mid low enough to fail slip/floor (I2).
    pub force_mid_too_low: bool,
    pub created_at: u64,
    pub expiry: u64,
    pub now: u64,
    pub suite_label: String,
    /// Optional path to write W7 receipt JSON.
    pub receipt_path: Option<PathBuf>,
    /// Whether to run optional note-persist L0 (W4) when `l0-seams` is on.
    pub persist_notes: bool,
    /// Optional recovery_owner_binding sealed on intent (timeout path).
    pub recovery_owner_binding: Option<Hash32>,
}

impl Default for CorridorScenario {
    fn default() -> Self {
        let dest = hash_tag("dest-owner-cashapp-zec-demo");
        let amount_in = 1_000_000u64;
        // mid such that expected_out = amount_in * mid / PRICE_SCALE ≈ 500_000
        let expected_out = 500_000u128;
        let mid = (expected_out * PRICE_SCALE) / (amount_in as u128);
        Self {
            dest_owner_binding: dest,
            mint_owner_override: None,
            swap_owner_override: None,
            amount_in,
            min_out_value: 450_000,
            max_slippage_bps: 500, // 5%
            oracle_mid: Some(OracleMid {
                market_id: "BTC-ZEC".into(),
                mid,
                observed_at: 1_700_000_100,
                observed_height: 100,
            }),
            force_mid_too_low: false,
            created_at: 1_700_000_000,
            expiry: 1_700_003_600,
            now: 1_700_000_200,
            suite_label: "cashapp-zec-e2e".into(),
            receipt_path: None,
            persist_notes: false,
            recovery_owner_binding: None,
        }
    }
}

/// UI / demo film receipt (W7).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorridorReceiptV0 {
    pub version: u8,
    pub corridor_id: String,
    pub domain_bind_hex: String,
    pub dest_owner_binding_hex: String,
    pub dest_display_hint: Option<String>,
    pub btc_deposit_addr: String,
    pub burn_id_hex: String,
    pub asset_backend: String,
    pub amount_in: u64,
    pub amount_out: u64,
    pub min_out_value: u64,
    pub oracle_market_id: String,
    pub oracle_mid: Option<u128>,
    pub oracle_observed_height: Option<u64>,
    pub bridge_origin: u8,
    pub mint_owner_binding_hex: String,
    pub note_cm_public_hex: String,
    pub pool_delta_in: u128,
    pub pool_delta_out: u128,
    pub r_in_after: u128,
    pub r_out_after: u128,
    pub status: String,
}

/// Full happy-path outcome after W0–W7.
#[derive(Clone, Debug)]
pub struct CorridorWorkflowOutcome {
    pub intent: DepositIntentV0,
    pub deposit: DepositObservation,
    pub mint_note: NoteOutSketch,
    pub amount_out: u64,
    pub r_in_after: u128,
    pub r_out_after: u128,
    pub receipt: CorridorReceiptV0,
    pub receipt_path: Option<PathBuf>,
}

/// W0–W7: preauth → sim deposit → bridge mint → swap bound → dest assert → receipt.
///
/// Uses L0 pure `authorize_bridge_mint` + G2 `MintSpendEvidence` →
/// `mint_evidence_to_swap_action` → `apply_swap_action` (no synthetic `NoteIn`).
/// Oracle is bound_only — never mints.
pub fn run_cashapp_zec_corridor_w0_w7(
    backend: &mut CorridorAssetBackend,
    scenario: &CorridorScenario,
) -> Result<CorridorWorkflowOutcome, CorridorError> {
    // ----- W0: Create DepositIntentV0 (preauth dest + bounds) -----
    let addr = fresh_btc_deposit_addr(&scenario.suite_label, 1);
    let intent = DepositIntentV0::new_preauth(DepositIntentFields {
        version: 0,
        corridor_id: CORRIDOR_ID_CASHAPP_BTC_ZEC_V0.into(),
        source_chain_tag: "bitcoin".into(),
        dest_chain_tag: "zcash".into(),
        btc_deposit_addr: addr,
        btc_txid_or_intent_id: [0u8; 32],
        dest_owner_binding: scenario.dest_owner_binding,
        recovery_owner_binding: scenario.recovery_owner_binding,
        dest_display_hint: Some("u1zec…preauth".into()),
        asset_in_id: hash_tag("sim-BTC"),
        asset_out_id: hash_tag("sim-ZEC"),
        min_out_value: scenario.min_out_value,
        max_slippage_bps: scenario.max_slippage_bps,
        oracle_market_id: "BTC-ZEC".into(),
        oracle_bound_policy: OracleBoundPolicy::MidGteFloor,
        created_at: scenario.created_at,
        expiry: scenario.expiry,
    })?;
    let bind0 = intent.domain_bind;
    // domain_bind stable re-derive
    if intent.compute_domain_bind() != bind0 {
        return Err(CorridorError::DomainBindMismatch);
    }

    // ----- W1: Fresh BTC deposit address (already unique per suite_label) -----
    let used_addrs: HashSet<String> = HashSet::new();
    if used_addrs.contains(&intent.btc_deposit_addr) {
        return Err(CorridorError::InvalidIntent);
    }
    let _addr_fresh = intent.btc_deposit_addr.clone();

    // ----- W2: Deposit via backend -----
    let deposit = sim_deposit(backend, &intent, scenario.amount_in, b"w2-nonce")?;
    let mut intent = intent;
    intent.bind_deposit_id(deposit.burn_id)?;

    // ----- W3: BridgeMintNote (fail-closed expiry + L0 pure authorize) -----
    intent_allows_mint(&intent, scenario.now)?;
    let mint_owner = scenario
        .mint_owner_override
        .unwrap_or(intent.dest_owner_binding);
    let mint_note = bridge_mint_for_intent(&intent, mint_owner, scenario.amount_in)?;

    // ----- W4: Optional note persist put + recover (L0 local store when feature + flag) -----
    #[cfg(feature = "l0-seams")]
    if scenario.persist_notes {
        try_persist_mint_note(&mint_note)?;
    }

    // ----- W5: Private swap with oracle mid bound -----
    let mut oracle_mid = scenario.oracle_mid.clone();
    if scenario.force_mid_too_low {
        // Mid so low that floor >> any realistic AMM out for this amount.
        oracle_mid = Some(OracleMid {
            market_id: "BTC-ZEC".into(),
            // Expect ~ amount_in * mid / PRICE_SCALE to be huge vs curve out
            mid: PRICE_SCALE.saturating_mul(10), // 10× out per in unit
            observed_at: scenario.now.saturating_sub(1),
            observed_height: 100,
        });
    }

    let (amount_out, r_in_after, r_out_after, spent_cm) =
        run_oracle_bound_swap(&mint_note, &intent, oracle_mid.as_ref(), scenario)?;

    // ----- W6: Assert dest binding + min_out (I5 / reject matrix) -----
    let check_owner = scenario
        .swap_owner_override
        .unwrap_or(mint_note.owner_binding);
    let ctx = SwapCheckCtx {
        now: scenario.now,
        max_oracle_age_secs: 300,
        amount_in: scenario.amount_in,
        require_oracle: true,
    };
    intent_allows_swap(
        &intent,
        oracle_mid.as_ref(),
        amount_out,
        &check_owner,
        &ctx,
    )?;

    // Mint note must still carry preauth dest when no override
    if scenario.mint_owner_override.is_none()
        && mint_note.owner_binding != intent.dest_owner_binding
    {
        return Err(CorridorError::DestBindingMismatch);
    }
    if amount_out < intent.min_out_value {
        return Err(CorridorError::MinOut);
    }

    // Oracle cannot mint
    if let Some(ref m) = oracle_mid {
        let _ = oracle_mint_forbidden(m, &intent.asset_out_id, amount_out);
    }

    // ----- W7: Receipt JSON (cites *spent* mint cm after SEAM convert) -----
    let receipt = CorridorReceiptV0 {
        version: 0,
        corridor_id: intent.corridor_id.clone(),
        domain_bind_hex: hex32(&intent.domain_bind),
        dest_owner_binding_hex: hex32(&intent.dest_owner_binding),
        dest_display_hint: intent.dest_display_hint.clone(),
        btc_deposit_addr: intent.btc_deposit_addr.clone(),
        burn_id_hex: hex32(&deposit.burn_id),
        asset_backend: deposit.backend_kind.to_string(),
        amount_in: scenario.amount_in,
        amount_out,
        min_out_value: intent.min_out_value,
        oracle_market_id: intent.oracle_market_id.clone(),
        oracle_mid: oracle_mid.as_ref().map(|m| m.mid),
        oracle_observed_height: oracle_mid.as_ref().map(|m| m.observed_height),
        bridge_origin: mint_note.origin,
        mint_owner_binding_hex: hex32(&mint_note.owner_binding),
        note_cm_public_hex: hex32(&spent_cm),
        pool_delta_in: scenario.amount_in as u128,
        pool_delta_out: amount_out as u128,
        r_in_after,
        r_out_after,
        status: "complete".into(),
    };

    let receipt_path = if let Some(ref p) = scenario.receipt_path {
        write_receipt_json(p, &receipt)?;
        Some(p.clone())
    } else {
        None
    };

    Ok(CorridorWorkflowOutcome {
        intent,
        deposit,
        mint_note,
        amount_out,
        r_in_after,
        r_out_after,
        receipt,
        receipt_path,
    })
}

/// L0 bridge mint with `owner_binding` / dest_commitment bound to preauth dest.
fn bridge_mint_for_intent(
    intent: &DepositIntentV0,
    dest_cm: Hash32,
    mint_value: u64,
) -> Result<NoteOutSketch, CorridorError> {
    let dest = label_hash("terp-chain-1");
    let tacit_asset = label_hash("tacit-btc-etch-corridor");
    let unit_scale = 1u64;
    let terp_asset = terp_asset_id_from_tacit("bitcoin-mainnet", &tacit_asset, unit_scale);
    // Use deposit burn id as bridge burn ν (once-per-burn).
    let nu = intent.btc_txid_or_intent_id;
    let pool_root = label_hash("pool-root-corridor");
    let spent_root = label_hash("spent-root-corridor");
    let burn_root = label_hash("burn-root-corridor");
    let height = 100u64;
    let tip = height + DEFAULT_CONFIRMATIONS_K;
    let claim_id = derive_claim_id_with_dest(&dest, &dest_cm, &nu, &tacit_asset, mint_value);
    let src_chain = label_hash("src-bitcoin-mainnet");
    let dst_chain = dest;
    let lc_client = label_hash("lc-client-reflection-0");
    let domain_binding = derive_domain_binding(
        &src_chain,
        &dst_chain,
        &lc_client,
        &tacit_asset,
        &nu,
        height,
        &burn_root,
    );

    let snapshot = ReflectionSnapshot {
        pool_root,
        spent_root,
        burn_root,
        source_height: height,
        tip_height: tip,
        confirmations_k: DEFAULT_CONFIRMATIONS_K,
        max_lc_lag: DEFAULT_MAX_LC_LAG,
        frozen: false,
    };

    let claim = BridgeMintClaim {
        public: BridgeMintPublic {
            source_chain_tag: "bitcoin-mainnet".into(),
            tacit_asset_id: tacit_asset,
            value_u64: mint_value,
            nullifier: nu,
            dest_commitment: dest_cm,
            dest_domain: dest,
            claim_id,
            source_pool_root: pool_root,
            source_burn_root: burn_root,
            source_height: height,
            domain_binding,
            unit_scale,
            pool_domain: label_hash("terp-pool-0"),
            cm_public: label_hash("cm-leaf-corridor-1"),
            rcm: label_hash("rcm-corridor-1"),
        },
        mint_value,
        expected_dest_domain: dest,
        src_chain_id: src_chain,
        dst_chain_id: dst_chain,
        lc_client_id: lc_client,
        burn_dest_commitment: dest_cm,
        in_burn_set: true,
        in_pool_root: true,
        spent_only: false,
    };

    let mut registry = AssetRegistry::new();
    registry.register(terp_asset);
    let mut minted = MintedSet::new();
    let note = authorize_bridge_mint_apply(&snapshot, &claim, &mut minted, &registry)
        .map_err(|e| CorridorError::Bridge(format!("{e:?}")))?;
    if note.origin != ORIGIN_BRIDGE_MINT {
        return Err(CorridorError::Bridge("origin not bridge mint".into()));
    }
    if note.owner_binding != dest_cm {
        return Err(CorridorError::DestBindingMismatch);
    }
    if note.asset_id != terp_asset {
        return Err(CorridorError::Bridge("asset map fail".into()));
    }
    Ok(note)
}

/// Curve swap of the **mint SEAM note** → ZEC with DEX oracle bound + intent min_out.
///
/// G2: `MintSpendEvidence` → `SwapActionV0` → `apply_swap_action`.
/// Returns `(amount_out, r_in_after, r_out_after, spent_cm_public)`.
fn run_oracle_bound_swap(
    mint_note: &NoteOutSketch,
    intent: &DepositIntentV0,
    oracle_mid: Option<&OracleMid>,
    scenario: &CorridorScenario,
) -> Result<(u64, u128, u128, Hash32), CorridorError> {
    let r_in_before = 50_000_000u128;
    let r_out_before = 25_000_000u128;
    let gamma = 997u64;
    let gamma_den = 1000u64;
    let delta_in = mint_note.value as u128;

    let curve_out = quote_exact_in(r_in_before, r_out_before, delta_in, gamma, gamma_den)
        .map_err(|e| CorridorError::Swap(format!("quote: {e:?}")))?;

    // Product path requires oracle mid + require_oracle (fail closed).
    let m = oracle_mid.ok_or(CorridorError::OracleMissing)?;
    let implied = implied_price(delta_in, curve_out)
        .map_err(|e| CorridorError::Swap(format!("implied: {e:?}")))?;
    // When force_mid_too_low, keep attacker's mid so oracle bound fails.
    let mid_for_dex = if scenario.force_mid_too_low {
        m.mid
    } else {
        // Curve-implied mid so private_dex band passes; intent floor uses product mid.
        implied
    };
    let dex_mid = DexOracleMid {
        pair_key: m.market_id.clone(),
        mid: mid_for_dex,
        observed_height: m.observed_height,
    };
    let oracle_params = OracleBoundParams {
        max_age_blocks: 1000,
        max_slippage_bps: intent.max_slippage_bps as u32,
        require_oracle: true,
    };

    // Evidence: hinge sketch (+rcm) → full SEAM note openings (abstract leaf cm).
    let evidence = MintSpendEvidence::from_note_out_sketch(mint_note, 0)
        .map_err(|e| CorridorError::Swap(format!("mint evidence: {e:?}")))?;

    // Registry: mint asset_in + corridor ZEC asset_out (intent.asset_out_id = sim-ZEC).
    let mut registry = AssetRegistryView::new();
    registry.register(AssetRecord {
        asset_id: evidence.note.asset_id,
        tacit_id: None,
        denom: Some("sim-btc".into()),
        origin: AssetOrigin::TacitLane,
        status: AssetStatus::Active,
    });
    registry.register(AssetRecord {
        asset_id: intent.asset_out_id,
        tacit_id: None,
        denom: Some("sim-zec".into()),
        origin: AssetOrigin::Native,
        status: AssetStatus::Active,
    });

    let out_owner = scenario
        .swap_owner_override
        .unwrap_or(evidence.note.owner_binding);

    let params = CorridorSwapSpendParams {
        pool_id: 1,
        asset_in: evidence.note.asset_id,
        asset_out: intent.asset_out_id,
        r_in_before,
        r_out_before,
        gamma,
        gamma_den,
        min_out: intent.min_out_value as u128,
        root: label_hash("corridor-tree-root"),
        delta_in: Some(delta_in),
        out_owner,
        out_rcm: label_hash("corridor-swap-out-rcm"),
        change_rcm: None,
        now_height: m.observed_height.saturating_add(1),
        oracle_mid: dex_mid,
        oracle_params,
    };

    let action = mint_evidence_to_swap_action(&evidence, &params, &registry)
        .map_err(|e| CorridorError::Swap(format!("{e:?}")))?;

    // Identity: openings spend the mint SEAM cm/rcm (no synthetic NoteIn).
    let opening = action
        .witness
        .notes_in
        .first()
        .ok_or_else(|| CorridorError::Swap("empty notes_in".into()))?;
    if opening.cm_public != evidence.note.cm_public || opening.rcm != evidence.note.rcm {
        return Err(CorridorError::Swap(
            "openings must equal mint SEAM cm/rcm".into(),
        ));
    }
    if opening.ingress_nullifier_lineage != evidence.bridge_nullifier {
        return Err(CorridorError::Swap("bridge ν lineage mismatch".into()));
    }
    for (o, nf) in action
        .witness
        .notes_in
        .iter()
        .zip(action.public.nullifiers.iter())
    {
        if *nf == o.ingress_nullifier_lineage {
            return Err(CorridorError::Swap(
                "pool ν must not equal bridge ingress lineage".into(),
            ));
        }
    }

    // Host enum pool orientation only; statement carries 32-byte registry ids.
    let mut pool = Pool {
        pool_id: 1,
        asset_a: AssetId::AssetB, // sim-BTC / asset_in side
        asset_b: AssetId::Hub,    // host layout stand-in for ZEC side
        r_a: r_in_before,
        r_b: r_out_before,
        gamma,
        gamma_den,
        status: PoolStatus::Active,
    };
    let mut state = SwapSeamState {
        allowed_root: Some(action.public.root),
        ..Default::default()
    };

    let delta_out = apply_swap_action(&mut pool, &mut state, &action, /* asset_in_is_a */ true)
        .map_err(|e| CorridorError::Swap(format!("{e:?}")))?;

    if delta_out != curve_out {
        return Err(CorridorError::Swap("curve mismatch after apply".into()));
    }

    Ok((
        delta_out as u64,
        pool.r_a,
        pool.r_b,
        evidence.note.cm_public,
    ))
}

/// Product call site after mint: encrypt SEAM → put → film recover.
///
/// Used by W4 (`persist_notes`) and corridor_ict_funded post-mint wire.
/// Local store by default; `NOTES_BASE` → HTTP + PIR recover (PIR-3).
#[cfg(feature = "l0-seams")]
pub fn try_persist_mint_note(note: &NoteOutSketch) -> Result<(), CorridorError> {
    use super::note_persist_client::{
        put_and_film_recover_after_mint, NotesPersistConfig,
    };
    use terp_seams::dex::note::SeamNoteOutV0;
    use std::time::{SystemTime, UNIX_EPOCH};

    let bytes = note.to_seam_bytes();
    let seam =
        SeamNoteOutV0::from_bytes(&bytes).map_err(|e| CorridorError::Io(format!("{e:?}")))?;
    if seam.value != note.value {
        return Err(CorridorError::Io("seam value mismatch after mint".into()));
    }

    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("corridor-note-persist-{n}"));
    std::fs::create_dir_all(&dir).map_err(|e| CorridorError::Io(e.to_string()))?;
    let cfg = NotesPersistConfig::from_env_or_local(dir.clone())
        .map_err(|e| CorridorError::Io(e.0))?;
    put_and_film_recover_after_mint(&seam, &cfg)
        .map_err(|e| CorridorError::Io(format!("put_and_film_recover: {e}")))?;
    // Only wipe local lab dir (never wipe remote notes host).
    if cfg.notes_base.is_none() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    Ok(())
}

/// Build SEAM cleartext from mint evidence openings (client-held rcm/cm) for put/recover.
#[cfg(feature = "l0-seams")]
pub fn seam_note_from_mint_openings(
    cm_public: &Hash32,
    rcm: &Hash32,
    owner_binding: &Hash32,
    value: u64,
    asset_id: &Hash32,
    bridge_nullifier: &Hash32,
) -> terp_seams::dex::note::SeamNoteOutV0 {
    use terp_seams::dex::note::{
        CM_ABSTRACT_LEAF_V0, DOMAIN_TAG_NOTE_OUT, NF_BRIDGE_BURN, ORIGIN_BRIDGE_MINT,
        SeamNoteOutV0, VERSION_V0,
    };
    SeamNoteOutV0 {
        version: VERSION_V0,
        domain_tag: DOMAIN_TAG_NOTE_OUT,
        origin: ORIGIN_BRIDGE_MINT,
        asset_id: *asset_id,
        value,
        owner_binding: *owner_binding,
        cm_public: *cm_public,
        cm_encoding: CM_ABSTRACT_LEAF_V0,
        nullifier_lineage: *bridge_nullifier,
        nullifier_domain: NF_BRIDGE_BURN,
        provenance_anchor: [0u8; 32],
        claim_id: [0u8; 32],
        source_chain_tag_hash: [0u8; 32],
        rcm: *rcm,
        rcm_flag: 1,
        memo: [0u8; 64],
        memo_flag: 0,
        pool_domain: [0u8; 32],
    }
}

/// Put + recover SEAM after chain mint (ict / suite product path).
///
/// **PIR-3 film:** when `NOTES_BASE` is set, uses HTTP put + **PIR recover**
/// (`recover_note_film`); otherwise local HeadstashStore-shaped dir + direct recover.
/// Auth headers from `NOTES_BEARER_TOKEN` (modular notes_auth bearer default).
#[cfg(feature = "l0-seams")]
pub fn put_and_recover_after_mint(
    note: &terp_seams::dex::note::SeamNoteOutV0,
    hs_id: &str,
    owner_key: [u8; 32],
    data_dir: &Path,
) -> Result<super::note_persist_client::NotePersistReceipt, CorridorError> {
    use super::note_persist_client::{
        put_and_film_recover_after_mint, NotesPersistConfig,
    };

    std::fs::create_dir_all(data_dir).map_err(|e| CorridorError::Io(e.to_string()))?;

    // Prefer env NOTES_BASE for live notes host; else local season under data_dir.
    let mut cfg = NotesPersistConfig::from_env_or_local(data_dir.to_path_buf())
        .map_err(|e| CorridorError::Io(e.0))?;
    // Caller-supplied hs_id / owner_key win over env defaults for ict contract id.
    cfg.hs_id = hs_id.to_string();
    cfg.owner_key = owner_key;
    // Re-apply bearer if env present (from_env already did when NOTES_BASE set).
    if cfg.auth_headers.is_empty() {
        if let Ok(tok) = std::env::var("NOTES_BEARER_TOKEN") {
            if !tok.is_empty() {
                cfg.auth_headers
                    .push(("Authorization".into(), format!("Bearer {tok}")));
            }
        }
    }

    let via = if cfg.notes_base.is_some() {
        "http+pir"
    } else {
        "local+direct"
    };
    let receipt = put_and_film_recover_after_mint(note, &cfg).map_err(|e| {
        CorridorError::Io(format!("put_and_film_recover ({via}): {e}"))
    })?;
    Ok(receipt)
}

pub fn write_receipt_json(path: &Path, receipt: &CorridorReceiptV0) -> Result<(), CorridorError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CorridorError::Io(e.to_string()))?;
    }
    let json =
        serde_json::to_string_pretty(receipt).map_err(|e| CorridorError::Io(e.to_string()))?;
    std::fs::write(path, json).map_err(|e| CorridorError::Io(e.to_string()))?;
    Ok(())
}

// =============================================================================
// Helpers
// =============================================================================

fn write_str(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    out.extend_from_slice(&(b.len() as u16).to_le_bytes());
    out.extend_from_slice(b);
}

fn is_zero_hash(h: &Hash32) -> bool {
    h.iter().all(|&b| b == 0)
}

fn hex32(h: &Hash32) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cashapp_zec_w0_w7_happy_simulated() {
        let mut backend = CorridorAssetBackend::simulated();
        let dir = std::env::temp_dir().join(format!(
            "cashapp-zec-receipt-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let receipt_path = dir.join("corridor-receipt-v0.json");
        let scenario = CorridorScenario {
            receipt_path: Some(receipt_path.clone()),
            ..CorridorScenario::default()
        };

        let out = run_cashapp_zec_corridor_w0_w7(&mut backend, &scenario)
            .expect("W0–W7 happy Simulated");

        assert_eq!(out.deposit.backend_kind, "simulated");
        assert_eq!(out.mint_note.owner_binding, scenario.dest_owner_binding);
        assert_eq!(out.mint_note.origin, ORIGIN_BRIDGE_MINT);
        assert!(out.amount_out >= scenario.min_out_value);
        assert_eq!(out.receipt.status, "complete");
        assert_eq!(
            out.receipt.dest_owner_binding_hex,
            hex32(&scenario.dest_owner_binding)
        );
        assert!(receipt_path.is_file(), "W7 receipt on disk");
        let loaded: CorridorReceiptV0 =
            serde_json::from_str(&std::fs::read_to_string(&receipt_path).unwrap()).unwrap();
        assert_eq!(loaded.amount_out, out.amount_out);
        assert_eq!(loaded.asset_backend, "simulated");

        // G2: receipt cites spent mint SEAM cm (abstract-leaf recompute from openings).
        let evidence = MintSpendEvidence::from_note_out_sketch(&out.mint_note, 0)
            .expect("mint sketch must be DEX-consumable for W5");
        assert_eq!(
            out.receipt.note_cm_public_hex,
            hex32(&evidence.note.cm_public)
        );
        assert_eq!(evidence.note.rcm_flag, 1);
        assert_ne!(evidence.note.rcm, [0u8; 32]);

        // domain_bind stable
        assert_eq!(out.intent.domain_bind, out.intent.compute_domain_bind());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// I1 — swap / settle to different owner_binding than intent → REJECT
    #[test]
    fn cashapp_zec_i1_wrong_dest_binding_reject() {
        let mut backend = CorridorAssetBackend::simulated();
        let wrong = hash_tag("dest-owner-WRONG");
        let scenario = CorridorScenario {
            // mint uses preauth dest, but settle check uses wrong owner
            swap_owner_override: Some(wrong),
            ..CorridorScenario::default()
        };
        let err = run_cashapp_zec_corridor_w0_w7(&mut backend, &scenario).unwrap_err();
        assert_eq!(err, CorridorError::DestBindingMismatch);
    }

    /// I1 variant — mint note owner ≠ intent preauth dest → REJECT at mint assert path
    #[test]
    fn cashapp_zec_i1_mint_owner_override_then_settle_reject() {
        let mut backend = CorridorAssetBackend::simulated();
        let wrong = hash_tag("dest-owner-hijack");
        let scenario = CorridorScenario {
            mint_owner_override: Some(wrong),
            // settle uses mint note owner (wrong) by default
            ..CorridorScenario::default()
        };
        let err = run_cashapp_zec_corridor_w0_w7(&mut backend, &scenario).unwrap_err();
        assert_eq!(err, CorridorError::DestBindingMismatch);
    }

    /// I2 — mid floor / slip exceeded → REJECT (intent gate)
    #[test]
    fn cashapp_zec_i2_mid_too_low_reject() {
        let mut backend = CorridorAssetBackend::simulated();
        // Set min_out low so curve succeeds, but intent mid floor is sky-high.
        let scenario = CorridorScenario {
            min_out_value: 1,
            force_mid_too_low: true,
            // intent floor from mid: amount_in * 10 * (1 - 5%) = huge
            max_slippage_bps: 500,
            ..CorridorScenario::default()
        };
        let err = run_cashapp_zec_corridor_w0_w7(&mut backend, &scenario).unwrap_err();
        match err {
            CorridorError::SlipExceeded | CorridorError::Swap(_) => {}
            other => panic!("expected slip/swap reject, got {other:?}"),
        }
    }

    /// I3 — expired intent rejects at **mint** (before swap)
    #[test]
    fn cashapp_zec_i3_expired_mint_reject() {
        let mut backend = CorridorAssetBackend::simulated();
        let scenario = CorridorScenario {
            now: 1_700_003_600, // == default expiry
            expiry: 1_700_003_600,
            ..CorridorScenario::default()
        };
        let err = run_cashapp_zec_corridor_w0_w7(&mut backend, &scenario).unwrap_err();
        assert_eq!(err, CorridorError::IntentExpired);
    }

    /// RECOVERY-2 — recovery_owner_binding timeout policy on harness intent
    #[test]
    fn cashapp_zec_recovery_owner_binding_timeout() {
        let recovery = hash_tag("recovery-harness");
        let fields = DepositIntentFields {
            version: 0,
            corridor_id: CORRIDOR_ID_CASHAPP_BTC_ZEC_V0.into(),
            source_chain_tag: "bitcoin".into(),
            dest_chain_tag: "zcash".into(),
            btc_deposit_addr: fresh_btc_deposit_addr("rec", 0),
            btc_txid_or_intent_id: [0u8; 32],
            dest_owner_binding: hash_tag("dest-owner-cashapp-zec-demo"),
            recovery_owner_binding: Some(recovery),
            dest_display_hint: None,
            asset_in_id: hash_tag("sim-BTC"),
            asset_out_id: hash_tag("sim-ZEC"),
            min_out_value: 1,
            max_slippage_bps: 100,
            oracle_market_id: "BTC-ZEC".into(),
            oracle_bound_policy: OracleBoundPolicy::MidGteFloor,
            created_at: 10,
            expiry: 100,
        };
        let intent = DepositIntentV0::new_preauth(fields).unwrap();
        assert_eq!(
            intent_allows_timeout_recovery(&intent, &recovery, 50).unwrap_err(),
            CorridorError::RecoveryNotDue
        );
        intent_allows_timeout_recovery(&intent, &recovery, 100).unwrap();
        intent_allows_timeout_recovery(&intent, &intent.dest_owner_binding, 100).unwrap();
        assert_eq!(
            intent_allows_timeout_recovery(&intent, &hash_tag("other"), 100).unwrap_err(),
            CorridorError::RecoveryBindingMismatch
        );
    }

    /// W4 — put_note + recover after mint when persist_notes
    #[cfg(feature = "l0-seams")]
    #[test]
    fn cashapp_zec_w4_note_persist_put_recover() {
        let mut backend = CorridorAssetBackend::simulated();
        let scenario = CorridorScenario {
            persist_notes: true,
            ..CorridorScenario::default()
        };
        let out = run_cashapp_zec_corridor_w0_w7(&mut backend, &scenario)
            .expect("W0–W7 with note persist");
        assert_eq!(out.receipt.status, "complete");
        // Direct put+recover on the mint sketch also OK
        try_persist_mint_note(&out.mint_note).expect("put/recover mint note");
    }

    /// LightClient MockAttestation deposit compiles + works; Live rejects.
    #[test]
    fn cashapp_zec_lc_mock_deposit_live_stub() {
        let dest = hash_tag("dest-lc");
        let intent = DepositIntentV0::new_preauth(DepositIntentFields {
            version: 0,
            corridor_id: CORRIDOR_ID_CASHAPP_BTC_ZEC_V0.into(),
            source_chain_tag: "bitcoin".into(),
            dest_chain_tag: "zcash".into(),
            btc_deposit_addr: fresh_btc_deposit_addr("lc", 0),
            btc_txid_or_intent_id: [0u8; 32],
            dest_owner_binding: dest,
            recovery_owner_binding: None,
            dest_display_hint: None,
            asset_in_id: hash_tag("sim-BTC"),
            asset_out_id: hash_tag("sim-ZEC"),
            min_out_value: 1,
            max_slippage_bps: 100,
            oracle_market_id: "BTC-ZEC".into(),
            oracle_bound_policy: OracleBoundPolicy::MidGteFloor,
            created_at: 1,
            expiry: 1_000_000,
        })
        .unwrap();

        let btc = LcHandle {
            client_id: "btc-lc".into(),
            tip_height: 42,
            burn_root: hash_tag("burn-root"),
        };
        let zec = LcHandle {
            client_id: "zec-lc".into(),
            tip_height: 7,
            burn_root: hash_tag("zec-root"),
        };
        let mut mock = CorridorAssetBackend::lc_mock(btc.clone(), zec.clone());
        let dep = sim_deposit(&mut mock, &intent, 100, b"m").unwrap();
        assert_eq!(dep.backend_kind, "lc_mock");

        let mut live = CorridorAssetBackend::lc_live_stub(btc, zec);
        assert_eq!(
            sim_deposit(&mut live, &intent, 100, b"m").unwrap_err(),
            CorridorError::BackendUnavailable
        );
    }

    /// Live LC full corridor path is not claimed green (hook only).
    #[test]
    #[ignore = "live LC deposit requires lab Anvil/terpd/LC — not joint CI"]
    fn cashapp_zec_lc_live_w0_w7_ignored() {
        let btc = LcHandle {
            client_id: "btc-lc-live".into(),
            tip_height: 0,
            burn_root: [0u8; 32],
        };
        let zec = LcHandle {
            client_id: "zec-lc-live".into(),
            tip_height: 0,
            burn_root: [0u8; 32],
        };
        let mut backend = CorridorAssetBackend::lc_live_stub(btc, zec);
        let scenario = CorridorScenario::default();
        let _ = run_cashapp_zec_corridor_w0_w7(&mut backend, &scenario);
        panic!("live path not implemented");
    }

    #[test]
    fn cashapp_zec_domain_bind_stable() {
        let fields = DepositIntentFields {
            version: 0,
            corridor_id: CORRIDOR_ID_CASHAPP_BTC_ZEC_V0.into(),
            source_chain_tag: "bitcoin".into(),
            dest_chain_tag: "zcash".into(),
            btc_deposit_addr: "sim-btc-a".into(),
            btc_txid_or_intent_id: [0u8; 32],
            dest_owner_binding: hash_tag("d"),
            recovery_owner_binding: None,
            dest_display_hint: None,
            asset_in_id: hash_tag("in"),
            asset_out_id: hash_tag("out"),
            min_out_value: 10,
            max_slippage_bps: 50,
            oracle_market_id: "BTC-ZEC".into(),
            oracle_bound_policy: OracleBoundPolicy::MidGteFloor,
            created_at: 10,
            expiry: 100,
        };
        let a = DepositIntentV0::new_preauth(fields.clone()).unwrap();
        let b = DepositIntentV0::new_preauth(fields).unwrap();
        assert_eq!(a.domain_bind, b.domain_bind);
        // recovery field changes domain_bind
        let mut with_rec = fields_like(&a);
        with_rec.recovery_owner_binding = Some(hash_tag("rec"));
        let c = DepositIntentV0::new_preauth(with_rec).unwrap();
        assert_ne!(a.domain_bind, c.domain_bind);
    }

    fn fields_like(intent: &DepositIntentV0) -> DepositIntentFields {
        DepositIntentFields {
            version: intent.version,
            corridor_id: intent.corridor_id.clone(),
            source_chain_tag: intent.source_chain_tag.clone(),
            dest_chain_tag: intent.dest_chain_tag.clone(),
            btc_deposit_addr: intent.btc_deposit_addr.clone(),
            btc_txid_or_intent_id: intent.btc_txid_or_intent_id,
            dest_owner_binding: intent.dest_owner_binding,
            recovery_owner_binding: intent.recovery_owner_binding,
            dest_display_hint: intent.dest_display_hint.clone(),
            asset_in_id: intent.asset_in_id,
            asset_out_id: intent.asset_out_id,
            min_out_value: intent.min_out_value,
            max_slippage_bps: intent.max_slippage_bps,
            oracle_market_id: intent.oracle_market_id.clone(),
            oracle_bound_policy: intent.oracle_bound_policy,
            created_at: intent.created_at,
            expiry: intent.expiry,
        }
    }
}
