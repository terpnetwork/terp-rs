//! Poseidon-v1 **public distribution / inclusion** Merkle set.
//!
//! ADR: `docs/plans/spectrum/ADR-POSEIDON-DISTRO-TREE.md`
//!
//! # Scope
//!
//! This module is the **pure off-circuit** (and future in-circuit) hash SSOT for the
//! **public eligibility / distribution tree only**. Private note commitment is Poseidon-v1
//! under [`crate::note_poseidon`] (ADR-POSEIDON-NOTE-COMMIT) — separate personalizations.
//!
//! # Hash domain tag (contract / manifold)
//!
//! New roots must be published under:
//!
//! ```text
//! distro_hash_domain = "poseidon-v1"
//! ```
//!
//! Root bytes are the **canonical little-endian 32-byte** encoding of a `pallas::Base`
//! element (`PrimeField::to_repr`), identical to [`crate::tree::Anchor::to_bytes`].
//! Contracts must **not** re-interpret Sinsemilla leaf / CRH personalizations for
//! Poseidon-v1 roots.
//!
//! # Poseidon parameters (must match in-circuit gadgets)
//!
//! | Param | Value |
//! |-------|--------|
//! | Spec / round constants | `halo2_gadgets::poseidon::primitives::P128Pow5T3` (Orchard-compatible) |
//! | Width `t` | 3 |
//! | Rate | 2 |
//! | Capacity | 1 |
//! | Mode | `ConstantLength<N>` sponge (same padding as nullifier PRF / `hdkf_pallas`) |
//!
//! # Personalizations (domain separation)
//!
//! Domain tags are **not** Sinsemilla SWU personalizations. They are fixed UTF-8
//! strings packed into a single `pallas::Base` as **little-endian bytes**, zero-padded
//! to 32 bytes, then used as the **first message element** of a `ConstantLength` Poseidon
//! hash (same pattern as `DST_HKDF` in [`crate::spec::hdkf_pallas`]).
//!
//! | Role | String | Message arity `N` | Layout |
//! |------|--------|-------------------|--------|
//! | Leaf | `"terp-hs-distro-leaf-v1"` | 6 | `[tag, epk_x, epk_y, nd, v, fdi]` |
//! | CRH  | `"terp-hs-distro-crh-v1"`  | 4 | `[tag, layer, left, right]` |
//!
//! ## Field packing
//!
//! All inputs are full `pallas::Base` field elements (no Sinsemilla bit-slicing):
//!
//! - **`epk_x`, `epk_y`**: affine secp256k1 eligibility pubkey coordinates reduced /
//!   witnessed as Pasta base-field elements (same native values the suite already uses
//!   for leaf construction). Use the **full y coordinate**, not the Sinsemilla 1-bit
//!   packing.
//! - **`nd`**: note denom field form (e.g. blake3-trimmed `NoteDenom` → `Fp`).
//! - **`v`, `fdi`**: `u64` amounts / indices as `pallas::Base::from(u64)`.
//! - **`layer`**: `u32` as `pallas::Base::from(u64::from(layer))`.
//!   Convention matches the suite tree builder: **`layer = 0` hashes two leaves**;
//!   layer increases by 1 toward the root. (This is **not** Orchard’s
//!   `MERKLE_DEPTH - layer - 1` `l` encoding; Poseidon-v1 distro trees use the suite’s
//!   ascending counter.)
//!
//! Empty / missing siblings for padding incomplete levels: **`pallas::Base::ZERO`**
//! (not Orchard’s uncommitted leaf `2`, which is Sinsemilla note-tree specific).
//!
//! # In-circuit gadget migration notes
//!
//! Circuit Part I should call the **same packing** as these pure helpers:
//!
//! 1. Witness / constrain `tag_leaf = personalization_to_fp(POSEIDON_DISTRO_LEAF_PERSONALIZATION)`
//!    as a **fixed** constant (copy into advice or use a constant assignment; do not
//!    let the prover choose the tag).
//! 2. Run `PoseidonHash::<_, P128Pow5T3, ConstantLength<6>, 3, 2>` over
//!    `[tag_leaf, epk_x, epk_y, nd, v, fdi]` with the existing `Pow5Chip` /
//!    `poseidon_config` already configured in `Circuit` (width 3, rate 2).
//! 3. For each Merkle step, hash `[tag_crh, Base::from(layer), left, right]` with
//!    `ConstantLength<4>`; `layer` is a public path-level counter starting at 0 at
//!    the leaves (or an assigned constant per level).
//! 4. Constrain the final digest against the public `Anchor` instance (still 32-byte
//!    LE field encoding on-chain).
//! 5. **Do not** feed Poseidon-v1 leaves into Sinsemilla `MerkleChip` paths, and do not
//!    mix Sinsemilla roots with `distro_hash_domain = poseidon-v1`.
//!
//! Keep note-commit / nullifier Poseidon (`prf_nf`) domains separate; they use
//! different arities and **no** distro personalization strings.

