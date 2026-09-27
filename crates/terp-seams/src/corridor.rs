//! Cash App → fresh BTC → private bridge → ZEC corridor (pure L0).
//!
//! SPEC: `docs/plans/spectrum/DEMO-CASHAPP-ZEC-CORRIDOR.md` §3–4, §6
//! Agent: `docs/plans/spectrum/agents/DEMO-CORRIDOR-PREAUTH.md`
//!
//! - [`DepositIntentV0`] preauthenticates diversified dest + rate policy
//! - [`intent_allows_swap`] enforces dest bind, min_out, slip vs mid, expiry
//! - [`CorridorAssetBackend`] Simulated | LightClient (MockAttestation | Live stub)
//! - Oracle never mints (hard reject API)
//!
//! No Docker, no cosmwasm, no halo2.

#![deny(unsafe_code)]

use std::collections::HashSet;

use sha2::{Digest, Sha256};

pub use crate::domain::{
    BPS_DEN, CORRIDOR_ID_CASHAPP_BTC_ZEC_V0, CORRIDOR_TAG_DOMAIN, Hash32, INTENT_DOMAIN_TAG,
    PRICE_SCALE, SIM_BURN_DOMAIN_TAG,
};

// =============================================================================
// Errors
// =============================================================================

/// Normative reject codes for intent + swap gate + deposit tracking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CorridorError {
    /// Schema / version bad.
    BadVersion,
    /// Empty or all-zero dest binding (preauth mandatory).
    EmptyDestBinding,
    /// `domain_bind` does not re-derive from canonical fields.
    DomainBindMismatch,
    /// Intent past `expiry` at check time.
    IntentExpired,
    /// Actual owner_binding ≠ intent.dest_owner_binding (I1).
    DestBindingMismatch,
    /// out_value < intent.min_out_value.
    MinOut,
    /// Mid/slip policy violated (I2).
    SlipExceeded,
    /// Oracle mid missing when required (I6).
    OracleMissing,
    /// Oracle mid stale vs now (I6).
    OracleStale,
    /// Mid market id does not match intent.
    OracleMarketMismatch,
    /// Burn ν / intent already minted (I4).
    AlreadyMinted,
    /// Empty burn / deposit id.
    EmptyBurnId,
    /// Backend path not available (Live LC stub).
    BackendUnavailable,
    /// Oracle-only mint is forbidden.
    OracleDisabledMint,
    /// Bad numeric / zero mid.
    BadAmount,
    /// Intent structural validation failed (empty tags, expiry ≤ created, etc.).
    InvalidIntent,
}

pub type CorridorResult<T> = Result<T, CorridorError>;

// =============================================================================
// Oracle policy + mid
// =============================================================================

/// Oracle bound policy on the intent (SPEC §3.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum OracleBoundPolicy {
    /// Accept when `out_value >= floor(mid × in_hint × (1 − slip))` (v0 floor).
    MidGteFloor = 0,
    /// Accept when implied out is within mid band ± slip (same floor + optional upper).
    WithinBand = 1,
}

impl OracleBoundPolicy {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::MidGteFloor),
            1 => Some(Self::WithinBand),
            _ => None,
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Oracle mid snapshot used only as acceptance bounds (never mints).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OracleMid {
    pub market_id: String,
    /// Fixed-point mid: ZEC-out per BTC-in scaled by `PRICE_SCALE`
    /// (`out ≈ in * mid / PRICE_SCALE`).
    pub mid: u128,
    /// Observation time (unix seconds) for staleness.
    pub observed_at: u64,
    /// Optional height for LC-aligned consumers (0 if unused).
    pub observed_height: u64,
}

/// Context for [`intent_allows_swap`] beyond the mid payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwapCheckCtx {
    /// Unix now for expiry + oracle age.
    pub now: u64,
    /// Max age of mid in seconds (0 = only exact observed_at == now allowed if strict).
    pub max_oracle_age_secs: u64,
    /// BTC-side input amount used to derive mid floor (`0` = skip mid-floor, still enforce min_out).
    pub amount_in: u64,
    /// When true (default product path), missing mid is reject (I6).
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

// =============================================================================
// DepositIntentV0
// =============================================================================

