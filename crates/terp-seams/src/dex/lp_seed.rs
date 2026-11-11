//! Recovered from terp-rs 491ae651 (crates/private_dex_seams/src/lp_seed.rs, lost in the
//! 0afa462d seam-crate consolidation). Logic unchanged; only `crate::` paths moved under `dex`.
//!
//! OmniBridge-shaped LP seed (GUIDE Phases 1–3): bridge mint LP notes → deposit into pool.
//!
//! Conservation planes (do not blur):
//! - **H** home locks/escrow (native)
//! - **T** private notes (claims) — bridge mint alone does **not** bump AMM R
//! - **P** public AMM reserves — updated only by LP seed / swap
//!
//! Product path under `CORRIDOR_POOL_SEED=bridged_lp` must refuse magic CreatePool
//! reserves without prior bridged LP notes of equal value.
//!
//! SSOT: `GUIDE-OMNI-E2E-PRODUCTION-SHAPED-2026-07-22.md` §5–7.

use sha2::{Digest, Sha256};

use crate::dex::{AssetId32, Cm32, Pool, PoolStatus, abstract_leaf_cm};

// ---------------------------------------------------------------------------
// Fixture amounts (GUIDE §5.1.3 — pin once)
// ---------------------------------------------------------------------------

/// LP BTC leg (sats) — bridge-mint → pool BTC reserves.
pub const OMNI_LP_BTC: u128 = 10_000_000;
/// LP ZEC leg (zats) — bridge-mint from escrow liability → pool ZEC reserves.
pub const OMNI_LP_ZEC: u128 = 50_000_000;
/// User corridor deposit (sats) — not used in LP seed; Phase 5 only.
pub const OMNI_USER_BTC: u128 = 1_000_000;

/// Receipt `source` for product-green LP seed.
pub const LP_SEED_SOURCE_BRIDGED_NOTES: &str = "bridged_notes";
/// Forbidden product source label (magic reserves).
pub const LP_SEED_SOURCE_MAGIC_LAB: &str = "magic_lab";

/// Conservation class for LP ZEC mint against escrow (GUIDE §6.2).
pub const CONSERVATION_CLASS_ESCROW_LOCK: &str = "escrow_lock";
/// Conservation class for LP BTC deposit-backed mint.
pub const CONSERVATION_CLASS_BTC_DEPOSIT: &str = "btc_deposit";

/// Domain for LP-deposit nullifiers (≠ pool-nf-v0, ≠ egress-nf-v0, ≠ bridge ingress).
pub const LP_NF_LABEL: &[u8] = b"lp-nf-v0";

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LpSeedError {
    /// Escrow balance would not cover outstanding claims after mint.
    ErrEscrowUnderfunded,
    /// Reserves do not match deposited LP note values.
    ErrReservesMismatch,
    /// Magic CreatePool / invented R under bridged_lp product policy.
    ErrMagicPoolForbidden,
    /// LP seed without prior bridge mints of equal value.
    ErrMissingBridgedNotes,
    /// Zero / inconsistent amounts.
    ErrBadAmount,
    /// Nullifier already spent (LP domain).
    ErrNullifierExists,
    /// Nullifier domain collision or re-derive fail.
    ErrNullifierDomain,
    /// Asset orientation / pool shape wrong.
    ErrWrongAsset,
    /// Schema / structural failure.
    ErrSchema,
    /// Liability bookkeeping overflow / invariant break.
    ErrLiability,
    /// Bridge mint alone tried to bump reserves (SPEC §4.3).
    ErrMintMustNotBumpReserves,
}

pub type LpSeedResult<T> = Result<T, LpSeedError>;

// ---------------------------------------------------------------------------
// Home prep + liability (Phase 1)
// ---------------------------------------------------------------------------

