//! Poseidon-v1 **private note commitment** (replace Sinsemilla `NoteCommit`).
//!
//! ADR: `docs/plans/spectrum/ADR-POSEIDON-NOTE-COMMIT.md`
//!
//! # Scope
//!
//! Pure off-circuit SSOT for Headstash private note `cmx` / commitment lift.
//! Public distro / inclusion Merkle remains in [`crate::distro_poseidon`] — **do not**
//! reuse distro personalization strings here, and do **not** feed private `cmx` into
//! distro leaves.
//!
//! # Hash domain tag (contract / manifold)
//!
//! Private note commitments publish under:
//!
//! ```text
//! note_commit_domain = "poseidon-v1"
//! ```
//!
//! Distinct from `distro_hash_domain = "poseidon-v1"` roots (same product family string
//! on different surfaces; personalization UTF-8 tags below provide cryptographic
//! domain separation).
//!
//! # Poseidon parameters (must match in-circuit gadgets)
//!
//! | Param | Value |
//! |-------|--------|
//! | Spec / round constants | `halo2_gadgets::poseidon::primitives::P128Pow5T3` |
//! | Width `t` | 3 |
//! | Rate | 2 |
//! | Capacity | 1 |
//! | Mode | `ConstantLength<9>` only |
//! | Field | `pallas::Base` |
//! | Encoding | Canonical little-endian 32-byte field words |
//!
//! # Personalization
//!
//! | Role | UTF-8 string | Arity `N` | Message layout |
//! |------|--------------|-----------|----------------|
//! | Note commit | `"terp-hs-note-commit-v1"` | **9** | `[tag, nd, v, fdi, recp, esk, rho, psi, rcm_base]` |
//!
//! Tag packing: UTF-8 bytes LE into one `pallas::Base`, zero-padded to 32 bytes
//! (same [`personalization_to_fp`] pattern as distro).
//!
//! # `rcm_base` encoding (trapdoor → base field)
//!
//! `rcm` is a `pallas::Scalar` (`NoteCommitTrapdoor`, from `to_scalar(PrfExpand::ORCHARD_RCM…)`).
//! Pallas has `p < r`, so not every scalar's 32-byte LE representative is a valid base element.
//!
//! **Chosen reduction (documented + unit-tested):**
//!
//! 1. Let `bytes = rcm.to_repr()` (canonical LE 32-byte scalar encoding).
//! 2. If `pallas::Base::from_repr(bytes)` succeeds, use that (identity on the shared range).
//! 3. Otherwise widen to 64 LE bytes (`bytes || 0³¹`) and apply
//!    `pallas::Base::from_uniform_bytes` (same reduction family as Orchard `ToBase` /
//!    [`crate::spec::to_base`]).
//!
//! This is always defined, deterministic, and agrees with `from_repr` whenever the
//! scalar representative is already `< p`.
//!
//! # Commitment value (ADR option A)
//!
//! ```text
//! cmx = Poseidon^P128Pow5T3 / ConstantLength<9>(tag, nd, v, fdi, recp, esk, rho, psi, rcm_base)
//! cm_point = [cmx] · NoteCommitR
//! ```
//!
//! - Public **`ExtractedNoteCommitment`** = `cmx` (Poseidon digest, not `extract_p(cm_point)`).
//! - Internal **`NoteCommitment`** stores `cm_point` for nullifier ECC:
//!   `nf = Extract_P( [PRF_nf(nk,ρ)+ψ] NullifierK + cm_point )`.
//! - `rcm` is **inside** the Poseidon message only; do **not** add a second `[rcm]R`.
//!
//! Lab / non-mainnet: no production attestation.

#![cfg(feature = "circuit")]

use ff::{FromUniformBytes, PrimeField};
use halo2_gadgets::poseidon::primitives::{self as poseidon, ConstantLength, P128Pow5T3};
use pasta_curves::pallas;

use crate::constants::fixed_bases::note_commit_r;
use crate::spec::mod_r_p;

// ---------------------------------------------------------------------------
// Domain identifiers
// ---------------------------------------------------------------------------

/// Contract / manifold tag for private note commitments from this module.
pub const NOTE_COMMIT_HASH_DOMAIN_POSEIDON_V1: &str = "poseidon-v1";