/// Preauthentication packet: initial transfer authorizes final destination.
///
/// Canonical fields from DEMO-CASHAPP-ZEC-CORRIDOR §3.1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepositIntentV0 {
    pub version: u8,
    pub corridor_id: String,
    pub source_chain_tag: String,
    pub dest_chain_tag: String,
    /// Fresh receive address (demo may be sim).
    pub btc_deposit_addr: String,
    /// Once funded: bind to burn ν / claim_id material (all-zero pre-fund).
    pub btc_txid_or_intent_id: Hash32,
    /// **Preauth diversified destination** (mandatory on happy path).
    pub dest_owner_binding: Hash32,
    /// UI-only truncated UA/bech32; not authority.
    pub dest_display_hint: Option<String>,
    pub asset_in_id: Hash32,
    pub asset_out_id: Hash32,
    /// Absolute min ZEC-side units after swap.
    pub min_out_value: u64,
    /// Optional slip vs oracle mid (bps).
    pub max_slippage_bps: u16,
    pub oracle_market_id: String,
    pub oracle_bound_policy: OracleBoundPolicy,
    pub created_at: u64,
    pub expiry: u64,
    /// `H("terp-cashapp-intent-v0" ‖ canonical_bytes_without_domain_bind)`.
    pub domain_bind: Hash32,
}

impl DepositIntentV0 {
    /// Build intent and fill `domain_bind` from canonical bytes.
    ///
    /// Rejects empty dest binding and invalid time window.
    pub fn new_preauth(mut fields: DepositIntentFields) -> CorridorResult<Self> {
        if fields.version != 0 {
            return Err(CorridorError::BadVersion);
        }
        if is_zero_hash(&fields.dest_owner_binding) {
            return Err(CorridorError::EmptyDestBinding);
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
            dest_display_hint: fields.dest_display_hint.take(),
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

    /// Canonical bytes **without** `domain_bind` (SPEC §3.1 order).
    ///
    /// Layout (length-prefixed strings as u16 LE + bytes; fixed arrays raw):
    /// ```text
    /// version_u8
    /// ‖ corridor_id
    /// ‖ source_chain_tag
    /// ‖ dest_chain_tag
    /// ‖ btc_deposit_addr
    /// ‖ btc_txid_or_intent_id
    /// ‖ dest_owner_binding
    /// ‖ dest_display_hint_flag_u8 ‖ [hint]
    /// ‖ asset_in_id
    /// ‖ asset_out_id
    /// ‖ min_out_value_le8
    /// ‖ max_slippage_bps_le2
    /// ‖ oracle_market_id
    /// ‖ oracle_bound_policy_u8
    /// ‖ created_at_le8
    /// ‖ expiry_le8
    /// ```
    pub fn canonical_bytes_without_domain_bind(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(256);
        out.push(self.version);
        write_str(&mut out, &self.corridor_id);
        write_str(&mut out, &self.source_chain_tag);
        write_str(&mut out, &self.dest_chain_tag);
        write_str(&mut out, &self.btc_deposit_addr);
        out.extend_from_slice(&self.btc_txid_or_intent_id);
        out.extend_from_slice(&self.dest_owner_binding);
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
        out.push(self.oracle_bound_policy.as_u8());
        out.extend_from_slice(&self.created_at.to_le_bytes());
        out.extend_from_slice(&self.expiry.to_le_bytes());
        out
    }

    /// `H("terp-cashapp-intent-v0" ‖ canonical_bytes_without_domain_bind)`.
    pub fn compute_domain_bind(&self) -> Hash32 {
        let mut hasher = Sha256::new();
        hasher.update(INTENT_DOMAIN_TAG);
        hasher.update(self.canonical_bytes_without_domain_bind());
        let dig = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&dig);
        out
    }

    /// Structural validation + domain_bind re-derive + non-empty dest.
    pub fn validate(&self) -> CorridorResult<()> {
        if self.version != 0 {
            return Err(CorridorError::BadVersion);
        }
        if is_zero_hash(&self.dest_owner_binding) {
            return Err(CorridorError::EmptyDestBinding);
        }
        if self.corridor_id.is_empty()
            || self.source_chain_tag.is_empty()
            || self.dest_chain_tag.is_empty()
            || self.btc_deposit_addr.is_empty()
            || self.oracle_market_id.is_empty()
        {
            return Err(CorridorError::InvalidIntent);
        }
        if self.expiry <= self.created_at {
            return Err(CorridorError::InvalidIntent);
        }
        if self.domain_bind != self.compute_domain_bind() {
            return Err(CorridorError::DomainBindMismatch);
        }
        Ok(())
    }

    /// Bind funded deposit / burn id into the intent (updates domain_bind).
    pub fn bind_deposit_id(&mut self, burn_or_txid: Hash32) -> CorridorResult<()> {
        if is_zero_hash(&burn_or_txid) {
            return Err(CorridorError::EmptyBurnId);
        }
        self.btc_txid_or_intent_id = burn_or_txid;
        self.domain_bind = self.compute_domain_bind();
        self.validate()
    }
}

/// Field bag for [`DepositIntentV0::new_preauth`] (domain_bind filled automatically).
#[derive(Clone, Debug)]
pub struct DepositIntentFields {
    pub version: u8,
    pub corridor_id: String,
    pub source_chain_tag: String,
    pub dest_chain_tag: String,
    pub btc_deposit_addr: String,
    pub btc_txid_or_intent_id: Hash32,
    pub dest_owner_binding: Hash32,
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

// =============================================================================
// intent_allows_swap
// =============================================================================

/// Enforce dest binding + min_out + slip vs mid + expiry against preauth intent.
///
/// Product path: call before any successful swap settle. Oracle never mints.
///
/// Checks:
/// 1. intent.validate()
/// 2. now < expiry (I3)
/// 3. actual_owner_binding == intent.dest_owner_binding (I1)
/// 4. out_value >= min_out_value
/// 5. oracle mid present/fresh when required (I6)
/// 6. mid market matches; slip / floor policy (I2)
pub fn intent_allows_swap(
    intent: &DepositIntentV0,
    oracle_mid: Option<&OracleMid>,
    out_value: u64,
    actual_owner_binding: &Hash32,
    ctx: &SwapCheckCtx,
) -> CorridorResult<()> {
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
            return Ok(());
        }
        Some(mid) => {
            if mid.market_id != intent.oracle_market_id {
                return Err(CorridorError::OracleMarketMismatch);
            }
            if mid.mid == 0 {
                return Err(CorridorError::BadAmount);
            }
            // Staleness by unix age (demo); height-only consumers can set observed_at = now.
            if ctx.now < mid.observed_at {
                return Err(CorridorError::OracleStale);
            }
            let age = ctx.now - mid.observed_at;
            if age > ctx.max_oracle_age_secs {
                return Err(CorridorError::OracleStale);
            }

            // Floor from mid × (1 − slip): expected_out = amount_in * mid / PRICE_SCALE
            // floor = expected_out * (10000 - bps) / 10000
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
        }
    }

