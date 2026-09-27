//! BLAKE3 sorted-concat binary merkle tree.
//!
//! Implements exactly the same algorithm as `SortingBlake3Hasher` from
//! `rs_merkle` used in the cw-whitelist-merkletree contract:
//!
//!   leaf(addr, alloc)  = blake3((addr || alloc).as_bytes())
//!   parent(L, R)       = blake3(sorted([L, R]).concat())
//!
//! Each leaf is a composite of address + allocation, matching the contract's
//! `format!("{}{}", info.sender, allocation)` verification logic.
//!
//! Odd-length layers are padded by duplicating the last node.
//! Proofs are sibling hashes from leaf to root (not including root).

use anyhow::{bail, Result};
use std::collections::HashMap;

fn leaf(address: &str, allocation: u32) -> [u8; 32] {
    let input = format!("{}{}", address, allocation);
    *blake3::hash(input.as_bytes()).as_bytes()
}

fn parent(left: [u8; 32], right: [u8; 32]) -> [u8; 32] {
    let (a, b) = if left <= right {
        (left, right)
    } else {
        (right, left)
    };
    let mut combined = [0u8; 64];
    combined[..32].copy_from_slice(&a);
    combined[32..].copy_from_slice(&b);
    *blake3::hash(&combined).as_bytes()
}

/// Build a merkle tree from (address, allocation) pairs and return proofs for every entry.
///
/// Returns `(root_hex, address → (allocation, proof_hashes_hex))`.
pub fn build(entries: &[(String, u32)]) -> Result<(String, HashMap<String, (u32, Vec<String>)>)> {
    if entries.is_empty() {
        bail!("entry list is empty");
    }

    // Deduplicate by address, then sort so leaf order (and thus the root)
    // does not depend on caller HashMap / insertion order.
    let mut seen = std::collections::HashSet::new();
    let mut entries: Vec<&(String, u32)> = entries
        .iter()
        .filter(|(a, _)| seen.insert(a.as_str()))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let leaves: Vec<[u8; 32]> = entries.iter().map(|(a, alloc)| leaf(a, *alloc)).collect();
    let (root, layers) = build_layers(leaves);

    let root_hex = hex::encode(root);
    let members: HashMap<String, (u32, Vec<String>)> = entries
        .iter()
        .enumerate()
        .map(|(i, (addr, alloc))| {
            let proof = proof_for(&layers, i);
            let hashes = proof.into_iter().map(hex::encode).collect();
            (addr.to_string(), (*alloc, hashes))
        })
        .collect();

    Ok((root_hex, members))
}

/// Verify a proof — mirrors the contract's `verify_merkle_proof`.
///
/// The composite key `address || allocation` must exactly match what was used at build time.
pub fn verify(
    address: &str,
    allocation: u32,
    proof_hashes: &[String],
    root_hex: &str,
) -> Result<bool> {
    let mut current = leaf(address, allocation);
    for ph in proof_hashes {
        let sibling_bytes = hex::decode(ph)?;
        let sibling: [u8; 32] = sibling_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("proof hash must be 32 bytes"))?;
        current = parent(current, sibling);
    }
    Ok(hex::encode(current) == root_hex)
}

fn build_layers(leaves: Vec<[u8; 32]>) -> ([u8; 32], Vec<Vec<[u8; 32]>>) {
    let mut layers: Vec<Vec<[u8; 32]>> = vec![leaves];
    loop {
        let current = layers.last().unwrap();
        if current.len() == 1 {
            break;
        }
        let mut next = Vec::with_capacity((current.len() + 1) / 2);
        let mut padded = current.clone();
        if padded.len() % 2 == 1 {
            let last = *padded.last().unwrap();
            padded.push(last);
        }
        for chunk in padded.chunks(2) {
            next.push(parent(chunk[0], chunk[1]));
        }
        layers.push(next);
    }
    let root = *layers.last().unwrap().first().unwrap();
    (root, layers)
}

fn proof_for(layers: &[Vec<[u8; 32]>], leaf_index: usize) -> Vec<[u8; 32]> {
    let mut proof = Vec::new();
    let mut idx = leaf_index;
    // Iterate all layers except the root layer
    for layer in &layers[..layers.len().saturating_sub(1)] {
        let mut padded = layer.clone();
        if padded.len() % 2 == 1 {
            let last = *padded.last().unwrap();
            padded.push(last);
        }
        let sibling = idx ^ 1; // flip last bit to get sibling
        if sibling < padded.len() {
            proof.push(padded[sibling]);
        }
        idx /= 2;
    }
    proof
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_single() {
        let entries = vec![("terp1abc".to_string(), 1u32)];
        let (root, members) = build(&entries).unwrap();
        let (alloc, proof) = &members["terp1abc"];
        assert!(verify("terp1abc", *alloc, proof, &root).unwrap());
        assert!(proof.is_empty());
        assert_eq!(root, hex::encode(leaf("terp1abc", 1)));
    }

    #[test]
    fn round_trip_four() {
        let entries: Vec<(String, u32)> = [("a1", 1), ("a2", 3), ("a3", 6), ("a4", 1)]
            .iter()
            .map(|(a, n)| (a.to_string(), *n))
            .collect();
        let (root, members) = build(&entries).unwrap();
        for (addr, (alloc, proof)) in &members {
            assert!(
                verify(addr, *alloc, proof, &root).unwrap(),
                "proof failed for {addr}"
            );
        }
    }

    #[test]
    fn round_trip_odd() {
        let entries: Vec<(String, u32)> = [("x1", 3), ("x2", 3), ("x3", 6)]
            .iter()
            .map(|(a, n)| (a.to_string(), *n))
            .collect();
        let (root, members) = build(&entries).unwrap();
        for (addr, (alloc, proof)) in &members {
            assert!(
                verify(addr, *alloc, proof, &root).unwrap(),
                "proof failed for {addr}"
            );
        }
    }

    #[test]
    fn bad_proof_fails() {
        let entries = vec![("a".to_string(), 1u32), ("b".to_string(), 1u32)];
        let (root, members) = build(&entries).unwrap();
        let (alloc, proof) = &members["a"];
        // Tamper with the proof
        let mut bad_proof = proof.clone();
        bad_proof[0] = "0".repeat(64);
        assert!(!verify("a", *alloc, &bad_proof, &root).unwrap());
    }

    #[test]
    fn wrong_allocation_fails() {
        let entries = vec![("a".to_string(), 3u32), ("b".to_string(), 1u32)];
        let (root, members) = build(&entries).unwrap();
        let (_, proof) = &members["a"];
        // Correct proof but wrong allocation — should not verify
        assert!(!verify("a", 1, proof, &root).unwrap());
    }
}