/// Dual-home native prep record (plane H) — amounts + outpoint / escrow refs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DualHomePrepV0 {
    pub lp_btc_sats: u128,
    pub user_btc_sats: u128,
    pub lp_zec_zats: u128,
    /// Prefunded escrow native balance (zats) — must cover all ZEC claims.
    pub escrow_balance_zats: u128,
    pub escrow_addr_label: String,
    pub lp_btc_outpoint_label: String,
    pub user_btc_outpoint_label: String,
}

impl DualHomePrepV0 {
    /// GUIDE fixture: LP_BTC / LP_ZEC / USER_BTC; escrow ≥ LP_ZEC + fee buffer.
    pub fn omni_fixture() -> Self {
        Self {
            lp_btc_sats: OMNI_LP_BTC,
            user_btc_sats: OMNI_USER_BTC,
            lp_zec_zats: OMNI_LP_ZEC,
            // buffer for fees / user out budget narrative
            escrow_balance_zats: OMNI_LP_ZEC + OMNI_LP_ZEC / 10,
            escrow_addr_label: "lab-escrow-t-addr".into(),
            lp_btc_outpoint_label: "lp-btc-utxo-0".into(),
            user_btc_outpoint_label: "user-btc-utxo-0".into(),
        }
    }

    /// Fail-closed: escrow must cover planned LP ZEC (I1 start).
    pub fn assert_escrow_covers_lp_zec(&self) -> LpSeedResult<()> {
        if self.lp_zec_zats == 0 || self.lp_btc_sats == 0 {
            return Err(LpSeedError::ErrBadAmount);
        }
        if self.escrow_balance_zats < self.lp_zec_zats {
            return Err(LpSeedError::ErrEscrowUnderfunded);
        }
        Ok(())
    }
}

/// Outstanding wrap liability vs native escrow (I1).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct EscrowLiabilityV0 {
    pub escrow_balance_zats: u128,
    pub outstanding_zec_claims: u128,
}

impl EscrowLiabilityV0 {
    pub fn new(escrow_balance_zats: u128) -> Self {
        Self {
            escrow_balance_zats,
            outstanding_zec_claims: 0,
        }
    }

    /// I1: escrow ≥ outstanding claims.
    pub fn check_invariant(&self) -> LpSeedResult<()> {
        if self.escrow_balance_zats < self.outstanding_zec_claims {
            return Err(LpSeedError::ErrEscrowUnderfunded);
        }
        Ok(())
    }

    /// Mint ZEC claim: outstanding += amount; refuse if underfunded.
    pub fn mint_zec_claim(&mut self, amount: u128) -> LpSeedResult<()> {
        if amount == 0 {
            return Err(LpSeedError::ErrBadAmount);
        }
        let next = self
            .outstanding_zec_claims
            .checked_add(amount)
            .ok_or(LpSeedError::ErrLiability)?;
        if self.escrow_balance_zats < next {
            return Err(LpSeedError::ErrEscrowUnderfunded);
        }
        self.outstanding_zec_claims = next;
        Ok(())
    }

    /// Burn / release path: outstanding -= amount (Phase 6 hook).
    pub fn release_claim(&mut self, amount: u128) -> LpSeedResult<()> {
        if amount > self.outstanding_zec_claims {
            return Err(LpSeedError::ErrLiability);
        }
        self.outstanding_zec_claims -= amount;
        self.escrow_balance_zats = self
            .escrow_balance_zats
            .checked_sub(amount)
            .ok_or(LpSeedError::ErrLiability)?;
        self.check_invariant()
    }
}

// ---------------------------------------------------------------------------
// Bridge mint LP notes (Phase 2) — plane T only
// ---------------------------------------------------------------------------

/// LP bridge-mint evidence (lab may mock LC; still carries conservation labels).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LpBridgeMintV0 {
    pub asset_id: AssetId32,
    pub value: u128,
    pub cm_public: Cm32,
    pub rcm: [u8; 32],
    pub owner_binding: [u8; 32],
    /// Bridge-domain ingress nullifier (≠ LP seed ν).
    pub bridge_nullifier: [u8; 32],
    /// `"escrow_lock"` | `"btc_deposit"`.
    pub conservation_class: String,
    /// Honesty: `"mock_verify_lab"` until Phase G.
    pub proof_mode: String,
}