    Ok(())
}

/// expected_out = floor(amount_in * mid / PRICE_SCALE)
pub fn expected_out_from_mid(amount_in: u64, mid: u128) -> CorridorResult<u128> {
    (amount_in as u128)
        .checked_mul(mid)
        .map(|n| n / PRICE_SCALE)
        .ok_or(CorridorError::BadAmount)
}

/// floor = expected * (10000 - bps) / 10000
pub fn apply_slip_floor(expected: u128, max_slippage_bps: u16) -> u128 {
    let bps = max_slippage_bps as u128;
    let keep = BPS_DEN.saturating_sub(bps);
    expected.saturating_mul(keep) / BPS_DEN
}

/// ceiling = expected * (10000 + bps) / 10000
pub fn apply_slip_ceiling(expected: u128, max_slippage_bps: u16) -> u128 {
    let bps = max_slippage_bps as u128;
    expected
        .checked_mul(BPS_DEN.saturating_add(bps))
        .map(|x| x / BPS_DEN)
        .unwrap_or(u128::MAX)
}

/// Hard rule: oracle mid alone never credits balances / mints notes.
pub fn oracle_mint_forbidden(_mid: &OracleMid, _asset: &Hash32, _amount: u64) -> CorridorResult<()> {
    Err(CorridorError::OracleDisabledMint)
}

// =============================================================================
// Once-per-burn mint tracking (I4)
// =============================================================================

/// Tracks burn ν / intent deposit ids already used for mint (once-per-burn).
#[derive(Clone, Debug, Default)]
pub struct CorridorMintedSet {
    ids: HashSet<Hash32>,
}

impl CorridorMintedSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn contains(&self, id: &Hash32) -> bool {
        self.ids.contains(id)
    }

    pub fn insert(&mut self, id: Hash32) -> CorridorResult<()> {
        if is_zero_hash(&id) {
            return Err(CorridorError::EmptyBurnId);
        }
        if !self.ids.insert(id) {
            return Err(CorridorError::AlreadyMinted);
        }
        Ok(())
    }

    /// Authorize mint against intent-bound burn id (reject double mint).
    pub fn authorize_mint(&mut self, intent: &DepositIntentV0) -> CorridorResult<()> {
        intent.validate()?;
        if is_zero_hash(&intent.btc_txid_or_intent_id) {
            return Err(CorridorError::EmptyBurnId);
        }
        self.insert(intent.btc_txid_or_intent_id)
    }
}