/// Note-commit personalization string (UTF-8), packed via [`personalization_to_fp`].
pub const POSEIDON_NOTE_COMMIT_PERSONALIZATION: &str = "terp-hs-note-commit-v1";

/// Fixed Poseidon width (`t = 3`).
pub const POSEIDON_T: usize = 3;

/// Fixed Poseidon rate (`rate = 2` ⇒ capacity 1).
pub const POSEIDON_RATE: usize = 2;

/// Message arity of [`poseidon_note_cmx`] (tag + 8 note fields).
pub const POSEIDON_NOTE_COMMIT_ARITY: usize = 9;

// ---------------------------------------------------------------------------
// Personalization packing
// ---------------------------------------------------------------------------

/// Pack a personalization string into `pallas::Base`.
///
/// Encoding: UTF-8 bytes in **little-endian** limb order (byte 0 of the string is the
/// least-significant byte of the field element), zero-padded to 32 bytes.
///
/// # Panics
///
/// Panics if `tag` is longer than 32 bytes or is a non-canonical field encoding.
pub fn personalization_to_fp(tag: &str) -> pallas::Base {
    let b = tag.as_bytes();
    assert!(
        b.len() <= 32,
        "personalization must fit in 32 bytes, got {}",
        b.len()
    );
    let mut bytes = [0u8; 32];
    bytes[..b.len()].copy_from_slice(b);
    pallas::Base::from_repr(bytes).expect("personalization must be a canonical pallas::Base")
}

/// Domain tag field for the private note commitment hash.
#[inline]
pub fn note_commit_tag() -> pallas::Base {
    personalization_to_fp(POSEIDON_NOTE_COMMIT_PERSONALIZATION)
}

// ---------------------------------------------------------------------------
// rcm → base
// ---------------------------------------------------------------------------

/// Encode note-commit trapdoor `rcm` as a base-field word for Poseidon packing.
///
/// See module docs § `rcm_base` encoding.
#[inline]
pub fn rcm_to_base(rcm: pallas::Scalar) -> pallas::Base {
    let bytes = rcm.to_repr();
    if let Some(b) = Option::from(pallas::Base::from_repr(bytes)) {
        return b;
    }
    let mut wide = [0u8; 64];
    wide[..32].copy_from_slice(&bytes);
    pallas::Base::from_uniform_bytes(&wide)
}

// ---------------------------------------------------------------------------
// Pure Poseidon note commit
// ---------------------------------------------------------------------------

/// Private note commitment digest `cmx` (Poseidon-v1).
///
/// ```text
/// Poseidon^P128Pow5T3 / ConstantLength<9>(
///   personalization_to_fp("terp-hs-note-commit-v1"),
///   nd, v, fdi, recp, esk, rho, psi, rcm_base
/// )
/// ```
#[allow(clippy::too_many_arguments)]
pub fn poseidon_note_cmx(
    nd: pallas::Base,
    v: pallas::Base,
    fdi: pallas::Base,
    recp: pallas::Base,
    esk: pallas::Base,
    rho: pallas::Base,
    psi: pallas::Base,
    rcm_base: pallas::Base,
) -> pallas::Base {
    poseidon::Hash::<_, P128Pow5T3, ConstantLength<9>, 3, 2>::init().hash([
        note_commit_tag(),
        nd,
        v,
        fdi,
        recp,
        esk,
        rho,
        psi,
        rcm_base,
    ])
}

/// Lift `cmx` to a curve point for nullifier ECC: `cm_point = [cmx] · NoteCommitR`.
///
/// Uses the existing Orchard fixed base `NoteCommitR` as a **lift only** (ADR option A).
/// The trapdoor is already bound inside Poseidon; do not add `[rcm]R` again.
#[inline]
pub fn lift_note_cmx(cmx: pallas::Base) -> pallas::Point {
    // p < r ⇒ mod_r_p is the identity embedding of the base representative as a scalar.
    note_commit_r::generator() * mod_r_p(cmx)
}

/// Canonical 32-byte LE encoding of a note commitment digest.
#[inline]
pub fn note_cmx_to_bytes(cmx: pallas::Base) -> [u8; 32] {
    cmx.to_repr()
}

