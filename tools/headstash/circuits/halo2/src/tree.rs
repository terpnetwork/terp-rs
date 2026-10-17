//! Types related to Orchard note commitment trees and anchors.

use alloc::vec::Vec;
use core::iter;

use crate::{
    constants::{
        sinsemilla::{i2lebsp_k, L_ORCHARD_MERKLE, MERKLE_CRH_PERSONALIZATION},
        MERKLE_DEPTH_ORCHARD,
    },
    note::commitment::ExtractedNoteCommitment,
};

use incrementalmerkletree::{Hashable, Level};
use pasta_curves::pallas;
use sinsemilla::HashDomain;

use ff::{Field, PrimeField, PrimeFieldBits};
use lazy_static::lazy_static;
use rand::Rng;
use rand_core::RngCore;
use serde::de::{Deserializer, Error};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use subtle::{Choice, ConditionallySelectable, CtOption};

// Empty sibling / uncommitted leaf for the **public distro** tree.
//
// Poseidon-v1 (ADR-POSEIDON-DISTRO-TREE) uses `pallas::Base::ZERO` padding, not
// Orchard's historical uncommitted leaf `2` (Sinsemilla note-commitment tree).
lazy_static! {
    /// Product A distro empty / pad leaf (`ZERO`).
    pub(crate) static ref UNCOMMITTED_DISTRO: pallas::Base = pallas::Base::ZERO;
    /// Historical Orchard uncommitted leaf (Sinsemilla note tree / recovery only).
    pub(crate) static ref UNCOMMITTED_ORCHARD_LEGACY: pallas::Base = pallas::Base::from(2);
    /// Empty-subtree roots for **Poseidon-v1** public distro (Product A).
    pub(crate) static ref EMPTY_ROOTS: Vec<MerkleHashOrchard> = {
        iter::empty()
            .chain(Some(MerkleHashOrchard::empty_leaf()))
            .chain(
                (0..MERKLE_DEPTH_ORCHARD).scan(MerkleHashOrchard::empty_leaf(), |state, l| {
                    let l = l as u8;
                    *state = MerkleHashOrchard::combine(l.into(), state, state);
                    Some(*state)
                }),
            )
            .collect()
    };
}

/// The root of an Orchard commitment tree. This must be a value
/// in the range {0..=q_ℙ-1}
#[derive(Eq, PartialEq, Clone, Copy, Debug)]
pub struct Anchor(pallas::Base);

impl From<pallas::Base> for Anchor {
    fn from(anchor_field: pallas::Base) -> Anchor {
        Anchor(anchor_field)
    }
}

impl From<MerkleHashOrchard> for Anchor {
    fn from(anchor: MerkleHashOrchard) -> Anchor {
        Anchor(anchor.0)
    }
}

impl Anchor {
    /// The anchor of the empty Orchard note commitment tree.
    ///
    /// This anchor does not correspond to any valid anchor for a spend, so it
    /// may only be used for coinbase bundles or in circumstances where Orchard
    /// functionality is not active.
    pub fn empty_tree() -> Anchor {
        Anchor(MerkleHashOrchard::empty_root(Level::from(MERKLE_DEPTH_ORCHARD as u8)).0)
    }

    pub(crate) fn inner(&self) -> pallas::Base {
        self.0
    }

    /// Parses an Orchard anchor from a byte encoding.
    pub fn from_bytes(bytes: [u8; 32]) -> CtOption<Anchor> {
        pallas::Base::from_repr(bytes).map(Anchor)
    }

    /// Returns the byte encoding of this anchor.
    pub fn to_bytes(self) -> [u8; 32] {
        self.0.to_repr()
    }
}

/// The Merkle path from a leaf of the note commitment tree
/// to its anchor.
#[derive(Clone, Debug)]
pub struct MerklePath {
    position: u32,
    auth_path: [MerkleHashOrchard; MERKLE_DEPTH_ORCHARD],
}

#[cfg(any(test, feature = "test-dependencies"))]
#[cfg_attr(docsrs, doc(cfg(feature = "test-dependencies")))]
impl From<(incrementalmerkletree::Position, Vec<MerkleHashOrchard>)> for MerklePath {
    fn from(path: (incrementalmerkletree::Position, Vec<MerkleHashOrchard>)) -> Self {
        let position: u64 = path.0.into();
        Self {
            position: position as u32,
            auth_path: path.1.try_into().unwrap(),
        }
    }
}

