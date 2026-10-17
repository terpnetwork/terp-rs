use ff::Field;
use pasta_curves::pallas;
use subtle::{ConstantTimeEq, CtOption};

use crate::{
    address::RecpAddr,
    keys::EligibleSk,
    note_poseidon::{lift_note_cmx, poseidon_note_cmx, rcm_to_base},
    value::NoteValue,
};

#[derive(Clone, Debug)]
pub(crate) struct NoteCommitTrapdoor(pub(super) pallas::Scalar);

impl NoteCommitTrapdoor {
    pub(crate) fn inner(&self) -> pallas::Scalar {
        self.0
    }
}

/// A commitment to a note (Poseidon-v1 + lift).
///
/// - **`cmx`**: Poseidon digest (public extracted commitment).
/// - **`point`**: `[cmx] · NoteCommitR` for nullifier ECC continuity (ADR option A).
#[derive(Clone, Debug)]
pub struct NoteCommitment {
    pub(super) point: pallas::Point,
    pub(super) cmx: pallas::Base,
}

impl NoteCommitment {
    /// Curve point form used by nullifier derivation.
    pub(crate) fn inner(&self) -> pallas::Point {
        self.point
    }

    /// Poseidon-v1 note commitment digest (`cmx`).
    pub(crate) fn cmx(&self) -> pallas::Base {
        self.cmx
    }
}

impl NoteCommitment {
    /// Poseidon-v1 private note commitment (ADR-POSEIDON-NOTE-COMMIT).
    ///
    /// ```text
    /// cmx = Poseidon_CL<9>(tag, nd, v, fdi, recp, esk, rho, psi, rcm_base)
    /// cm_point = [cmx] · NoteCommitR
    /// ```
    ///
    /// `rcm_base` encoding: [`crate::note_poseidon::rcm_to_base`].
    pub(super) fn derive(
        nd: pallas::Base,
        v: NoteValue,
        fdi: pallas::Base,
        recp: RecpAddr,
        esk: EligibleSk,
        rho: pallas::Base,
        psi: pallas::Base,
        rcm: NoteCommitTrapdoor,
    ) -> CtOption<Self> {
        // The eligible key is not a note-commit input. Ownership is the signature.
        let _ = esk;
        let esk = pallas::Base::ZERO;
        let recp = recp.to_fp();
        let rcm_base = rcm_to_base(rcm.0);
        let v_base = pallas::Base::from(v.inner());

        let cmx = poseidon_note_cmx(nd, v_base, fdi, recp, esk, rho, psi, rcm_base);
        let point = lift_note_cmx(cmx);
        CtOption::new(NoteCommitment { point, cmx }, 1.into())
    }
}

/// The public note commitment digest (`cmx`).
///
/// Under Poseidon-v1 this is the Poseidon output itself (not `extract_p` of a Sinsemilla point).
#[derive(Copy, Clone, Debug)]
pub struct ExtractedNoteCommitment(pub(super) pallas::Base);

impl ExtractedNoteCommitment {
    /// Deserialize the extracted note commitment from a byte array.
    ///
    /// This method enforces the [consensus rule][cmxcanon] that the
    /// byte representation of cmx MUST be canonical.
    ///
    /// [cmxcanon]: https://zips.z.cash/protocol/protocol.pdf#actionencodingandconsensus
    pub fn from_bytes(bytes: &[u8; 32]) -> CtOption<Self> {
        pallas::Base::from_repr(*bytes).map(ExtractedNoteCommitment)
    }

    /// Serialize the value commitment to its canonical byte representation.
    pub fn to_bytes(self) -> [u8; 32] {
        self.0.to_repr()
    }
}

impl From<NoteCommitment> for ExtractedNoteCommitment {
    fn from(cm: NoteCommitment) -> Self {
        ExtractedNoteCommitment(cm.cmx)
    }
}

impl ExtractedNoteCommitment {
    pub(crate) fn inner(&self) -> pallas::Base {
        self.0
    }
}

impl From<&ExtractedNoteCommitment> for [u8; 32] {
    fn from(cmx: &ExtractedNoteCommitment) -> Self {
        cmx.to_bytes()
    }
}

impl ConstantTimeEq for ExtractedNoteCommitment {
    fn ct_eq(&self, other: &Self) -> subtle::Choice {
        self.0.ct_eq(&other.0)
    }
}

impl PartialEq for ExtractedNoteCommitment {
    fn eq(&self, other: &Self) -> bool {
        self.ct_eq(other).into()
    }
}

impl Eq for ExtractedNoteCommitment {}

// Re-export PrimeField for from_bytes.
use ff::PrimeField;

#[cfg(test)]
mod packing_tests {
    use super::*;
    use crate::note::Note;
    use crate::note_poseidon::{lift_note_cmx, poseidon_note_cmx, rcm_to_base};
    use group::Curve;
    use crate::os_rng;

    #[test]
    fn derive_matches_poseidon_ssot_for_dummy_note() {
        let mut rng = os_rng();
        let (_sk, _fvk, esk, note) = Note::dummy(&mut rng, None);
        let rho = note.rho();
        let psi = note.rseed().psi(&rho);
        let rcm = note.rseed().rcm(&rho);
        let derived = note.commitment();

        let rcm_base = rcm_to_base(rcm.inner());
        let cmx = poseidon_note_cmx(
            note.nd().to_fp(),
            pallas::Base::from(note.value().inner()),
            pallas::Base::from(note.fdi()),
            note.recipient().to_fp(),
            esk.derive_pallas(),
            rho.into_inner(),
            psi,
            rcm_base,
        );
        let point = lift_note_cmx(cmx);

        assert_eq!(
            derived.cmx(),
            cmx,
            "NoteCommitment::derive cmx must match poseidon_note_cmx SSOT"
        );
        assert_eq!(
            derived.inner().to_affine(),
            point.to_affine(),
            "NoteCommitment::derive point must match lift_note_cmx"
        );
        assert_eq!(
            ExtractedNoteCommitment::from(derived).inner(),
            cmx,
            "ExtractedNoteCommitment must be Poseidon cmx (not extract_p of lift)"
        );
    }

    #[test]
    fn rcm_base_encoding_documented() {
        // Small scalars: identity via from_repr.
        let small = pallas::Scalar::from(99u64);
        assert_eq!(rcm_to_base(small), pallas::Base::from(99u64));

        // Random rseed-derived rcm still yields a stable base word.
        let mut rng = os_rng();
        let (_sk, _fvk, _esk, note) = Note::dummy(&mut rng, None);
        let rcm = note.rseed().rcm(&note.rho());
        let a = rcm_to_base(rcm.inner());
        let b = rcm_to_base(rcm.inner());
        assert_eq!(a, b);
    }

    #[test]
    fn esk_native_matches_derive_pallas() {
        use crate::spec::esk_to_base;
        let mut rng = os_rng();
        let (_sk, _fvk, esk, _note) = Note::dummy(&mut rng, None);
        let via_derive = esk.derive_pallas();
        let via_esk_to_base = esk_to_base(&esk);
        assert_eq!(via_derive, via_esk_to_base);
        // Guest-safe reduction: BE secret_bytes → BigUint → pallas (matches Fq LE limbs).
        let sk_big = num_bigint::BigUint::from_bytes_be(&esk.secret_bytes());
        let via_big = crate::spec::biguint_to_fe_simple(&sk_big);
        assert_eq!(via_derive, via_big, "CRT-style native must match derive_pallas");
    }
}
