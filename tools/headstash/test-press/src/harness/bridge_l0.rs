//! L0 pure re-exports: `bridge_auth_seams` authorize + thin SEAM compose.
//!
//! Maps to E2E-01 / E2E-02 / E2E-03 at **min layer L0** (no CosmWasm, no Docker).
//! CW bridge-mint on `cw-headstash` (CLARITY) may wire the same assertions later.

use terp_seams::bridge::{
    authorize_bridge_mint, authorize_bridge_mint_apply, hinge_happy_fixture, BridgeMintError,
    NoteOutSketch, ORIGIN_BRIDGE_MINT, NF_BRIDGE_BURN,
};

/// L0 harness error (stringly for suite → CwOrchError mapping).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct L0Error(pub String);

impl std::fmt::Display for L0Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for L0Error {}

impl From<BridgeMintError> for L0Error {
    fn from(e: BridgeMintError) -> Self {
        L0Error(format!("{e:?}"))
    }
}

/// E2E-01 min-layer: happy burn → authorize → NoteOutSketch (bridge origin).
pub fn assert_policy_bridge_mint_happy() -> Result<NoteOutSketch, L0Error> {
    let (snapshot, claim, mut minted, registry, terp_asset) = hinge_happy_fixture();
    let note = authorize_bridge_mint_apply(&snapshot, &claim, &mut minted, &registry)?;
    if note.origin != ORIGIN_BRIDGE_MINT {
        return Err(L0Error(format!(
            "expected ORIGIN_BRIDGE_MINT, got {}",
            note.origin
        )));
    }
    if note.nullifier_domain != NF_BRIDGE_BURN {
        return Err(L0Error(format!(
            "expected NF_BRIDGE_BURN, got {}",
            note.nullifier_domain
        )));
    }
    if note.value != claim.public.value_u64 {
        return Err(L0Error("value conservation fail".into()));
    }
    if note.asset_id != terp_asset {
        return Err(L0Error("asset_id map fail".into()));
    }
    if !minted.contains(&claim.public.nullifier) {
        return Err(L0Error("minted set not updated".into()));
    }
    Ok(note)
}

/// E2E-03 / H-1: ordinary spent-only ν must not authorize mint.
pub fn assert_h1_spent_only_reject() -> Result<(), L0Error> {
    let (snapshot, mut claim, minted, registry, _) = hinge_happy_fixture();
    claim.in_burn_set = false;
    claim.spent_only = true;
    match authorize_bridge_mint(&snapshot, &claim, &minted, &registry) {
        Err(BridgeMintError::NotInBurnSet) => Ok(()),
        other => Err(L0Error(format!(
            "H-1 expected NotInBurnSet, got {other:?}"
        ))),
    }
}

/// E2E-02: second mint same ν rejects (`AlreadyMinted`).
pub fn assert_double_mint_reject() -> Result<(), L0Error> {
    let (snapshot, claim, mut minted, registry, _) = hinge_happy_fixture();
    authorize_bridge_mint_apply(&snapshot, &claim, &mut minted, &registry)?;
    match authorize_bridge_mint(&snapshot, &claim, &minted, &registry) {
        Err(BridgeMintError::AlreadyMinted) => Ok(()),
        other => Err(L0Error(format!(
            "double mint expected AlreadyMinted, got {other:?}"
        ))),
    }
}

/// Thin compose: authorized sketch serializes as SEAM-NOTE-OUT V0 (382 bytes)
/// and is DEX-consumable when rcm was present on the hinge public packet.
pub fn compose_bridge_mint_to_seam_bytes() -> Result<NoteOutSketch, L0Error> {
    let note = assert_policy_bridge_mint_happy()?;
    let bytes = note.to_seam_bytes();
    if bytes.len() != 382 {
        return Err(L0Error(format!("seam bytes len {}", bytes.len())));
    }
    if note.rcm_flag == 1 && !note.is_dex_consumable() {
        return Err(L0Error(
            "rcm_flag=1 but is_dex_consumable=false".into(),
        ));
    }
    // Optional cross-crate decode when seam_note_out is linked.
    #[cfg(feature = "l0-seams")]
    {
        use terp_seams::dex::note::SeamNoteOutV0;
        let seam = SeamNoteOutV0::from_bytes(&bytes).map_err(|e| L0Error(format!("{e:?}")))?;
        if seam.origin != ORIGIN_BRIDGE_MINT {
            return Err(L0Error("seam origin mismatch".into()));
        }
        if seam.nullifier_domain != NF_BRIDGE_BURN {
            return Err(L0Error("seam nf domain mismatch".into()));
        }
        if seam.value != note.value {
            return Err(L0Error("seam value mismatch".into()));
        }
    }
    Ok(note)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e2e01_policy_bridge_mint_happy() {
        let note = assert_policy_bridge_mint_happy().expect("E2E-01 L0");
        assert_eq!(note.origin, ORIGIN_BRIDGE_MINT);
        assert!(note.is_dex_consumable());
    }

    #[test]
    fn e2e03_h1_spent_only_reject() {
        assert_h1_spent_only_reject().expect("E2E-03 L0");
    }

    #[test]
    fn e2e02_double_mint_reject() {
        assert_double_mint_reject().expect("E2E-02 L0");
    }

    #[test]
    fn compose_sketch_seam_bytes() {
        compose_bridge_mint_to_seam_bytes().expect("compose L0");
    }
}