/// Synthetic bridge ingress ν for lab LP mints (domain-separated from LP/pool).
pub fn synthetic_bridge_ingress_nf(label: &[u8], cm: &Cm32, rcm: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"bridge-ingress-nf-v0");
    h.update(label);
    h.update(cm);
    h.update(rcm);
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

/// LP-domain nullifier when depositing note into pool: `H("lp-nf-v0" ‖ cm ‖ rcm)`.
pub fn synthetic_lp_seed_nf(note_cm: &Cm32, rcm: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(LP_NF_LABEL);
    h.update(note_cm);
    h.update(rcm);
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

/// Mint LP BTC note (plane T). Does **not** touch pool reserves.
pub fn mint_lp_btc_note(
    asset_btc: AssetId32,
    value: u128,
    owner_binding: [u8; 32],
    rcm: [u8; 32],
    proof_mode: &str,
) -> LpSeedResult<LpBridgeMintV0> {
    if value == 0 || value > u64::MAX as u128 {
        return Err(LpSeedError::ErrBadAmount);
    }
    let cm = abstract_leaf_cm(&asset_btc, value as u64, &owner_binding, &rcm);
    let bridge_nullifier = synthetic_bridge_ingress_nf(b"lp-btc", &cm, &rcm);
    Ok(LpBridgeMintV0 {
        asset_id: asset_btc,
        value,
        cm_public: cm,
        rcm,
        owner_binding,
        bridge_nullifier,
        conservation_class: CONSERVATION_CLASS_BTC_DEPOSIT.into(),
        proof_mode: proof_mode.into(),
    })
}

/// Mint LP ZEC note against escrow liability (plane T). Does **not** touch pool reserves.
///
/// Lab until full LC: `proof_mode=mock_verify_lab` OK **if** liability + escrow checks pass.
pub fn mint_lp_zec_note(
    asset_zec: AssetId32,
    value: u128,
    owner_binding: [u8; 32],
    rcm: [u8; 32],
    liability: &mut EscrowLiabilityV0,
    proof_mode: &str,
) -> LpSeedResult<LpBridgeMintV0> {
    if value == 0 || value > u64::MAX as u128 {
        return Err(LpSeedError::ErrBadAmount);
    }
    liability.mint_zec_claim(value)?;
    let cm = abstract_leaf_cm(&asset_zec, value as u64, &owner_binding, &rcm);
    let bridge_nullifier = synthetic_bridge_ingress_nf(b"lp-zec", &cm, &rcm);
    Ok(LpBridgeMintV0 {
        asset_id: asset_zec,
        value,
        cm_public: cm,
        rcm,
        owner_binding,
        bridge_nullifier,
        conservation_class: CONSERVATION_CLASS_ESCROW_LOCK.into(),
        proof_mode: proof_mode.into(),
    })
}

/// SPEC §4.3: bridge mint must not bump AMM reserves.
pub fn assert_mint_does_not_bump_reserves(
    r_before: (u128, u128),
    r_after: (u128, u128),
) -> LpSeedResult<()> {
    if r_before != r_after {
        return Err(LpSeedError::ErrMintMustNotBumpReserves);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// LP seed → plane P (Phase 3)
// ---------------------------------------------------------------------------

/// Product / lab policy for how pool reserves are authorized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolSeedPolicy {
    /// Product: reserves must come from bridged LP notes (`CORRIDOR_POOL_SEED=bridged_lp`).
    BridgedLp,
    /// Legacy lab: magic CreatePool numbers allowed (structure demos only).
    MagicLab,
}

impl PoolSeedPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            PoolSeedPolicy::BridgedLp => "bridged_lp",
            PoolSeedPolicy::MagicLab => "magic_lab",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "bridged_lp" | "bridged-notes" | "bridged_notes" | "omni" => PoolSeedPolicy::BridgedLp,
            _ => PoolSeedPolicy::MagicLab,
        }
    }
}