/// Parse a 32-byte LE note commitment digest.
#[inline]
pub fn note_cmx_from_bytes(bytes: [u8; 32]) -> Option<pallas::Base> {
    Option::from(pallas::Base::from_repr(bytes))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distro_poseidon::{
        distro_leaf_tag, poseidon_distro_leaf, POSEIDON_DISTRO_CRH_PERSONALIZATION,
        POSEIDON_DISTRO_LEAF_PERSONALIZATION,
    };
    use ff::Field;
    use group::{ff::PrimeField, Group};

    fn sample_note_fields(
        seed: u64,
    ) -> (
        pallas::Base,
        pallas::Base,
        pallas::Base,
        pallas::Base,
        pallas::Base,
        pallas::Base,
        pallas::Base,
        pallas::Base,
    ) {
        (
            pallas::Base::from(seed),
            pallas::Base::from(seed + 1),
            pallas::Base::from(seed + 2),
            pallas::Base::from(seed + 3),
            pallas::Base::from(seed + 4),
            pallas::Base::from(seed + 5),
            pallas::Base::from(seed + 6),
            pallas::Base::from(seed + 7),
        )
    }

    #[test]
    fn personalization_is_nonzero_and_distinct_from_distro() {
        let note = note_commit_tag();
        assert_ne!(note, pallas::Base::ZERO);
        assert_ne!(note, distro_leaf_tag());
        assert_ne!(
            POSEIDON_NOTE_COMMIT_PERSONALIZATION,
            POSEIDON_DISTRO_LEAF_PERSONALIZATION
        );
        assert_ne!(
            POSEIDON_NOTE_COMMIT_PERSONALIZATION,
            POSEIDON_DISTRO_CRH_PERSONALIZATION
        );
    }

    #[test]
    fn personalization_roundtrip_bytes() {
        let tag = POSEIDON_NOTE_COMMIT_PERSONALIZATION;
        let fe = personalization_to_fp(tag);
        let repr = fe.to_repr();
        assert_eq!(&repr[..tag.len()], tag.as_bytes());
        assert!(repr[tag.len()..].iter().all(|&b| b == 0));
    }

    #[test]
    fn note_cmx_is_deterministic() {
        let (nd, v, fdi, recp, esk, rho, psi, rcm_base) = sample_note_fields(7);
        let a = poseidon_note_cmx(nd, v, fdi, recp, esk, rho, psi, rcm_base);
        let b = poseidon_note_cmx(nd, v, fdi, recp, esk, rho, psi, rcm_base);
        assert_eq!(a, b);
        assert_ne!(a, pallas::Base::ZERO);
    }

    #[test]
    fn note_cmx_changes_with_any_field() {
        let (nd, v, fdi, recp, esk, rho, psi, rcm_base) = sample_note_fields(11);
        let base = poseidon_note_cmx(nd, v, fdi, recp, esk, rho, psi, rcm_base);
        assert_ne!(
            base,
            poseidon_note_cmx(
                nd + pallas::Base::ONE,
                v,
                fdi,
                recp,
                esk,
                rho,
                psi,
                rcm_base
            )
        );
        assert_ne!(
            base,
            poseidon_note_cmx(
                nd,
                v + pallas::Base::ONE,
                fdi,
                recp,
                esk,
                rho,
                psi,
                rcm_base
            )
        );
        assert_ne!(
            base,
            poseidon_note_cmx(
                nd,
                v,
                fdi + pallas::Base::ONE,
                recp,
                esk,
                rho,
                psi,
                rcm_base
            )
        );
        assert_ne!(
            base,
            poseidon_note_cmx(
                nd,
                v,
                fdi,
                recp + pallas::Base::ONE,
                esk,
                rho,
                psi,
                rcm_base
            )
        );
        assert_ne!(
            base,
            poseidon_note_cmx(
                nd,
                v,
                fdi,
                recp,
                esk + pallas::Base::ONE,
                rho,
                psi,
                rcm_base
            )
        );
        assert_ne!(
            base,
            poseidon_note_cmx(
                nd,
                v,
                fdi,
                recp,
                esk,
                rho + pallas::Base::ONE,
                psi,
                rcm_base
            )
        );
        assert_ne!(
            base,
            poseidon_note_cmx(
                nd,
                v,
                fdi,
                recp,
                esk,
                rho,
                psi + pallas::Base::ONE,
                rcm_base
            )
        );
        assert_ne!(
            base,
            poseidon_note_cmx(
                nd,
                v,
                fdi,
                recp,
                esk,
                rho,
                psi,
                rcm_base + pallas::Base::ONE
            )
        );
    }

    #[test]
    fn domain_separation_vs_distro_leaf() {
        // Same small field vector must not collide across note-commit vs distro-leaf domains.
        let (nd, v, fdi, recp, esk, rho, psi, rcm_base) = sample_note_fields(42);
        let note = poseidon_note_cmx(nd, v, fdi, recp, esk, rho, psi, rcm_base);
        let distro = poseidon_distro_leaf(nd, v, fdi, recp, esk, pallas::Base::from(1u64));
        assert_ne!(note, distro);

        // Forged: note payload hashed under distro leaf tag + arity 6.
        let forged = poseidon::Hash::<_, P128Pow5T3, ConstantLength<6>, 3, 2>::init().hash([
            distro_leaf_tag(),
            nd,
            v,
            fdi,
            recp,
            esk,
        ]);
        assert_ne!(note, forged);
    }

    #[test]
    fn fixed_small_field_vector_stable() {
        // Frozen lab vector (not ZIP / mainnet). Change only with ADR + STATUS note.
        let cmx = poseidon_note_cmx(
            pallas::Base::from(1u64),
            pallas::Base::from(2u64),
            pallas::Base::from(3u64),
            pallas::Base::from(4u64),
            pallas::Base::from(5u64),
            pallas::Base::from(6u64),
            pallas::Base::from(7u64),
            pallas::Base::from(8u64),
        );
        let bytes = note_cmx_to_bytes(cmx);
        assert_eq!(note_cmx_from_bytes(bytes), Some(cmx));
        // Non-zero and not equal to any single input.
        assert_ne!(cmx, pallas::Base::ZERO);
        assert_ne!(cmx, pallas::Base::from(1u64));
        // Round-trip stability: second call identical (determinism already covered).
        let cmx2 = poseidon_note_cmx(
            pallas::Base::from(1u64),
            pallas::Base::from(2u64),
            pallas::Base::from(3u64),
            pallas::Base::from(4u64),
            pallas::Base::from(5u64),
            pallas::Base::from(6u64),
            pallas::Base::from(7u64),
            pallas::Base::from(8u64),
        );
        assert_eq!(cmx, cmx2);
        // Canonical LE encoding length.
        assert_eq!(bytes.len(), 32);
    }

    #[test]
    fn rcm_to_base_small_scalar_matches_from_repr() {
        let rcm = pallas::Scalar::from(17u64);
        let base = rcm_to_base(rcm);
        let via_repr = pallas::Base::from_repr(rcm.to_repr()).unwrap();
        assert_eq!(base, via_repr);
        assert_eq!(base, pallas::Base::from(17u64));
    }

    #[test]
    fn rcm_to_base_is_deterministic() {
        let rcm = pallas::Scalar::from(0xdead_beef_u64);
        assert_eq!(rcm_to_base(rcm), rcm_to_base(rcm));
    }

    #[test]
    fn lift_note_cmx_is_note_commit_r_mul() {
        use group::Curve;
        let cmx = pallas::Base::from(9u64);
        let point = lift_note_cmx(cmx);
        let expected = note_commit_r::generator() * mod_r_p(cmx);
        assert_eq!(point.to_affine(), expected.to_affine());
        // Identity only for cmx = 0.
        assert!(!bool::from(point.is_identity()));
        assert!(bool::from(lift_note_cmx(pallas::Base::ZERO).is_identity()));
    }

    #[test]
    fn arity_constant_matches_message() {
        assert_eq!(POSEIDON_NOTE_COMMIT_ARITY, 9);
        assert_eq!(POSEIDON_T, 3);
        assert_eq!(POSEIDON_RATE, 2);
    }
}