// =============================================================================
// CorridorAssetBackend
// =============================================================================

/// Simulated faucet state (demo credits only).
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

/// Opaque LC handle (no networking in this crate).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LcHandle {
    pub client_id: String,
    /// Attested tip height (mock or live).
    pub tip_height: u64,
    /// Burn root tip pin (32B).
    pub burn_root: Hash32,
}

/// Light-client mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LcMode {
    /// Fixture attestation only.
    MockAttestation,
    /// Live path stub — not wired in v0 pure tests.
    Live,
}

/// Pluggable asset backend (SPEC §4). Demo defaults to Simulated.
#[derive(Clone, Debug)]
pub enum CorridorAssetBackend {
    /// Demo: mint sim-BTC / sim-ZEC credits; mock burn membership.
    Simulated {
        btc_faucet: SimFaucet,
        zec_faucet: SimFaucet,
    },
    /// Future: LC-attested BTC burn + ZEC transfer proofs.
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

    pub fn is_simulated(&self) -> bool {
        matches!(self, Self::Simulated { .. })
    }
}

/// Result of a successful deposit observation (sim or mock LC).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepositObservation {
    /// Burn ν / deposit id for later mint (bind into intent).
    pub burn_id: Hash32,
    pub deposit_addr: String,
    pub amount: u64,
    pub asset_id: Hash32,
    pub backend_kind: &'static str,
}