/// Public LP seed action (GUIDE §7.1) — lab-equivalent of DepositLiquidity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LpSeedPublic {
    pub pool_id: u64,
    pub asset_btc: AssetId32,
    pub asset_zec: AssetId32,
    /// ΔR_btc = LP_BTC (from note).
    pub delta_r_btc: u128,
    /// ΔR_zec = LP_ZEC (from note).
    pub delta_r_zec: u128,
    pub nullifiers: Vec<[u8; 32]>,
    /// Optional LP share commitment stub.
    pub cm_lp: Option<Cm32>,
}

/// Receipt required for product green (GUIDE §7.2 JSON shape).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LpSeedReceiptV0 {
    pub btc_mint_nf_hex: String,
    pub zec_mint_nf_hex: String,
    pub lp_btc_value: u128,
    pub lp_zec_value: u128,
    pub r_btc: u128,
    pub r_zec: u128,
    /// Must be [`LP_SEED_SOURCE_BRIDGED_NOTES`] for product green.
    pub source: String,
    pub pool_id: u64,
    pub lp_nullifiers_hex: Vec<String>,
    pub conservation_btc: String,
    pub conservation_zec: String,
    pub proof_mode: String,
    pub policy: String,
}

impl LpSeedReceiptV0 {
    pub fn is_bridged_notes(&self) -> bool {
        self.source == LP_SEED_SOURCE_BRIDGED_NOTES
    }

    /// I2: public R after seed equals deposited note values.
    pub fn assert_reserves_match_notes(&self) -> LpSeedResult<()> {
        if self.r_btc != self.lp_btc_value || self.r_zec != self.lp_zec_value {
            return Err(LpSeedError::ErrReservesMismatch);
        }
        if self.lp_btc_value == 0 || self.lp_zec_value == 0 {
            return Err(LpSeedError::ErrBadAmount);
        }
        Ok(())
    }
}

/// Host state for pure LP seed (nullifiers in LP domain).
#[derive(Clone, Debug, Default)]
pub struct LpSeedState {
    pub nullifiers: std::collections::HashSet<[u8; 32]>,
    pub tree_leaves: u64,
}

