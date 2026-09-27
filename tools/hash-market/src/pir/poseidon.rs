//! Poseidon-v1 distro Merkle paths.
//!
//! Same CRH as `zk_headstash::distro_poseidon` and `gen_community_notes`:
//! layer 0 hashes two leaves, odd levels pad with `Base::ZERO`, sibling order
//! follows the leaf index bits (current node is left when the bit is clear).

use ff::{Field, PrimeField};
use halo2_gadgets::poseidon::primitives::{self as poseidon, ConstantLength, P128Pow5T3};
use pasta_curves::pallas;

const CRH_TAG: &str = "terp-hs-distro-crh-v1";

fn personalization_to_fp(tag: &str) -> pallas::Base {
    let b = tag.as_bytes();
    assert!(b.len() <= 32, "personalization longer than 32 bytes");
    let mut bytes = [0u8; 32];
    bytes[..b.len()].copy_from_slice(b);
    pallas::Base::from_repr(bytes).expect("personalization must be canonical")
}

pub(crate) fn poseidon_distro_crh(
    layer: u32,
    left: pallas::Base,
    right: pallas::Base,
) -> pallas::Base {
    poseidon::Hash::<_, P128Pow5T3, ConstantLength<4>, 3, 2>::init().hash([
        personalization_to_fp(CRH_TAG),
        pallas::Base::from(u64::from(layer)),
        left,
        right,
    ])
}

/// Recompute root from a leaf, its index, and the sibling path (leaf → root).
pub(crate) fn path_root(
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

pub(crate) struct AuthTree {
    pub root: pallas::Base,
    /// `paths[i]` is the sibling list for leaf `i` (not including the root).
    pub paths: Vec<Vec<pallas::Base>>,
}

/// Binary tree over `leaves` in order. Matches `gen_community_notes::poseidon_root`.
///
/// Odd levels are padded once. Paths index those layers — do not clone a layer per leaf
/// (community-notes is ~50k leaves).
pub(crate) fn auth_tree(leaves: &[pallas::Base]) -> AuthTree {
    assert!(!leaves.is_empty(), "auth_tree requires at least one leaf");
    let mut layers: Vec<Vec<pallas::Base>> = vec![leaves.to_vec()];
    let mut layer_i = 0u32;
    while layers.last().map(|l| l.len()).unwrap_or(0) > 1 {
        let mut padded = layers.last().unwrap().clone();
        if padded.len() % 2 == 1 {
            padded.push(pallas::Base::ZERO);
        }
        let mut next = Vec::with_capacity(padded.len() / 2);
        for chunk in padded.chunks(2) {
            next.push(poseidon_distro_crh(layer_i, chunk[0], chunk[1]));
        }
        *layers.last_mut().unwrap() = padded;
        layers.push(next);
        layer_i += 1;
    }
    let root = *layers.last().unwrap().first().unwrap();
    let depth = layers.len() - 1;
    let mut paths = Vec::with_capacity(leaves.len());
    for leaf_index in 0..leaves.len() {
        let mut idx = leaf_index;
        let mut proof = Vec::with_capacity(depth);
        for layer in &layers[..depth] {
            proof.push(layer[idx ^ 1]);
            idx /= 2;
        }
        paths.push(proof);
    }
    AuthTree { root, paths }
}

pub(crate) fn fp_from_hex(s: &str) -> anyhow::Result<pallas::Base> {
    let h = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    let raw = hex::decode(h)?;
    anyhow::ensure!(raw.len() == 32, "field element must be 32 bytes");
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&raw);
    Option::from(pallas::Base::from_repr(bytes)).ok_or_else(|| anyhow::anyhow!("non-canonical field element"))
}

pub(crate) fn fp_to_hex(fp: pallas::Base) -> String {
    format!("0x{}", hex::encode(fp.to_repr()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_leaf_path_is_empty() {
        let leaf = pallas::Base::from(7u64);
        let tree = auth_tree(&[leaf]);
        assert!(tree.paths[0].is_empty());
        assert_eq!(tree.root, leaf);
        assert_eq!(path_root(leaf, 0, &tree.paths[0]), tree.root);
    }

    #[test]
    fn odd_width_paths_verify() {
        let leaves: Vec<pallas::Base> = (1..4).map(pallas::Base::from).collect();
        let tree = auth_tree(&leaves);
        for (i, leaf) in leaves.iter().enumerate() {
            assert_eq!(
                path_root(*leaf, i as u32, &tree.paths[i]),
                tree.root,
                "path {i}"
            );
        }
    }
}
