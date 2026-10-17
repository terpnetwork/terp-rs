//! Pure `SwapActionPublic` → CW `SwapStatementPublic` map + lab settle handoff (G3).
//!
//! SSOT map: `DESIGN-HARNESS-SETTLE.md` / HANDOFF § G2→G3.
//! Does **not** put witness (rcm / notes_in) on chain.

use cosmwasm_std::{Binary, Uint128};
use cw_private_dex::msg::{
    OracleBoundParams as CwOracleBoundParams, OracleMid as CwOracleMid, SwapStatementPublic,
};
use terp_seams::dex::{
    asset_id_b, build_swap_action_from_seam_notes, quote_exact_in, synthetic_pool_spend_nf,
    AssetId32, AssetMap, CM_ABSTRACT_LEAF_V0, SeamNoteSketch, SwapActionPublic, SwapActionV0,
    SwapFromSeamParams, SwapActionError,
};

use super::mint_evidence::{
    decode_hex32, hex32, MintEvidenceError, MintEvidenceV0, PROOF_MODE_MOCK_VERIFY_LAB,
};

/// Lab mock swap proof under `Config.mock_verify=true` (honest label — not Halo2).
pub const LAB_MOCK_SWAP_PROOF: &[u8] = b"corridor-ict-mock-swap-proof-v0";

/// Proof mode for settle handoff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProofModeLabel {
    /// Config.mock_verify=true; non-empty proof accepted after host seams.
    MockVerifyLab,
    /// mock_verify=false + zk-api guest — residual, not funded default.
    ZkApi,
}

impl ProofModeLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            ProofModeLabel::MockVerifyLab => PROOF_MODE_MOCK_VERIFY_LAB,
            ProofModeLabel::ZkApi => "zk_api",
        }
    }
}