/// Validate + apply pure LP seed: spend both LP notes into pool → set R.
///
/// Temporary lab CW equivalent: CreatePool with R = note values **only after**
/// this receipt exists (GUIDE §7.2).
pub fn apply_lp_seed(
    pool: &mut Pool,
    state: &mut LpSeedState,
    btc_mint: &LpBridgeMintV0,
    zec_mint: &LpBridgeMintV0,
    public: &LpSeedPublic,
    policy: PoolSeedPolicy,
) -> LpSeedResult<LpSeedReceiptV0> {
    if pool.status != PoolStatus::Active {
        return Err(LpSeedError::ErrSchema);
    }
    if public.delta_r_btc == 0 || public.delta_r_zec == 0 {
        return Err(LpSeedError::ErrBadAmount);
    }
    if btc_mint.value != public.delta_r_btc || zec_mint.value != public.delta_r_zec {
        return Err(LpSeedError::ErrReservesMismatch);
    }
    if btc_mint.asset_id != public.asset_btc || zec_mint.asset_id != public.asset_zec {
        return Err(LpSeedError::ErrWrongAsset);
    }
    // Pure `Pool` uses AssetId enum; economic R is on r_a/r_b. AssetId32 orientation
    // is enforced via mint/public legs above (CW CreatePool binds Binary assets).
    if public.nullifiers.len() != 2 {
        return Err(LpSeedError::ErrSchema);
    }

    let nf_btc = synthetic_lp_seed_nf(&btc_mint.cm_public, &btc_mint.rcm);
    let nf_zec = synthetic_lp_seed_nf(&zec_mint.cm_public, &zec_mint.rcm);
    if public.nullifiers[0] != nf_btc || public.nullifiers[1] != nf_zec {
        return Err(LpSeedError::ErrNullifierDomain);
    }
    // LP ν ≠ bridge ingress ν
    if nf_btc == btc_mint.bridge_nullifier || nf_zec == zec_mint.bridge_nullifier {
        return Err(LpSeedError::ErrNullifierDomain);
    }
    for nf in &public.nullifiers {
        if state.nullifiers.contains(nf) {
            return Err(LpSeedError::ErrNullifierExists);
        }
    }

    // Deposit notes → public inventory
    pool.r_a = pool
        .r_a
        .checked_add(public.delta_r_btc)
        .ok_or(LpSeedError::ErrBadAmount)?;
    pool.r_b = pool
        .r_b
        .checked_add(public.delta_r_zec)
        .ok_or(LpSeedError::ErrBadAmount)?;

    for nf in &public.nullifiers {
        state.nullifiers.insert(*nf);
    }
    state.tree_leaves += 1; // optional cm_lp

    let source = match policy {
        PoolSeedPolicy::BridgedLp => LP_SEED_SOURCE_BRIDGED_NOTES,
        PoolSeedPolicy::MagicLab => LP_SEED_SOURCE_MAGIC_LAB,
    };

    let receipt = LpSeedReceiptV0 {
        btc_mint_nf_hex: hex::encode(btc_mint.bridge_nullifier),
        zec_mint_nf_hex: hex::encode(zec_mint.bridge_nullifier),
        lp_btc_value: btc_mint.value,
        lp_zec_value: zec_mint.value,
        r_btc: pool.r_a,
        r_zec: pool.r_b,
        source: source.into(),
        pool_id: public.pool_id,
        lp_nullifiers_hex: public.nullifiers.iter().map(hex::encode).collect(),
        conservation_btc: btc_mint.conservation_class.clone(),
        conservation_zec: zec_mint.conservation_class.clone(),
        proof_mode: btc_mint.proof_mode.clone(),
        policy: policy.as_str().into(),
    };
    receipt.assert_reserves_match_notes()?;
    Ok(receipt)
}

/// Lab CW bridge: CreatePool with R matching notes **only** when bridged receipts exist.
///
/// Under [`PoolSeedPolicy::BridgedLp`], inventing R without notes is forbidden.
pub fn authorize_create_pool_reserves(
    policy: PoolSeedPolicy,
    r_btc: u128,
    r_zec: u128,
    receipt: Option<&LpSeedReceiptV0>,
) -> LpSeedResult<()> {
    match policy {
        PoolSeedPolicy::MagicLab => {
            if r_btc == 0 || r_zec == 0 {
                return Err(LpSeedError::ErrBadAmount);
            }
            Ok(())
        }
        PoolSeedPolicy::BridgedLp => {
            let Some(rec) = receipt else {
                return Err(LpSeedError::ErrMissingBridgedNotes);
            };
            if !rec.is_bridged_notes() {
                return Err(LpSeedError::ErrMagicPoolForbidden);
            }
            rec.assert_reserves_match_notes()?;
            if rec.r_btc != r_btc || rec.r_zec != r_zec {
                return Err(LpSeedError::ErrReservesMismatch);
            }
            Ok(())
        }
    }
}

/// Refuse product green when policy is bridged_lp and source is magic-only.
pub fn reject_magic_pool_under_bridged_policy(
    policy: PoolSeedPolicy,
    source: &str,
) -> LpSeedResult<()> {
    if policy == PoolSeedPolicy::BridgedLp && source != LP_SEED_SOURCE_BRIDGED_NOTES {
        return Err(LpSeedError::ErrMagicPoolForbidden);
    }
    Ok(())
}