#![cfg(feature = "circuit")]

use ff::PrimeField;
use halo2_gadgets::poseidon::primitives::{self as poseidon, ConstantLength, P128Pow5T3};
use pasta_curves::pallas;

// ---------------------------------------------------------------------------
// Domain identifiers
// ---------------------------------------------------------------------------

/// Contract / manifold tag for roots produced by this module.
pub const DISTRO_HASH_DOMAIN_POSEIDON_V1: &str = "poseidon-v1";

/// Leaf personalization string (UTF-8), packed via [`personalization_to_fp`].
pub const POSEIDON_DISTRO_LEAF_PERSONALIZATION: &str = "terp-hs-distro-leaf-v1";

/// Merkle CRH personalization string (UTF-8), packed via [`personalization_to_fp`].
pub const POSEIDON_DISTRO_CRH_PERSONALIZATION: &str = "terp-hs-distro-crh-v1";

/// Fixed Poseidon width used by Orchard / this crate (`t = 3`).
pub const POSEIDON_T: usize = 3;

/// Fixed Poseidon rate (`rate = 2` ⇒ capacity 1).
pub const POSEIDON_RATE: usize = 2;

/// Message arity of [`poseidon_distro_leaf`] (tag + 6 fields, including `rseed_com`).
pub const POSEIDON_DISTRO_LEAF_ARITY: usize = 7;

/// Message arity of [`poseidon_distro_crh`] (tag + layer + left + right).
pub const POSEIDON_DISTRO_CRH_ARITY: usize = 4;

// ---------------------------------------------------------------------------
// Hash domain selector (new helpers only; does not rewire legacy suite paths)
// ---------------------------------------------------------------------------

/// Public distribution tree hash domain.
///
/// New pure helpers default to [`DistroHashDomain::PoseidonV1`]. Legacy Sinsemilla
/// genesis trees remain available as [`DistroHashDomain::SinsemillaLegacy`] for
/// reading historical roots only — do not register new manifold roots under legacy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DistroHashDomain {
    /// Historical Sinsemilla leaf / MerkleCRH personalizations (Orchard-family).
    SinsemillaLegacy,
    /// Poseidon-v1 public inclusion set (this module).
    PoseidonV1,
}

impl Default for DistroHashDomain {
    fn default() -> Self {
        DistroHashDomain::PoseidonV1
    }
}

impl DistroHashDomain {
    /// Stable string for contract / manifold `distro_hash_domain` fields.
    pub const fn as_str(self) -> &'static str {
        match self {
            DistroHashDomain::SinsemillaLegacy => "sinsemilla-legacy",
            DistroHashDomain::PoseidonV1 => DISTRO_HASH_DOMAIN_POSEIDON_V1,
        }
    }
}

// ---------------------------------------------------------------------------
// Personalization packing
// ---------------------------------------------------------------------------