/// G2 handoff into settle stage.
#[derive(Clone, Debug)]
pub struct SwapSpendHandoffV0 {
    pub mint: MintEvidenceV0,
    pub statement: SwapStatementPublic,
    pub pool_nullifiers_hex: Vec<String>,
    pub proof: Binary,
    pub proof_mode: ProofModeLabel,
    /// Pure action retained for tests / continuity asserts (witness client-side only).
    pub action: SwapActionV0,
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum SwapHandoffError {
    #[error("mint evidence: {0}")]
    Mint(#[from] MintEvidenceError),
    #[error("swap compose: {0:?}")]
    Compose(SwapActionError),
    #[error("pool ν equals bridge ingress ν (NE-4 / nullifier domain)")]
    NullifierDomain,
    #[error("empty proof under mock_verify lab")]
    EmptyProof,
    #[error("{0}")]
    Other(String),
}

/// Map pure public half → CosmWasm statement. Witness NEVER on chain.
/// Drops pure `now_height` — contract uses `env.block.height` for oracle age.
pub fn swap_action_public_to_cw(p: &SwapActionPublic) -> SwapStatementPublic {
    SwapStatementPublic {
        pool_id: p.pool_id,
        asset_in: Binary::from(p.asset_in.to_vec()),
        asset_out: Binary::from(p.asset_out.to_vec()),
        root: Binary::from(p.root.to_vec()),
        nullifiers: p
            .nullifiers
            .iter()
            .map(|n| Binary::from(n.to_vec()))
            .collect(),
        cm_out: p.cm_out.iter().map(|c| Binary::from(c.to_vec())).collect(),
        delta_r_in: Uint128::new(p.delta_r_in),
        delta_r_out: Uint128::new(p.delta_r_out),
        min_out: Uint128::new(p.min_out),
        gamma: p.gamma,
        gamma_den: p.gamma_den,
        r_in_before: Uint128::new(p.r_in_before),
        r_out_before: Uint128::new(p.r_out_before),
        oracle_mid: p.oracle_mid.as_ref().map(|m| CwOracleMid {
            pair_key: m.pair_key.clone(),
            mid: Uint128::new(m.mid),
            observed_height: m.observed_height,
        }),
        oracle_params: p.oracle_params.as_ref().map(|q| CwOracleBoundParams {
            max_age_blocks: q.max_age_blocks,
            max_slippage_bps: q.max_slippage_bps,
            require_oracle: q.require_oracle,
        }),
    }
}

pub fn lab_mock_swap_proof() -> Binary {
    Binary::from(LAB_MOCK_SWAP_PROOF)
}

/// Lab virtual pool reserves (same spirit as cw-private-dex multitest).
pub const LAB_R_IN: u128 = 1_000_000;
pub const LAB_R_OUT: u128 = 2_000_000;
pub const LAB_GAMMA: u64 = 997;
pub const LAB_GAMMA_DEN: u64 = 1000;
pub const LAB_POOL_ID: u64 = 1;

/// Corridor ZEC / sim-ZEC registry id for lab settle (G2: registry id, not silent hub enum).
/// Uses private_dex_seams demo asset_b (`B` prefix) as sim-ZEC stand-in until product pin lands.
pub fn lab_asset_out_zec() -> AssetId32 {
    asset_id_b()
}

/// Lab membership root window (must match instantiate `allowed_root`).
pub fn lab_allowed_root() -> [u8; 32] {
    [7u8; 32]
}

/// Build SEAM sketch from mint evidence openings (uses mint cm/rcm as-is; no synthetic NoteIn).
pub fn seam_sketch_from_mint_evidence(
    mint: &MintEvidenceV0,
    path_position: u64,
) -> Result<SeamNoteSketch, SwapHandoffError> {
    mint.require_settle_openings()?;
    let asset_id = decode_hex32(&mint.asset_id_hex, "asset_id_hex")?;
    let cm_public = decode_hex32(&mint.cm_public_hex, "cm_public_hex")?;
    let rcm = decode_hex32(mint.rcm_hex.as_ref().unwrap(), "rcm_hex")?;
    let owner_binding =
        decode_hex32(mint.owner_binding_hex.as_ref().unwrap(), "owner_binding_hex")?;
    let lineage = decode_hex32(&mint.bridge_nullifier_hex, "bridge_nullifier_hex")?;
    Ok(SeamNoteSketch {
        origin: 0x02, // ORIGIN_BRIDGE_MINT
        asset_id,
        value: mint.value_u64,
        owner_binding,
        cm_public,
        cm_encoding: CM_ABSTRACT_LEAF_V0,
        nullifier_lineage: lineage,
        nullifier_domain: 0x02, // NF_BRIDGE_BURN
        rcm,
        rcm_flag: 1,
        path_position,
    })
}

/// Params for lab CreatePool + settle (no oracle required for default funded settle lab).
pub fn lab_swap_from_seam_params(
    asset_in: AssetId32,
    asset_out: AssetId32,
    delta_in: Option<u128>,
    out_owner: [u8; 32],
    out_rcm: [u8; 32],
) -> SwapFromSeamParams {
    lab_swap_from_seam_params_with_reserves(
        asset_in,
        asset_out,
        delta_in,
        out_owner,
        out_rcm,
        LAB_R_IN,
        LAB_R_OUT,
    )
}

/// Like [`lab_swap_from_seam_params`] but with explicit pre-trade reserves
/// (omni bridged-LP seed uses LP note values, not magic LAB_R_*).
pub fn lab_swap_from_seam_params_with_reserves(
    asset_in: AssetId32,
    asset_out: AssetId32,
    delta_in: Option<u128>,
    out_owner: [u8; 32],
    out_rcm: [u8; 32],
    r_in_before: u128,
    r_out_before: u128,
) -> SwapFromSeamParams {
    SwapFromSeamParams {
        pool_id: LAB_POOL_ID,
        asset_in,
        asset_out,
        r_in_before,
        r_out_before,
        gamma: LAB_GAMMA,
        gamma_den: LAB_GAMMA_DEN,
        min_out: 1,
        root: lab_allowed_root(),
        delta_in,
        out_owner,
        out_rcm,
        change_rcm: None,
        now_height: 1,
        oracle_mid: None,
        oracle_params: None,
    }
}

/// Assert continuous mint → settle openings (MintEvidenceV0 glue into SwapSpendHandoffV0).
///
/// Aligns G3 file evidence with G2 `MintSpendEvidence` spirit: same cm / owner / bridge ν /
/// value feed notes_in[0] and note_out.owner_binding (lab out_owner = mint dest).
pub fn assert_mint_handoff_continuous(
    mint: &MintEvidenceV0,
    handoff: &SwapSpendHandoffV0,
) -> Result<(), SwapHandoffError> {
    mint.require_settle_openings()?;
    let mint_cm = decode_hex32(&mint.cm_public_hex, "cm_public_hex")?;
    let mint_owner = decode_hex32(
        mint.owner_binding_hex.as_ref().unwrap(),
        "owner_binding_hex",
    )?;
    let mint_rcm = decode_hex32(mint.rcm_hex.as_ref().unwrap(), "rcm_hex")?;
    let bridge_nu = decode_hex32(&mint.bridge_nullifier_hex, "bridge_nullifier_hex")?;

    let note_in = handoff
        .action
        .witness
        .notes_in
        .first()
        .ok_or_else(|| SwapHandoffError::Other("handoff notes_in empty".into()))?;
    if note_in.cm_public != mint_cm {
        return Err(SwapHandoffError::Other(format!(
            "mint cm_public ≠ handoff notes_in[0].cm (mint={} handoff={})",
            mint.cm_public_hex,
            hex32(&note_in.cm_public)
        )));
    }
    if note_in.rcm != mint_rcm {
        return Err(SwapHandoffError::Other(
            "mint rcm ≠ handoff notes_in[0].rcm — openings not continuous".into(),
        ));
    }
    // note_out owner is lab out_owner = mint owner_binding (single dest seal).
    if handoff.action.witness.note_out.owner_binding != mint_owner {
        return Err(SwapHandoffError::Other(format!(
            "mint owner_binding ≠ note_out.owner_binding (mint={} out={})",
            hex32(&mint_owner),
            hex32(&handoff.action.witness.note_out.owner_binding)
        )));
    }
    if handoff.mint.bridge_nullifier_hex != mint.bridge_nullifier_hex {
        return Err(SwapHandoffError::Other(
            "handoff.mint bridge ν ≠ input MintEvidenceV0".into(),
        ));
    }
    for nf in &handoff.action.public.nullifiers {
        if nf == &bridge_nu {
            return Err(SwapHandoffError::NullifierDomain);
        }
    }
    if handoff.mint.value_u64 != mint.value_u64 {
        return Err(SwapHandoffError::Other(
            "handoff.mint value ≠ mint evidence value".into(),
        ));
    }
    Ok(())
}

/// Build settle handoff from mint evidence + lab pool params (G2 spirit builders).
///
/// Continuous path: `MintEvidenceV0` openings → SEAM sketch → `SwapActionV0` (no synthetic NoteIn).
/// Fail-closed on missing openings, empty proof, pool ν == bridge ν.
/// Post-build: [`assert_mint_handoff_continuous`] (MintEvidence ↔ spend openings glue).
///
/// Uses magic lab reserves (`LAB_R_IN`/`LAB_R_OUT`). For omni bridged-LP, use
/// [`build_swap_spend_handoff_from_mint_with_reserves`].
pub fn build_swap_spend_handoff_from_mint(
    mint: &MintEvidenceV0,
    proof_mode: ProofModeLabel,
) -> Result<SwapSpendHandoffV0, SwapHandoffError> {
    build_swap_spend_handoff_from_mint_with_reserves(mint, proof_mode, LAB_R_IN, LAB_R_OUT)
}

/// Build settle handoff against an **existing** pool whose public reserves are
/// `r_in_before` / `r_out_before` (typically LP-seeded BTC/ZEC values).
pub fn build_swap_spend_handoff_from_mint_with_reserves(
    mint: &MintEvidenceV0,
    proof_mode: ProofModeLabel,
    r_in_before: u128,
    r_out_before: u128,
) -> Result<SwapSpendHandoffV0, SwapHandoffError> {
    mint.require_settle_openings()?;
    if r_in_before == 0 || r_out_before == 0 {
        return Err(SwapHandoffError::Other(
            "pool reserves zero — seed LP liquidity first".into(),
        ));
    }
    let mut sketch = seam_sketch_from_mint_evidence(mint, 0)?;
    let asset_in = sketch.asset_id;
    let asset_out = lab_asset_out_zec();
    if asset_in == asset_out {
        return Err(SwapHandoffError::Other(
            "asset_in == asset_out for lab settle".into(),
        ));
    }

    let mut registry = AssetMap::default();
    registry.register(asset_in);
    registry.register(asset_out);

    // Cap delta_in so virtual reserves stay healthy; residual → change note.
    // Omni pools have large R; still cap tiny lab user mints relative to full value.
    let full_value = mint.value_u64 as u128;
    let delta_in = full_value.min(10_000).min(r_in_before / 10).max(1).min(full_value);
    if delta_in == 0 {
        return Err(SwapHandoffError::Other("delta_in=0".into()));
    }

    let out_owner = sketch.owner_binding;
    let out_rcm = {
        let mut r = [0u8; 32];
        r[0] = 0x0A;
        r[1] = 0x0B;
        r
    };
    let mut params = lab_swap_from_seam_params_with_reserves(
        asset_in,
        asset_out,
        Some(delta_in),
        out_owner,
        out_rcm,
        r_in_before,
        r_out_before,
    );

    if full_value > delta_in {
        let mut change_rcm = [0u8; 32];
        change_rcm[0] = 0xCC;
        params.change_rcm = Some(change_rcm);
        // keep sketch.value = full mint value for conservation
    } else {
        sketch.value = delta_in as u64;
        params.change_rcm = None;
    }

    let action = build_swap_action_from_seam_notes(&[sketch], &params, &registry)
        .map_err(SwapHandoffError::Compose)?;

    // NE-4: pool ν ≠ bridge ingress
    let bridge_nu = decode_hex32(&mint.bridge_nullifier_hex, "bridge_nullifier_hex")?;
    for (n, nf) in action
        .witness
        .notes_in
        .iter()
        .zip(action.public.nullifiers.iter())
    {
        if nf == &bridge_nu {
            return Err(SwapHandoffError::NullifierDomain);
        }
        let derived = synthetic_pool_spend_nf(&n.cm_public, &n.rcm);
        if nf != &derived {
            return Err(SwapHandoffError::Other(
                "pool nullifier ≠ synthetic_pool_spend_nf(cm, rcm)".into(),
            ));
        }
    }

    let statement = swap_action_public_to_cw(&action.public);
    if statement.nullifiers.is_empty() {
        return Err(SwapHandoffError::Other(
            "nullifiers empty after compose".into(),
        ));
    }

    let proof = match proof_mode {
        ProofModeLabel::MockVerifyLab => {
            let p = lab_mock_swap_proof();
            if p.is_empty() {
                return Err(SwapHandoffError::EmptyProof);
            }
            p
        }
        ProofModeLabel::ZkApi => {
            return Err(SwapHandoffError::Other(
                "zk_api proof mode residual — not funded default".into(),
            ));
        }
    };

    let pool_nullifiers_hex = action.public.nullifiers.iter().map(|n| hex32(n)).collect();

    let handoff = SwapSpendHandoffV0 {
        mint: mint.clone(),
        statement,
        pool_nullifiers_hex,
        proof,
        proof_mode,
        action,
    };
    // Evidence glue: MintEvidenceV0 openings continuous into settle spend handoff.
    assert_mint_handoff_continuous(mint, &handoff)?;
    Ok(handoff)
}

/// Expected host quote for lab reserves (preflight / multitest assert).
pub fn lab_quote_delta_out(delta_in: u128) -> Result<u128, SwapActionError> {
    quote_exact_in(LAB_R_IN, LAB_R_OUT, delta_in, LAB_GAMMA, LAB_GAMMA_DEN)
        .map_err(|_| SwapActionError::ErrBadAmount)
}

/// Quote against explicit pool reserves (omni LP-seeded pool).
pub fn quote_delta_out_with_reserves(
    r_in: u128,
    r_out: u128,
    delta_in: u128,
) -> Result<u128, SwapActionError> {
    quote_exact_in(r_in, r_out, delta_in, LAB_GAMMA, LAB_GAMMA_DEN)
        .map_err(|_| SwapActionError::ErrBadAmount)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::mint_evidence::mint_evidence_from_claim_fields;

    fn fixture_mint() -> MintEvidenceV0 {
        // Distinct asset_in (not asset_b)
        let mut asset_in = [0u8; 32];
        asset_in[0] = b'H';
        asset_in[1] = b'U';
        asset_in[2] = b'B';
        mint_evidence_from_claim_fields(
            "ict_local_funded",
            "test-1",
            "terp1hs",
            &[0x11; 32],
            &[0x22; 32],
            5_000,
            &asset_in,
            Some(&[0x33; 32]),
            Some(&[0x44; 32]),
            true,
            None,
            None,
        )
    }

    #[test]
    fn map_public_fields_equal() {
        let mint = fixture_mint();
        let handoff =
            build_swap_spend_handoff_from_mint(&mint, ProofModeLabel::MockVerifyLab).unwrap();
        let p = &handoff.action.public;
        let cw = &handoff.statement;
        assert_eq!(cw.pool_id, p.pool_id);
        assert_eq!(cw.asset_in.as_slice(), p.asset_in.as_slice());
        assert_eq!(cw.asset_out.as_slice(), p.asset_out.as_slice());
        assert_eq!(cw.root.as_slice(), p.root.as_slice());
        assert_eq!(cw.nullifiers.len(), p.nullifiers.len());
        assert_eq!(cw.cm_out.len(), p.cm_out.len());
        assert_eq!(cw.delta_r_in.u128(), p.delta_r_in);
        assert_eq!(cw.delta_r_out.u128(), p.delta_r_out);
        assert_eq!(cw.min_out.u128(), p.min_out);
        assert_eq!(cw.gamma, p.gamma);
        assert_eq!(cw.r_in_before.u128(), p.r_in_before);
        // now_height not on CW statement
        assert_eq!(handoff.proof.as_slice(), LAB_MOCK_SWAP_PROOF);
        assert_eq!(handoff.proof_mode, ProofModeLabel::MockVerifyLab);
        // Continuity: spent mint cm
        assert_eq!(
            handoff.action.witness.notes_in[0].cm_public.as_slice(),
            decode_hex32(&mint.cm_public_hex, "cm").unwrap().as_slice()
        );
        // Pool ν ≠ bridge
        let bridge = decode_hex32(&mint.bridge_nullifier_hex, "nu").unwrap();
        assert_ne!(handoff.action.public.nullifiers[0], bridge);
    }

    #[test]
    fn mint_handoff_continuous_dest_and_cm() {
        let mint = fixture_mint();
        let handoff =
            build_swap_spend_handoff_from_mint(&mint, ProofModeLabel::MockVerifyLab).unwrap();
        assert_mint_handoff_continuous(&mint, &handoff).unwrap();
        assert_eq!(
            handoff.action.witness.note_out.owner_binding,
            decode_hex32(mint.owner_binding_hex.as_ref().unwrap(), "ob").unwrap()
        );
        assert_eq!(
            handoff.action.witness.notes_in[0].cm_public,
            decode_hex32(&mint.cm_public_hex, "cm").unwrap()
        );
    }

    #[test]
    fn missing_rcm_fail_closed() {
        let mut mint = fixture_mint();
        mint.rcm_hex = None;
        let err = build_swap_spend_handoff_from_mint(&mint, ProofModeLabel::MockVerifyLab)
            .unwrap_err();
        assert!(err.to_string().contains("rcm") || err.to_string().contains("openings"));
    }

    #[test]
    fn lab_quote_matches_action_delta() {
        let mint = fixture_mint();
        let handoff =
            build_swap_spend_handoff_from_mint(&mint, ProofModeLabel::MockVerifyLab).unwrap();
        let q = lab_quote_delta_out(handoff.action.public.delta_r_in).unwrap();
        assert_eq!(q, handoff.action.public.delta_r_out);
    }
}