/// Simulated deposit helper: credits faucet and produces a deterministic burn id.
///
/// `burn_id = H("terp-cashapp-sim-burn-v0" ‖ domain_bind ‖ addr ‖ amount_le8 ‖ nonce)`.
pub fn sim_deposit(
    backend: &mut CorridorAssetBackend,
    intent: &DepositIntentV0,
    amount: u64,
    nonce: &[u8],
) -> CorridorResult<DepositObservation> {
    intent.validate()?;
    if amount == 0 {
        return Err(CorridorError::BadAmount);
    }

    match backend {
        CorridorAssetBackend::Simulated { btc_faucet, .. } => {
            let burn_id = derive_sim_burn_id(&intent.domain_bind, &intent.btc_deposit_addr, amount, nonce);
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
                // Mock: attestation fixture under tip — synthetic burn id pinned to root.
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

/// Deterministic sim burn / deposit id.
pub fn derive_sim_burn_id(
    pin: &Hash32,
    addr: &str,
    amount: u64,
    nonce: &[u8],
) -> Hash32 {
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

// =============================================================================
// Helpers
// =============================================================================

fn write_str(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    let len = b.len() as u16;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(b);
}

fn is_zero_hash(h: &Hash32) -> bool {
    h.iter().all(|&b| b == 0)
}

/// Convenience: tag string → 32B hash (asset registry demo ids).
pub fn hash_tag(tag: &str) -> Hash32 {
    let mut hasher = Sha256::new();
    hasher.update(CORRIDOR_TAG_DOMAIN);
    hasher.update(tag.as_bytes());
    let dig = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&dig);
    out
}

/// Fresh sim BTC deposit address string for suite isolation.
pub fn fresh_btc_deposit_addr(suite_label: &str, index: u64) -> String {
    format!("sim-btc-{}", &hex_prefix(&hash_tag(&format!("{suite_label}:{index}")), 16))
}

fn hex_prefix(h: &Hash32, n: usize) -> String {
    h.iter()
        .take(n / 2)
        .map(|b| format!("{b:02x}"))
        .collect()
}

// =============================================================================
// Tests I1–I6 (+ helpers)
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn dest_a() -> Hash32 {
        hash_tag("dest-owner-A")
    }
    fn dest_b() -> Hash32 {
        hash_tag("dest-owner-B")
    }
    fn asset_btc() -> Hash32 {
        hash_tag("sim-BTC")
    }
    fn asset_zec() -> Hash32 {
        hash_tag("sim-ZEC")
    }

    fn sample_fields(dest: Hash32) -> DepositIntentFields {
        DepositIntentFields {
            version: 0,
            corridor_id: CORRIDOR_ID_CASHAPP_BTC_ZEC_V0.into(),
            source_chain_tag: "bitcoin".into(),
            dest_chain_tag: "zcash".into(),
            btc_deposit_addr: fresh_btc_deposit_addr("suite", 1),
            btc_txid_or_intent_id: [0u8; 32],
            dest_owner_binding: dest,
            dest_display_hint: Some("u1test…hint".into()),
            asset_in_id: asset_btc(),
            asset_out_id: asset_zec(),
            min_out_value: 90,
            max_slippage_bps: 100, // 1%
            oracle_market_id: "BTC-ZEC".into(),
            oracle_bound_policy: OracleBoundPolicy::MidGteFloor,
            created_at: 1_700_000_000,
            expiry: 1_700_000_000 + 3600,
        }
    }

    fn happy_intent() -> DepositIntentV0 {
        DepositIntentV0::new_preauth(sample_fields(dest_a())).expect("preauth")
    }

    /// mid such that amount_in * mid / PRICE_SCALE = 100
    fn mid_for_out100(amount_in: u64) -> OracleMid {
        // mid = 100 * PRICE_SCALE / amount_in
        let mid = (100u128 * PRICE_SCALE) / (amount_in as u128);
        OracleMid {
            market_id: "BTC-ZEC".into(),
            mid,
            observed_at: 1_700_000_100,
            observed_height: 100,
        }
    }

    fn happy_ctx(amount_in: u64) -> SwapCheckCtx {
        SwapCheckCtx {
            now: 1_700_000_200,
            max_oracle_age_secs: 300,
            amount_in,
            require_oracle: true,
        }
    }

    #[test]
    fn domain_bind_stable_and_domain_separated() {
        let a = happy_intent();
        let b = DepositIntentV0::new_preauth(sample_fields(dest_a())).unwrap();
        assert_eq!(a.domain_bind, b.domain_bind);
        assert_eq!(a.domain_bind, a.compute_domain_bind());

        let mut fields = sample_fields(dest_a());
        fields.min_out_value = 91;
        let c = DepositIntentV0::new_preauth(fields).unwrap();
        assert_ne!(a.domain_bind, c.domain_bind);

        // Tamper domain_bind → validate fails
        let mut bad = a.clone();
        bad.domain_bind[0] ^= 0xff;
        assert_eq!(bad.validate(), Err(CorridorError::DomainBindMismatch));
    }

    #[test]
    fn empty_dest_rejected() {
        let fields = sample_fields([0u8; 32]);
        assert_eq!(
            DepositIntentV0::new_preauth(fields).unwrap_err(),
            CorridorError::EmptyDestBinding
        );
    }

    /// I1 — Swap to different owner_binding than intent → REJECT
    #[test]
    fn i1_swap_different_owner_binding_reject() {
        let intent = happy_intent();
        let amount_in = 1_000;
        let mid = mid_for_out100(amount_in);
        let ctx = happy_ctx(amount_in);
        // out 100 ≥ min 90, mid floor OK, but wrong dest
        let err = intent_allows_swap(&intent, Some(&mid), 100, &dest_b(), &ctx).unwrap_err();
        assert_eq!(err, CorridorError::DestBindingMismatch);
    }

    /// I2 — Mid below depositor floor / slip exceeded → REJECT
    #[test]
    fn i2_slip_exceeded_reject() {
        let intent = happy_intent();
        let amount_in = 1_000;
        let mid = mid_for_out100(amount_in); // expected 100; floor at 1% = 99
        let ctx = happy_ctx(amount_in);
        // out well below floor
        let err = intent_allows_swap(&intent, Some(&mid), 50, &dest_a(), &ctx).unwrap_err();
        // min_out is 90, so MinOut fires first — use out that passes min_out but fails slip
        // expected 100, floor 99, min_out 90 → out=95 fails slip (if floor 99)
        let err2 = intent_allows_swap(&intent, Some(&mid), 95, &dest_a(), &ctx).unwrap_err();
        assert_eq!(err2, CorridorError::SlipExceeded);
        // also document pure min_out path
        assert_eq!(err, CorridorError::MinOut);
    }

    /// I3 — Intent expired → REJECT
    #[test]
    fn i3_intent_expired_reject() {
        let intent = happy_intent();
        let amount_in = 1_000;
        let mid = mid_for_out100(amount_in);
        let mut ctx = happy_ctx(amount_in);
        ctx.now = intent.expiry; // now >= expiry
        let err = intent_allows_swap(&intent, Some(&mid), 100, &dest_a(), &ctx).unwrap_err();
        assert_eq!(err, CorridorError::IntentExpired);
    }

    /// I4 — Double mint same burn ν / intent → REJECT
    #[test]
    fn i4_double_mint_reject() {
        let mut intent = happy_intent();
        let mut backend = CorridorAssetBackend::simulated();
        let dep = sim_deposit(&mut backend, &intent, 1_000, b"nonce-1").unwrap();
        intent.bind_deposit_id(dep.burn_id).unwrap();

        let mut minted = CorridorMintedSet::new();
        minted.authorize_mint(&intent).unwrap();
        let err = minted.authorize_mint(&intent).unwrap_err();
        assert_eq!(err, CorridorError::AlreadyMinted);
        assert!(minted.contains(&dep.burn_id));
    }

    /// I5 — Happy: bound OK + dest match → ACCEPT
    #[test]
    fn i5_happy_bound_ok_dest_match() {
        let mut intent = happy_intent();
        let amount_in = 1_000;
        let mut backend = CorridorAssetBackend::simulated();
        let dep = sim_deposit(&mut backend, &intent, amount_in, b"n0").unwrap();
        intent.bind_deposit_id(dep.burn_id).unwrap();

        let mid = mid_for_out100(amount_in);
        let ctx = happy_ctx(amount_in);
        intent_allows_swap(&intent, Some(&mid), 100, &dest_a(), &ctx).unwrap();

        let mut minted = CorridorMintedSet::new();
        minted.authorize_mint(&intent).unwrap();

        // oracle never mints
        assert_eq!(
            oracle_mint_forbidden(&mid, &asset_zec(), 100),
            Err(CorridorError::OracleDisabledMint)
        );
    }

    /// I6 — Oracle stale / missing mid → REJECT (no silent mint)
    #[test]
    fn i6_oracle_missing_or_stale_reject() {
        let intent = happy_intent();
        let amount_in = 1_000;
        let ctx = happy_ctx(amount_in);

        let err = intent_allows_swap(&intent, None, 100, &dest_a(), &ctx).unwrap_err();
        assert_eq!(err, CorridorError::OracleMissing);

        let mut mid = mid_for_out100(amount_in);
        mid.observed_at = ctx.now - 10_000; // very stale
        let err2 = intent_allows_swap(&intent, Some(&mid), 100, &dest_a(), &ctx).unwrap_err();
        assert_eq!(err2, CorridorError::OracleStale);
    }

    #[test]
    fn sim_deposit_produces_stable_burn_id() {
        let intent = happy_intent();
        let mut backend = CorridorAssetBackend::simulated();
        let a = sim_deposit(&mut backend, &intent, 500, b"x").unwrap();
        let b = sim_deposit(&mut backend, &intent, 500, b"x").unwrap();
        assert_eq!(a.burn_id, b.burn_id);
        assert_eq!(a.backend_kind, "simulated");
        assert_ne!(a.burn_id, [0u8; 32]);
        match &backend {
            CorridorAssetBackend::Simulated { btc_faucet, .. } => {
                assert_eq!(btc_faucet.credits.len(), 2);
            }
            _ => panic!("expected simulated"),
        }
    }

    #[test]
    fn lc_mock_deposit_and_live_stub() {
        let intent = happy_intent();
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
        assert_ne!(dep.burn_id, [0u8; 32]);

        let mut live = CorridorAssetBackend::lc_live_stub(btc, zec);
        assert_eq!(
            sim_deposit(&mut live, &intent, 100, b"m").unwrap_err(),
            CorridorError::BackendUnavailable
        );
    }

    #[test]
    fn within_band_rejects_above_ceiling() {
        let mut fields = sample_fields(dest_a());
        fields.oracle_bound_policy = OracleBoundPolicy::WithinBand;
        fields.min_out_value = 1;
        let intent = DepositIntentV0::new_preauth(fields).unwrap();
        let amount_in = 1_000;
        let mid = mid_for_out100(amount_in); // expected 100; ±1% → [99, 101]
        let ctx = happy_ctx(amount_in);
        assert_eq!(
            intent_allows_swap(&intent, Some(&mid), 150, &dest_a(), &ctx).unwrap_err(),
            CorridorError::SlipExceeded
        );
        intent_allows_swap(&intent, Some(&mid), 100, &dest_a(), &ctx).unwrap();
    }
}