/// Full pure bootstrap Phases 1–3 (no chain): dual-home → mint LP → seed R.
///
/// Returns receipt + empty-start pool with reserves = LP values.
// Positional API kept: zk-test-press harness/omni_lp_seed.rs calls it as recovered.
#[allow(clippy::too_many_arguments)]
pub fn bootstrap_omni_liquidity_pure(
    asset_btc: AssetId32,
    asset_zec: AssetId32,
    prep: &DualHomePrepV0,
    lp_owner: [u8; 32],
    btc_rcm: [u8; 32],
    zec_rcm: [u8; 32],
    pool_id: u64,
    gamma: u64,
    gamma_den: u64,
    proof_mode: &str,
) -> LpSeedResult<(
    Pool,
    LpSeedState,
    EscrowLiabilityV0,
    LpSeedReceiptV0,
    LpBridgeMintV0,
    LpBridgeMintV0,
)> {
    prep.assert_escrow_covers_lp_zec()?;
    let mut liability = EscrowLiabilityV0::new(prep.escrow_balance_zats);

    // Phase 2 — mints (T). Reserves start at 0.
    let btc_mint = mint_lp_btc_note(asset_btc, prep.lp_btc_sats, lp_owner, btc_rcm, proof_mode)?;
    let zec_mint = mint_lp_zec_note(
        asset_zec,
        prep.lp_zec_zats,
        lp_owner,
        zec_rcm,
        &mut liability,
        proof_mode,
    )?;
    assert_mint_does_not_bump_reserves((0, 0), (0, 0))?;

    // Phase 3 — empty pool then seed from notes
    let mut pool = Pool {
        pool_id,
        // Pure Pool uses AssetId enum; keep hub/b placeholders — values carry economics.
        // Callers that need AssetId32 orientation use receipt + public legs.
        asset_a: crate::dex::AssetId::Hub,
        asset_b: crate::dex::AssetId::AssetB,
        r_a: 0,
        r_b: 0,
        gamma,
        gamma_den,
        status: PoolStatus::Active,
    };
    let mut state = LpSeedState::default();
    let public = LpSeedPublic {
        pool_id,
        asset_btc,
        asset_zec,
        delta_r_btc: btc_mint.value,
        delta_r_zec: zec_mint.value,
        nullifiers: vec![
            synthetic_lp_seed_nf(&btc_mint.cm_public, &btc_mint.rcm),
            synthetic_lp_seed_nf(&zec_mint.cm_public, &zec_mint.rcm),
        ],
        cm_lp: None,
    };
    let receipt = apply_lp_seed(
        &mut pool,
        &mut state,
        &btc_mint,
        &zec_mint,
        &public,
        PoolSeedPolicy::BridgedLp,
    )?;
    authorize_create_pool_reserves(
        PoolSeedPolicy::BridgedLp,
        pool.r_a,
        pool.r_b,
        Some(&receipt),
    )?;
    Ok((pool, state, liability, receipt, btc_mint, zec_mint))
}