impl From<incrementalmerkletree::MerklePath<MerkleHashOrchard, 32>> for MerklePath {
    fn from(path: incrementalmerkletree::MerklePath<MerkleHashOrchard, 32>) -> Self {
        let position: u64 = path.position().into();
        Self {
            position: position as u32,
            auth_path: path.path_elems().try_into().unwrap(),
        }
    }
}

impl MerklePath {
    /// Generates a dummy Merkle path for use in dummy spent notes.
    pub(crate) fn dummy(mut rng: &mut impl Rng) -> Self {
        MerklePath {
            position: rng.next_u32(),
            auth_path: [(); MERKLE_DEPTH_ORCHARD]
                .map(|_| MerkleHashOrchard(pallas::Base::random(&mut rng))),
        }
    }

    /// Instantiates a new Merkle path given a leaf position and authentication path.
    pub(crate) fn new(position: u32, auth_path: [pallas::Base; MERKLE_DEPTH_ORCHARD]) -> Self {
        Self::from_parts(position, auth_path.map(MerkleHashOrchard))
    }

    /// Instantiates a new Merkle path given a leaf position and authentication path.
    pub fn from_parts(position: u32, auth_path: [MerkleHashOrchard; MERKLE_DEPTH_ORCHARD]) -> Self {
        Self {
            position,
            auth_path,
        }
    }

    /// Poseidon-v1 public inclusion root from a leaf field element.
    ///
    /// Path index `l = 0` is at the leaves (ADR / suite convention). Uses
    /// [`crate::distro_poseidon::poseidon_distro_path_root`] when the `circuit`
    /// feature is enabled.
    #[cfg(feature = "circuit")]
    pub fn root_from_leaf(&self, leaf: pallas::Base) -> Anchor {
        use crate::distro_poseidon::poseidon_distro_path_root;
        let path: [pallas::Base; MERKLE_DEPTH_ORCHARD] = self.auth_path.map(|h| h.0);
        Anchor(poseidon_distro_path_root(leaf, self.position, &path))
    }

    /// Public inclusion root from a leaf commitment extract (x-coordinate).
    ///
    /// **Poseidon-v1** for the Headstash distro tree (replaces Sinsemilla MerkleCRH).
    pub fn root(&self, cmx: ExtractedNoteCommitment) -> Anchor {
        #[cfg(feature = "circuit")]
        {
            self.root_from_leaf(cmx.inner())
        }
        #[cfg(not(feature = "circuit"))]
        {
            // Without halo2_gadgets, fall back to Sinsemilla-legacy combine.
            self.auth_path
                .iter()
                .enumerate()
                .fold(MerkleHashOrchard::from_cmx(&cmx), |node, (l, sibling)| {
                    let l = l as u8;
                    if self.position & (1 << l) == 0 {
                        MerkleHashOrchard::combine(l.into(), &node, sibling)
                    } else {
                        MerkleHashOrchard::combine(l.into(), sibling, &node)
                    }
                })
                .into()
        }
    }

    /// Sinsemilla-legacy root (recovery only; not for new Headstashes).
    #[cfg(feature = "circuit")]
    pub fn root_sinsemilla_legacy(&self, cmx: ExtractedNoteCommitment) -> Anchor {
        self.auth_path
            .iter()
            .enumerate()
            .fold(MerkleHashOrchard::from_cmx(&cmx), |node, (l, sibling)| {
                let l = l as u8;
                if self.position & (1 << l) == 0 {
                    MerkleHashOrchard::combine_sinsemilla_legacy(l.into(), &node, sibling)
                } else {
                    MerkleHashOrchard::combine_sinsemilla_legacy(l.into(), sibling, &node)
                }
            })
            .into()
    }

    /// Returns the position of the leaf using this Merkle path.
    pub fn position(&self) -> u32 {
        self.position
    }

    /// Returns the authentication path.
    pub fn auth_path(&self) -> [MerkleHashOrchard; MERKLE_DEPTH_ORCHARD] {
        self.auth_path
    }
}

