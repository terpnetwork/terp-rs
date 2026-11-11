//! Terp port addition (not upstream): the host/wallet values a chain pins before
//! voters prove with [`super::build_vote_proof_from_delegation`].
//!
//! A proposal's anchor is the root of the vote commitment tree over the voters'
//! VAN leaves (`vote_authority_note_old`); each voter proves membership with its
//! auth path. Upstream receives VAN leaves from the delegation step (ZKP #1),
//! which this port does not carry, so the leaf is derived here exactly as the
//! builder derives it.

use ff::{Field, PrimeField};
use group::Curve;
use orchard::keys::{FullViewingKey, Scope};
use pasta_curves::pallas;

/// Re-exported so hosts need no direct `zakura-orchard` dependency.
pub use orchard::keys::SpendingKey;

use super::builder::{address_points, extract_vsk, pallas_coordinates};
use super::circuit::van_integrity_hash;
use crate::params::{BALLOT_DIVISOR, VOTE_COMM_TREE_DEPTH};
use crate::protocol_hash::poseidon_hash_2;

/// Sibling hashes from leaf to root.
pub type AuthPath = [pallas::Base; VOTE_COMM_TREE_DEPTH];

/// The VAN leaf the builder recomputes from the same arguments.
pub fn van_commitment(
    sk: &SpendingKey,
    address_index: u32,
    total_note_value: u64,
    voting_round_id: pallas::Base,
    proposal_authority: u64,
    van_comm_rand: pallas::Base,
) -> pallas::Base {
    let fvk: FullViewingKey = sk.into();
    let (g_d, pk_d) = address_points(&fvk.address_at(address_index, Scope::External));
    let x = |p: pallas::Point| *pallas_coordinates(p.to_affine()).expect("non-identity").x();
    van_integrity_hash(
        x(g_d),
        x(pk_d),
        pallas::Base::from(total_note_value / BALLOT_DIVISOR),
        voting_round_id,
        pallas::Base::from(proposal_authority),
        van_comm_rand,
    )
}

/// Root and per-leaf auth paths of the depth-24 Poseidon tree holding `leaves` at
/// positions `0..n` (empty leaves are zero, matching the builder's hashing order).
pub fn vote_comm_tree(leaves: &[pallas::Base]) -> (pallas::Base, Vec<AuthPath>) {
    let mut level = leaves.to_vec();
    let mut empty = pallas::Base::ZERO;
    let mut paths = vec![[pallas::Base::ZERO; VOTE_COMM_TREE_DEPTH]; leaves.len()];
    for depth in 0..VOTE_COMM_TREE_DEPTH {
        for (i, path) in paths.iter_mut().enumerate() {
            path[depth] = level.get((i >> depth) ^ 1).copied().unwrap_or(empty);
        }
        level = level
            .chunks(2)
            .map(|c| poseidon_hash_2(c[0], c.get(1).copied().unwrap_or(empty)))
            .collect();
        empty = poseidon_hash_2(empty, empty);
    }
    (level.first().copied().unwrap_or(empty), paths)
}

/// Canonical encoding of `rsk_v = vsk + alpha_v`, the RedPallas SpendAuth signing
/// key for the proof's `r_vpk`. Secret: the caller owns its lifetime.
pub fn randomized_signing_key(sk: &SpendingKey, alpha_v: pallas::Scalar) -> [u8; 32] {
    (extract_vsk(sk) + alpha_v).to_repr()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_paths_reproduce_root() {
        let leaves: Vec<_> = (1..=5u64).map(pallas::Base::from).collect();
        let (root, paths) = vote_comm_tree(&leaves);
        for (pos, (leaf, path)) in leaves.iter().zip(&paths).enumerate() {
            let mut cur = *leaf;
            for (d, sib) in path.iter().enumerate() {
                cur = if pos >> d & 1 == 0 {
                    poseidon_hash_2(cur, *sib)
                } else {
                    poseidon_hash_2(*sib, cur)
                };
            }
            assert_eq!(cur, root);
        }
    }
}
