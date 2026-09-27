//! Keccak256 → Pallas Fp reduction.
//!
//! Ethereum state roots are 32-byte Keccak256 hashes. To use them in a
//! Pallas-curve ZK circuit, we reduce them modulo the Pallas base field
//! prime `p = 2^254 + 45560315531506369815346746415080538113`.
//!
//! The reduction uses school-book 256-bit unsigned division since we
//! don't want to pull in a big-integer crate.

use crate::msg::PallasLeaf;
use tiny_keccak::{Hasher, Keccak};

/// Pallas base field prime p.
///
/// p = 0x40000000000000000000000000000000224698fc094cf91b992d30ed00000001
const PALLAS_P: [u8; 32] = [
    0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x22, 0x46, 0x98, 0xfc, 0x09, 0x4c, 0xf9, 0x1b, 0x99, 0x2d, 0x30, 0xed, 0x00, 0x00, 0x00, 0x01,
];

/// Keccak256 hash, then reduce mod Pallas p.
pub fn keccak_to_pallas(data: &[u8]) -> PallasLeaf {
    let mut hasher = Keccak::v256();
    hasher.update(data);
    let mut hash = [0u8; 32];
    hasher.finalize(&mut hash);
    PallasLeaf(reduce_mod_p(&hash))
}

/// Reduce a 256-bit big-endian integer mod Pallas p.
///
/// If `val >= p`, returns `val - p` (at most one subtraction needed since
/// `val < 2^256` and `p > 2^254`, so `val < 4*p`).
/// Actually val can be up to 2^256 - 1 ≈ 3.99*p, so we may need up to 3 subtractions.
fn reduce_mod_p(val: &[u8; 32]) -> [u8; 32] {
    let mut result = *val;
    // Subtract p while result >= p (at most 3 times)
    while ge_p(&result) {
        result = sub_256(&result, &PALLAS_P);
    }
    result
}

/// Check if `a >= PALLAS_P` (big-endian comparison).
fn ge_p(a: &[u8; 32]) -> bool {
    for i in 0..32 {
        if a[i] > PALLAS_P[i] {
            return true;
        }
        if a[i] < PALLAS_P[i] {
            return false;
        }
    }
    true // equal
}

/// 256-bit subtraction: a - b (big-endian). Assumes a >= b.
fn sub_256(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut result = [0u8; 32];
    let mut borrow: u16 = 0;
    for i in (0..32).rev() {
        let diff = (a[i] as u16).wrapping_sub(b[i] as u16).wrapping_sub(borrow);
        result[i] = diff as u8;
        borrow = if diff > 0xFF { 1 } else { 0 };
    }
    result
}

/// Transform a batch of Ethereum proof data into Pallas leaves.
///
/// Each proof's storage_hash is hashed through Keccak256 then reduced to Pallas Fp.
pub fn transform_proofs(proof_data: &[Vec<u8>]) -> Vec<PallasLeaf> {
    proof_data.iter().map(|d| keccak_to_pallas(d)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keccak_to_pallas_deterministic() {
        let a = keccak_to_pallas(b"hello");
        let b = keccak_to_pallas(b"hello");
        assert_eq!(a, b);
    }

    #[test]
    fn result_less_than_p() {
        let leaf = keccak_to_pallas(b"test input");
        // Verify result < p
        assert!(!ge_p(&leaf.0) || leaf.0 == reduce_mod_p(&leaf.0));
    }

    #[test]
    fn sub_256_basic() {
        let a = [0u8; 32];
        let b = [0u8; 32];
        assert_eq!(sub_256(&a, &b), [0u8; 32]);
    }

    #[test]
    fn transform_proofs_batch() {
        let proofs = vec![b"proof1".to_vec(), b"proof2".to_vec()];
        let leaves = transform_proofs(&proofs);
        assert_eq!(leaves.len(), 2);
        assert_ne!(leaves[0], leaves[1]);
    }
}