/// A newtype wrapper for leaves and internal nodes in the Orchard
/// incremental note commitment tree.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MerkleHashOrchard(pallas::Base);

impl MerkleHashOrchard {
    /// Creates an incremental tree leaf digest from the specified
    /// Orchard extracted note commitment.
    pub fn from_cmx(value: &ExtractedNoteCommitment) -> Self {
        MerkleHashOrchard(value.inner())
    }

    /// Only used in the circuit.
    pub(crate) fn inner(&self) -> pallas::Base {
        self.0
    }

    /// Convert this digest to its canonical byte representation.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_repr()
    }

    /// Parses a incremental tree leaf digest from the bytes of
    /// a note commitment.
    ///
    /// Returns the empty `CtOption` if the provided bytes represent
    /// a non-canonical encoding.
    pub fn from_bytes(bytes: &[u8; 32]) -> CtOption<Self> {
        pallas::Base::from_repr(*bytes).map(MerkleHashOrchard)
    }
}

impl ConditionallySelectable for MerkleHashOrchard {
    fn conditional_select(a: &Self, b: &Self, choice: Choice) -> Self {
        MerkleHashOrchard(pallas::Base::conditional_select(&a.0, &b.0, choice))
    }
}

impl MerkleHashOrchard {
    /// Sinsemilla-legacy MerkleCRH (Orchard note tree / pre-Poseidon distro recovery).
    pub fn combine_sinsemilla_legacy(level: Level, left: &Self, right: &Self) -> Self {
        let domain = HashDomain::new(MERKLE_CRH_PERSONALIZATION);
        MerkleHashOrchard(
            domain
                .hash(
                    iter::empty()
                        .chain(i2lebsp_k(level.into()).iter().copied())
                        .chain(left.0.to_le_bits().iter().by_vals().take(L_ORCHARD_MERKLE))
                        .chain(right.0.to_le_bits().iter().by_vals().take(L_ORCHARD_MERKLE)),
                )
                .unwrap_or(pallas::Base::zero()),
        )
    }
}

impl Hashable for MerkleHashOrchard {
    /// Empty / padding leaf for **Poseidon-v1** public distro trees (`ZERO`).
    fn empty_leaf() -> Self {
        MerkleHashOrchard(*UNCOMMITTED_DISTRO)
    }

    /// Poseidon-v1 Merkle CRH for the Headstash public inclusion tree.
    ///
    /// `level` is treated as the suite/ADR layer counter: `0` when hashing leaves,
    /// increasing toward the root (same as path index `l` in [`MerklePath::root`]).
    fn combine(level: Level, left: &Self, right: &Self) -> Self {
        #[cfg(feature = "circuit")]
        {
            use crate::distro_poseidon::poseidon_distro_crh;
            let layer = u32::from(level);
            MerkleHashOrchard(poseidon_distro_crh(layer, left.0, right.0))
        }
        #[cfg(not(feature = "circuit"))]
        {
            Self::combine_sinsemilla_legacy(level, left, right)
        }
    }

    fn empty_root(level: Level) -> Self {
        EMPTY_ROOTS[<usize>::from(level)]
    }
}

impl Serialize for MerkleHashOrchard {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_bytes().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for MerkleHashOrchard {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let parsed = <[u8; 32]>::deserialize(deserializer)?;
        <Option<_>>::from(Self::from_bytes(&parsed)).ok_or_else(|| {
            Error::custom(
            "Attempted to deserialize a non-canonical representation of a Pallas base field element.",
        )
        })
    }
}

/// Test utilities available under the `test-dependencies` feature flag.
#[cfg(feature = "test-dependencies")]
pub mod testing {
    use ff::Field;
    use rand::{
        distributions::{Distribution, Standard},
        RngCore,
    };

    use super::MerkleHashOrchard;

    impl MerkleHashOrchard {
        /// Return a random fake `MerkleHashOrchard`.
        pub fn random(rng: &mut impl RngCore) -> Self {
            Standard.sample(rng)
        }
    }

    impl Distribution<MerkleHashOrchard> for Standard {
        fn sample<R: rand::Rng + ?Sized>(&self, rng: &mut R) -> MerkleHashOrchard {
            MerkleHashOrchard(pasta_curves::Fp::random(rng))
        }
    }
}