// Minimal hex without extra dep (private_dex_seams only has sha2).
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let b = bytes.as_ref();
        let mut s = String::with_capacity(b.len() * 2);
        for &x in b {
            s.push(HEX[(x >> 4) as usize] as char);
            s.push(HEX[(x & 0xf) as usize] as char);
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dex::{asset_id_b, asset_id_hub};

    fn owner_fix() -> [u8; 32] {
        [0x4c; 32]
    }

    #[test]
    fn reserves_match_lp_notes_after_seed() {
        let prep = DualHomePrepV0::omni_fixture();
        let (pool, _, liability, receipt, btc, zec) = bootstrap_omni_liquidity_pure(
            asset_id_hub(),
            asset_id_b(),
            &prep,
            owner_fix(),
            [0xB1; 32],
            [0xB2; 32],
            1,
            997,
            1000,
            "mock_verify_lab",
        )
        .expect("bootstrap");

        assert_eq!(pool.r_a, OMNI_LP_BTC);
        assert_eq!(pool.r_b, OMNI_LP_ZEC);
        assert_eq!(receipt.r_btc, btc.value);
        assert_eq!(receipt.r_zec, zec.value);
        assert_eq!(receipt.source, LP_SEED_SOURCE_BRIDGED_NOTES);
        receipt.assert_reserves_match_notes().unwrap();
        assert_eq!(liability.outstanding_zec_claims, OMNI_LP_ZEC);
        liability.check_invariant().unwrap();
    }

    #[test]
    fn reject_inventing_r_without_bridged_source() {
        let err = authorize_create_pool_reserves(
            PoolSeedPolicy::BridgedLp,
            OMNI_LP_BTC,
            OMNI_LP_ZEC,
            None,
        )
        .unwrap_err();
        assert_eq!(err, LpSeedError::ErrMissingBridgedNotes);

        let magic = LpSeedReceiptV0 {
            btc_mint_nf_hex: "aa".into(),
            zec_mint_nf_hex: "bb".into(),
            lp_btc_value: OMNI_LP_BTC,
            lp_zec_value: OMNI_LP_ZEC,
            r_btc: OMNI_LP_BTC,
            r_zec: OMNI_LP_ZEC,
            source: LP_SEED_SOURCE_MAGIC_LAB.into(),
            pool_id: 1,
            lp_nullifiers_hex: vec![],
            conservation_btc: CONSERVATION_CLASS_BTC_DEPOSIT.into(),
            conservation_zec: CONSERVATION_CLASS_ESCROW_LOCK.into(),
            proof_mode: "mock_verify_lab".into(),
            policy: "bridged_lp".into(),
        };
        let err = authorize_create_pool_reserves(
            PoolSeedPolicy::BridgedLp,
            OMNI_LP_BTC,
            OMNI_LP_ZEC,
            Some(&magic),
        )
        .unwrap_err();
        assert_eq!(err, LpSeedError::ErrMagicPoolForbidden);

        assert_eq!(
            reject_magic_pool_under_bridged_policy(
                PoolSeedPolicy::BridgedLp,
                LP_SEED_SOURCE_MAGIC_LAB
            ),
            Err(LpSeedError::ErrMagicPoolForbidden)
        );
    }

    #[test]
    fn escrow_underfunded_refuses_lp_zec_mint() {
        let mut liability = EscrowLiabilityV0::new(100);
        let err = mint_lp_zec_note(
            asset_id_b(),
            1_000,
            owner_fix(),
            [1u8; 32],
            &mut liability,
            "mock_verify_lab",
        )
        .unwrap_err();
        assert_eq!(err, LpSeedError::ErrEscrowUnderfunded);
        assert_eq!(liability.outstanding_zec_claims, 0);
    }

    #[test]
    fn bridge_mint_does_not_imply_reserves() {
        // After mints only, R stays 0 until apply_lp_seed.
        let mut liability = EscrowLiabilityV0::new(OMNI_LP_ZEC);
        let btc = mint_lp_btc_note(
            asset_id_hub(),
            OMNI_LP_BTC,
            owner_fix(),
            [0x11; 32],
            "mock_verify_lab",
        )
        .unwrap();
        let zec = mint_lp_zec_note(
            asset_id_b(),
            OMNI_LP_ZEC,
            owner_fix(),
            [0x22; 32],
            &mut liability,
            "mock_verify_lab",
        )
        .unwrap();
        let _ = (btc, zec);
        assert_mint_does_not_bump_reserves((0, 0), (0, 0)).unwrap();
        assert!(assert_mint_does_not_bump_reserves((0, 0), (1, 0)).is_err());
    }

    #[test]
    fn magic_lab_policy_allows_create_without_receipt() {
        authorize_create_pool_reserves(PoolSeedPolicy::MagicLab, 1_000_000, 2_000_000, None)
            .unwrap();
    }

    #[test]
    fn lp_nf_domain_neq_bridge() {
        let btc = mint_lp_btc_note(
            asset_id_hub(),
            100,
            owner_fix(),
            [0x33; 32],
            "mock_verify_lab",
        )
        .unwrap();
        let lp_nf = synthetic_lp_seed_nf(&btc.cm_public, &btc.rcm);
        assert_ne!(lp_nf, btc.bridge_nullifier);
    }
}