/// Pack a personalization string into `pallas::Base`.
///
/// Encoding: UTF-8 bytes in **little-endian** limb order (byte 0 of the string is the
/// least-significant byte of the field element), zero-padded to 32 bytes. The resulting
/// integer must be strictly less than the Pallas base modulus (always true for ASCII
/// tags of length ≤ 31 with high bytes zero).
///
/// # Panics
///
/// Panics if `tag` is longer than 32 bytes or is a non-canonical field encoding
/// (should not occur for the fixed ADR strings).
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

/// Domain tag field for the leaf hash.
#[inline]
pub fn distro_leaf_tag() -> pallas::Base {
    personalization_to_fp(POSEIDON_DISTRO_LEAF_PERSONALIZATION)
}

/// Domain tag field for the Merkle CRH.
#[inline]
pub fn distro_crh_tag() -> pallas::Base {
    personalization_to_fp(POSEIDON_DISTRO_CRH_PERSONALIZATION)
}

// ---------------------------------------------------------------------------
// Pure Poseidon hashes
// ---------------------------------------------------------------------------

/// Public distribution **leaf** hash (Poseidon-v1).
///
/// ```text
/// Poseidon^P128Pow5T3 / ConstantLength<7>(
///   personalization_to_fp("terp-hs-distro-leaf-v1"),
///   epk_x, epk_y, nd, v, fdi, rseed_com
/// )
/// ```
///
/// `rseed_com` is [`crate::claim_auth::claim_rseed_com`], not the raw 32-byte string.
pub fn poseidon_distro_leaf(
    epk_x: pallas::Base,
    epk_y: pallas::Base,
    nd: pallas::Base,
    v: pallas::Base,
    fdi: pallas::Base,
    rseed_com: pallas::Base,
) -> pallas::Base {
    poseidon::Hash::<_, P128Pow5T3, ConstantLength<7>, 3, 2>::init().hash([
        distro_leaf_tag(),
        epk_x,
        epk_y,
        nd,
        v,
        fdi,
        rseed_com,
    ])
}

/// Public distribution **Merkle CRH** (Poseidon-v1).
///
/// ```text
/// Poseidon^P128Pow5T3 / ConstantLength<4>(
///   personalization_to_fp("terp-hs-distro-crh-v1"),
///   Base::from(layer), left, right
/// )
/// ```
///
/// `layer`: `0` when combining two leaves; increments toward the root (suite convention).
pub fn poseidon_distro_crh(
    layer: u32,
    left: pallas::Base,
    right: pallas::Base,
) -> pallas::Base {
    poseidon::Hash::<_, P128Pow5T3, ConstantLength<4>, 3, 2>::init().hash([
        distro_crh_tag(),
        pallas::Base::from(u64::from(layer)),
        left,
        right,
    ])
}

/// Canonical 32-byte LE encoding of a distro tree root / node digest.
#[inline]
pub fn distro_digest_to_bytes(digest: pallas::Base) -> [u8; 32] {
    digest.to_repr()
}

/// Parse a 32-byte LE root / node digest.
#[inline]
pub fn distro_digest_from_bytes(bytes: [u8; 32]) -> Option<pallas::Base> {
    Option::from(pallas::Base::from_repr(bytes))
}

/// Depth-1 root over exactly two leaves (layer `0`).
#[inline]
pub fn poseidon_distro_root_two_leaves(
    left_leaf: pallas::Base,
    right_leaf: pallas::Base,
) -> pallas::Base {
    poseidon_distro_crh(0, left_leaf, right_leaf)
}

/// Recompute a distro-tree root from a leaf and authentication path (leaf → root).
///
/// At path index `l` (starting at `0` at the leaves), if bit `l` of `position` is clear
/// the current node is the **left** child; if set, it is the **right** child. This matches
/// Orchard path bit order and the suite's [`verify_merkle_path_poseidon_v1`].
///
/// `auth_path.len()` is the tree depth (often [`crate::constants::MERKLE_DEPTH_ORCHARD`] = 32).
pub fn poseidon_distro_path_root(
    leaf: pallas::Base,
    position: u32,
    auth_path: &[pallas::Base],
) -> pallas::Base {
    let mut node = leaf;
    for (l, sibling) in auth_path.iter().enumerate() {
        let layer = l as u32;
        if position & (1 << l) == 0 {
            node = poseidon_distro_crh(layer, node, *sibling);
        } else {
            node = poseidon_distro_crh(layer, *sibling, node);
        }
    }
    node
}