#[cfg(test)]
mod tests {
    //! Product A (`poseidon-v1` distro): empty leaf = `ZERO`, CRH = Poseidon.
    //! Orchard ZIP / Sinsemilla vectors live under [`sinsemilla_legacy`].

    use {
        crate::{
            constants::MERKLE_DEPTH_ORCHARD,
            tree::{MerkleHashOrchard, MerklePath, EMPTY_ROOTS, UNCOMMITTED_DISTRO},
        },
        group::ff::{Field, PrimeField},
        incrementalmerkletree::{Hashable, Level},
        pasta_curves::pallas,
    };

    /// Product A: empty / padding leaf is `pallas::Base::zero()` (not Orchard `2`).
    #[test]
    fn poseidon_v1_empty_leaf_is_zero() {
        assert_eq!(MerkleHashOrchard::empty_leaf().0, pallas::Base::zero());
        assert_eq!(*UNCOMMITTED_DISTRO, pallas::Base::zero());
        // Orchard ZIP empty leaf (`2`) must not equal Product A empty leaf.
        assert_ne!(MerkleHashOrchard::empty_leaf().0, pallas::Base::from(2));
    }

    /// Product A: EMPTY_ROOTS is the iterated Poseidon empty-subtree chain.
    #[test]
    fn poseidon_v1_empty_roots_match_iterated_crh() {
        assert_eq!(EMPTY_ROOTS.len(), MERKLE_DEPTH_ORCHARD + 1);
        assert_eq!(EMPTY_ROOTS[0], MerkleHashOrchard::empty_leaf());

        let mut state = MerkleHashOrchard::empty_leaf();
        for l in 0..MERKLE_DEPTH_ORCHARD {
            state = MerkleHashOrchard::combine((l as u8).into(), &state, &state);
            assert_eq!(
                EMPTY_ROOTS[l + 1],
                state,
                "EMPTY_ROOTS mismatch at height {}",
                l + 1
            );
            assert_eq!(
                MerkleHashOrchard::empty_root(Level::from((l + 1) as u8)),
                state
            );
        }
    }

    /// Product A: dummy path with ZERO siblings recomputes via Poseidon path root.
    #[test]
    fn poseidon_v1_path_root_with_empty_siblings() {
        #[cfg(feature = "circuit")]
        {
            use crate::distro_poseidon::{poseidon_distro_crh, poseidon_distro_path_root};

            let leaf = pallas::Base::from(0xA11CEu64);
            let position = 0u32;
            let auth_path = [MerkleHashOrchard(*UNCOMMITTED_DISTRO); MERKLE_DEPTH_ORCHARD];
            let path = MerklePath::from_parts(position, auth_path);
            let anchor = path.root_from_leaf(leaf);

            let path_fp = auth_path.map(|h| h.0);
            let expected = poseidon_distro_path_root(leaf, position, &path_fp);
            assert_eq!(anchor.inner(), expected);

            // First step: left=leaf, right=ZERO at layer 0.
            let parent0 = poseidon_distro_crh(0, leaf, pallas::Base::zero());
            let mut node = parent0;
            for l in 1..MERKLE_DEPTH_ORCHARD {
                node = poseidon_distro_crh(l as u32, node, pallas::Base::zero());
            }
            assert_eq!(anchor.inner(), node);
        }
    }

    /// Orchard ZIP / historical Sinsemilla note-tree vectors — **recovery only**.
    ///
    /// Product A public distro trees use Poseidon-v1 + ZERO padding. These tests
    /// do **not** assert against [`EMPTY_ROOTS`] (Poseidon). They use a local
    /// [`Hashable`] type with Orchard uncommitted leaf `2` and
    /// [`MerkleHashOrchard::combine_sinsemilla_legacy`].
    mod sinsemilla_legacy {
        use super::*;
        use crate::tree::UNCOMMITTED_ORCHARD_LEGACY;
        use incrementalmerkletree::{frontier::Frontier, Level, Marking, MerklePath, Retention};
        use shardtree::{store::memory::MemoryShardStore, ShardTree};
        use subtle::{Choice, ConditionallySelectable};

        /// Sinsemilla-legacy digest for Orchard ZIP recovery tests only.
        /// Default [`MerkleHashOrchard`] `Hashable` is Poseidon-v1 (Product A).
        #[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
        struct MerkleHashSinsemillaLegacy(pallas::Base);

        lazy_static::lazy_static! {
            static ref EMPTY_ROOTS_SINSEMILLA_LEGACY: alloc::vec::Vec<MerkleHashSinsemillaLegacy> = {
                let mut roots = alloc::vec::Vec::with_capacity(MERKLE_DEPTH_ORCHARD + 1);
                let mut state = MerkleHashSinsemillaLegacy(*UNCOMMITTED_ORCHARD_LEGACY);
                roots.push(state);
                for l in 0..MERKLE_DEPTH_ORCHARD {
                    state = Hashable::combine(Level::from(l as u8), &state, &state);
                    roots.push(state);
                }
                roots
            };
        }

        impl ConditionallySelectable for MerkleHashSinsemillaLegacy {
            fn conditional_select(a: &Self, b: &Self, choice: Choice) -> Self {
                MerkleHashSinsemillaLegacy(pallas::Base::conditional_select(&a.0, &b.0, choice))
            }
        }

        impl Hashable for MerkleHashSinsemillaLegacy {
            fn empty_leaf() -> Self {
                MerkleHashSinsemillaLegacy(*UNCOMMITTED_ORCHARD_LEGACY)
            }

            fn combine(level: Level, left: &Self, right: &Self) -> Self {
                let h = MerkleHashOrchard::combine_sinsemilla_legacy(
                    level,
                    &MerkleHashOrchard(left.0),
                    &MerkleHashOrchard(right.0),
                );
                MerkleHashSinsemillaLegacy(h.0)
            }

            fn empty_root(level: Level) -> Self {
                EMPTY_ROOTS_SINSEMILLA_LEGACY[<usize>::from(level)]
            }
        }

        #[test]
        fn orchard_zip_empty_roots_sinsemilla_legacy() {
            let tv_empty_roots =
                crate::test_vectors::commitment_tree::test_vectors().empty_roots;
            assert_eq!(
                MerkleHashSinsemillaLegacy::empty_leaf().0,
                pallas::Base::from(2)
            );
            for (height, root) in EMPTY_ROOTS_SINSEMILLA_LEGACY.iter().enumerate() {
                assert_eq!(
                    tv_empty_roots[height],
                    root.0.to_repr(),
                    "Sinsemilla-legacy empty root mismatch at height {}",
                    height
                );
            }
        }

        #[test]
        fn empty_roots_incremental_sinsemilla_legacy() {
            let tv_empty_roots =
                crate::test_vectors::commitment_tree::test_vectors().empty_roots;
            for (level, tv_root) in tv_empty_roots.iter().enumerate() {
                assert_eq!(
                    MerkleHashSinsemillaLegacy::empty_root(Level::from(level as u8))
                        .0
                        .to_repr(),
                    *tv_root,
                    "Sinsemilla-legacy empty root mismatch at level {}",
                    level
                );
            }
        }

        #[test]
        fn test_vectors_sinsemilla_legacy() {
            let tv_empty_roots =
                crate::test_vectors::commitment_tree::test_vectors().empty_roots;
            for (height, root) in EMPTY_ROOTS_SINSEMILLA_LEGACY.iter().enumerate() {
                assert_eq!(tv_empty_roots[height], root.0.to_repr());
            }

            let mut tree: ShardTree<MemoryShardStore<MerkleHashSinsemillaLegacy, u32>, 4, 3> =
                ShardTree::new(MemoryShardStore::empty(), 100);
            for (i, tv) in crate::test_vectors::merkle_path::test_vectors()
                .into_iter()
                .enumerate()
            {
                let checkpoint_id = u32::try_from(i).unwrap();
                let cmx = MerkleHashSinsemillaLegacy(
                    pallas::Base::from_repr(tv.leaves[i]).unwrap(),
                );
                tree.append(
                    cmx,
                    Retention::Checkpoint {
                        id: checkpoint_id,
                        marking: Marking::Marked,
                    },
                )
                .unwrap();

                let root = tree.root_at_checkpoint_id(&checkpoint_id).unwrap().unwrap();
                assert_eq!(root.0, pallas::Base::from_repr(tv.root).unwrap());

                for j in 0..=i {
                    let position = j.try_into().unwrap();
                    assert_eq!(
                        tree.witness_at_checkpoint_id(position, &checkpoint_id)
                            .unwrap(),
                        MerklePath::from_parts(
                            tv.paths[j]
                                .iter()
                                .map(|v| MerkleHashSinsemillaLegacy(
                                    pallas::Base::from_repr(*v).unwrap()
                                ))
                                .collect(),
                            position
                        )
                        .ok()
                    );
                }
            }
        }