/// Empty-sibling padding used for incomplete levels and padded paths.
#[inline]
pub fn poseidon_distro_empty_sibling() -> pallas::Base {
    pallas::Base::zero()
}

// ---------------------------------------------------------------------------
// Tests (pure field arithmetic — no MockProver)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ff::Field;

    fn sample_leaf_inputs(seed: u64) -> (pallas::Base, pallas::Base, pallas::Base, pallas::Base, pallas::Base) {
        (
            pallas::Base::from(seed),
            pallas::Base::from(seed + 1),
            pallas::Base::from(seed + 2),
            pallas::Base::from(seed + 3),
            pallas::Base::from(seed + 4),
        )
    }

    #[test]
    fn personalizations_are_distinct_field_elements() {
        let leaf = distro_leaf_tag();
        let crh = distro_crh_tag();
        assert_ne!(leaf, crh);
        assert_ne!(leaf, pallas::Base::ZERO);
        assert_ne!(crh, pallas::Base::ZERO);
    }

    #[test]
    fn personalization_roundtrip_bytes() {
        let tag = POSEIDON_DISTRO_LEAF_PERSONALIZATION;
        let fe = personalization_to_fp(tag);
        let repr = fe.to_repr();
        assert_eq!(&repr[..tag.len()], tag.as_bytes());
        assert!(repr[tag.len()..].iter().all(|&b| b == 0));
    }

    #[test]
    fn leaf_hash_is_deterministic() {
        let (x, y, nd, v, fdi) = sample_leaf_inputs(7);
        let a = poseidon_distro_leaf(x, y, nd, v, fdi, pallas::Base::from(1u64));
        let b = poseidon_distro_leaf(x, y, nd, v, fdi, pallas::Base::from(1u64));
        assert_eq!(a, b);
        assert_ne!(a, pallas::Base::ZERO);
    }

    #[test]
    fn leaf_hash_changes_with_any_field() {
        let (x, y, nd, v, fdi) = sample_leaf_inputs(11);
        let base = poseidon_distro_leaf(x, y, nd, v, fdi, pallas::Base::from(1u64));
        assert_ne!(base, poseidon_distro_leaf(x + pallas::Base::ONE, y, nd, v, fdi, pallas::Base::from(1u64)));
        assert_ne!(base, poseidon_distro_leaf(x, y + pallas::Base::ONE, nd, v, fdi, pallas::Base::from(1u64)));
        assert_ne!(base, poseidon_distro_leaf(x, y, nd + pallas::Base::ONE, v, fdi, pallas::Base::from(1u64)));
        assert_ne!(base, poseidon_distro_leaf(x, y, nd, v + pallas::Base::ONE, fdi, pallas::Base::from(1u64)));
        assert_ne!(base, poseidon_distro_leaf(
            x,
            y,
            nd,
            v,
            fdi + pallas::Base::ONE,
            pallas::Base::from(1u64),
        ));
    }

    #[test]
    fn domain_separation_leaf_neq_crh() {
        // Same five payload fields hashed under leaf vs under a forged CRH-shaped message
        // must not collide with either domain's honest output.
        let (x, y, nd, v, fdi) = sample_leaf_inputs(42);
        let leaf = poseidon_distro_leaf(x, y, nd, v, fdi, pallas::Base::from(1u64));

        // CRH with layer/left/right drawn from subset of leaf fields still uses CRH tag.
        let crh = poseidon_distro_crh(0, x, y);
        assert_ne!(leaf, crh);

        // Swapping personalization domain (leaf payload hashed with CRH tag length) —
        // ConstantLength<4> on first four leaf message limbs ≠ ConstantLength<6> leaf.
        let forged_as_crh_arity = poseidon::Hash::<_, P128Pow5T3, ConstantLength<4>, 3, 2>::init()
            .hash([distro_leaf_tag(), x, y, nd]);
        assert_ne!(leaf, forged_as_crh_arity);
        assert_ne!(crh, forged_as_crh_arity);
    }

    #[test]
    fn crh_layer_domain_separation() {
        let left = pallas::Base::from(100u64);
        let right = pallas::Base::from(200u64);
        let h0 = poseidon_distro_crh(0, left, right);
        let h1 = poseidon_distro_crh(1, left, right);
        assert_ne!(h0, h1);
    }

    #[test]
    fn crh_is_order_sensitive() {
        let a = pallas::Base::from(1u64);
        let b = pallas::Base::from(2u64);
        assert_ne!(
            poseidon_distro_crh(0, a, b),
            poseidon_distro_crh(0, b, a)
        );
    }

    #[test]
    fn two_leaf_root_determinism_and_structure() {
        let (x0, y0, nd0, v0, fdi0) = sample_leaf_inputs(1);
        let (x1, y1, nd1, v1, fdi1) = sample_leaf_inputs(1000);

        let leaf0 = poseidon_distro_leaf(x0, y0, nd0, v0, fdi0, pallas::Base::from(1u64));
        let leaf1 = poseidon_distro_leaf(x1, y1, nd1, v1, fdi1, pallas::Base::from(1u64));
        assert_ne!(leaf0, leaf1);

        let root = poseidon_distro_root_two_leaves(leaf0, leaf1);
        let root_again = poseidon_distro_crh(0, leaf0, leaf1);
        assert_eq!(root, root_again);

        // Swapping leaves changes root.
        assert_ne!(root, poseidon_distro_root_two_leaves(leaf1, leaf0));

        // Root is not equal to either leaf.
        assert_ne!(root, leaf0);
        assert_ne!(root, leaf1);

        // Bytes round-trip.
        let bytes = distro_digest_to_bytes(root);
        assert_eq!(distro_digest_from_bytes(bytes), Some(root));
        assert_eq!(bytes.len(), 32);
    }

    #[test]
    fn default_domain_is_poseidon_v1() {
        assert_eq!(DistroHashDomain::default(), DistroHashDomain::PoseidonV1);
        assert_eq!(
            DistroHashDomain::PoseidonV1.as_str(),
            DISTRO_HASH_DOMAIN_POSEIDON_V1
        );
        assert_eq!(
            DistroHashDomain::SinsemillaLegacy.as_str(),
            "sinsemilla-legacy"
        );
    }

    #[test]
    fn empty_padding_zero_is_stable_child() {
        // Documented padding: ZERO sibling at layer 0.
        let leaf = poseidon_distro_leaf(
            pallas::Base::from(9u64),
            pallas::Base::from(8u64),
            pallas::Base::from(7u64),
            pallas::Base::from(6u64),
            pallas::Base::from(5u64),
            pallas::Base::from(1u64),
        );
        let parent = poseidon_distro_crh(0, leaf, pallas::Base::ZERO);
        assert_ne!(parent, leaf);
        assert_eq!(parent, poseidon_distro_crh(0, leaf, pallas::Base::ZERO));
    }

    #[test]
    fn path_root_matches_single_crh_steps() {
        let leaf = pallas::Base::from(1u64);
        let s0 = pallas::Base::from(2u64);
        let s1 = pallas::Base::from(3u64);
        // position 0b10 → right at layer 0, left at layer 1
        let pos = 0b01u32;
        let mid = poseidon_distro_crh(0, s0, leaf); // right child at layer 0
        let root = poseidon_distro_crh(1, mid, s1); // left child at layer 1
        assert_eq!(poseidon_distro_path_root(leaf, pos, &[s0, s1]), root);
    }
}