        #[test]
        fn anchor_incremental_sinsemilla_legacy() {
            // zcashd merkle_roots_orchard.h / orchard_merkle_tree.py
            let commitments = [
                [
                    0x68, 0x13, 0x5c, 0xf4, 0x99, 0x33, 0x22, 0x90, 0x99, 0xa4, 0x4e, 0xc9, 0x9a,
                    0x75, 0xe1, 0xe1, 0xcb, 0x46, 0x40, 0xf9, 0xb5, 0xbd, 0xec, 0x6b, 0x32, 0x23,
                    0x85, 0x6f, 0xea, 0x16, 0x39, 0x0a,
                ],
                [
                    0x78, 0x31, 0x50, 0x08, 0xfb, 0x29, 0x98, 0xb4, 0x30, 0xa5, 0x73, 0x1d, 0x67,
                    0x26, 0x20, 0x7d, 0xc0, 0xf0, 0xec, 0x81, 0xea, 0x64, 0xaf, 0x5c, 0xf6, 0x12,
                    0x95, 0x69, 0x01, 0xe7, 0x2f, 0x0e,
                ],
                [
                    0xee, 0x94, 0x88, 0x05, 0x3a, 0x30, 0xc5, 0x96, 0xb4, 0x30, 0x14, 0x10, 0x5d,
                    0x34, 0x77, 0xe6, 0xf5, 0x78, 0xc8, 0x92, 0x40, 0xd1, 0xd1, 0xee, 0x17, 0x43,
                    0xb7, 0x7b, 0xb6, 0xad, 0xc4, 0x0a,
                ],
                [
                    0x9d, 0xdc, 0xe7, 0xf0, 0x65, 0x01, 0xf3, 0x63, 0x76, 0x8c, 0x5b, 0xca, 0x3f,
                    0x26, 0x46, 0x60, 0x83, 0x4d, 0x4d, 0xf4, 0x46, 0xd1, 0x3e, 0xfc, 0xd7, 0xc6,
                    0xf1, 0x7b, 0x16, 0x7a, 0xac, 0x1a,
                ],
                [
                    0xbd, 0x86, 0x16, 0x81, 0x1c, 0x6f, 0x5f, 0x76, 0x9e, 0xa4, 0x53, 0x9b, 0xba,
                    0xff, 0x0f, 0x19, 0x8a, 0x6c, 0xdf, 0x3b, 0x28, 0x0d, 0xd4, 0x99, 0x26, 0x16,
                    0x3b, 0xd5, 0x3f, 0x53, 0xa1, 0x21,
                ],
            ];
            let expected_anchor = [
                0xc8, 0x75, 0xbe, 0x2d, 0x60, 0x87, 0x3f, 0x8b, 0xcd, 0xeb, 0x91, 0x28, 0x2e, 0x64,
                0x2e, 0x0c, 0xc6, 0x5f, 0xf7, 0xd0, 0x64, 0x2d, 0x13, 0x7b, 0x28, 0xcf, 0x28, 0xcc,
                0x9c, 0x52, 0x7f, 0x0e,
            ];

            let mut frontier: Frontier<MerkleHashSinsemillaLegacy, 32> = Frontier::empty();
            for commitment in commitments.iter() {
                let cmx = MerkleHashSinsemillaLegacy(
                    pallas::Base::from_repr(*commitment).unwrap(),
                );
                frontier.append(cmx);
            }
            assert_eq!(
                frontier.root().0,
                pallas::Base::from_repr(expected_anchor).unwrap()
            );
        }
    }
}
