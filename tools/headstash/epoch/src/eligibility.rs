//! Combined EVM + Solana eligibility tree for `RegisterHeadstash` /
//! `RegisterHeadstashSol`. Encodings mirror `cw-headstash::registration`:
//!
//! - EVM leaf:    `sha256("terp-hs-elig-leaf-v1" || address[20] || amount_le[8])` (48-byte preimage, unchanged)
//! - Solana leaf: `sha256("terp-hs-elig-leaf-v1" || 0x02 || pubkey[32] || amount_le[8])` (61-byte preimage)
//! - node:        `sha256("terp-hs-elig-node-v1" || left || right)`
//! - padding:     the zero leaf `[0; 32]` (no preimage opens it)
//!
//! Index bit `level` clear means the running node is the left child.

use std::collections::BTreeMap;

use base64::Engine;
use sha2::{Digest, Sha256};

pub const SOL_LEAF_TAG: u8 = 0x02;
pub const MAX_DEPTH: u32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Identity {
    Evm([u8; 20]),
    Sol([u8; 32]),
}

impl Identity {
    /// Same key the contract stores in its one-registration-per-identity map.
    pub fn key(&self) -> String {
        match self {
            Identity::Evm(a) => hex::encode(a),
            Identity::Sol(p) => format!("sol:{}", hex::encode(p)),
        }
    }
}

pub fn evm_leaf(address: &[u8; 20], amount: u64) -> [u8; 32] {
    let mut pre = b"terp-hs-elig-leaf-v1".to_vec();
    pre.extend_from_slice(address);
    pre.extend_from_slice(&amount.to_le_bytes());
    sha256(&pre)
}

pub fn sol_leaf(pubkey: &[u8; 32], amount: u64) -> [u8; 32] {
    let mut pre = b"terp-hs-elig-leaf-v1".to_vec();
    pre.push(SOL_LEAF_TAG);
    pre.extend_from_slice(pubkey);
    pre.extend_from_slice(&amount.to_le_bytes());
    sha256(&pre)
}

pub fn leaf(id: &Identity, amount: u64) -> [u8; 32] {
    match id {
        Identity::Evm(a) => evm_leaf(a, amount),
        Identity::Sol(p) => sol_leaf(p, amount),
    }
}

pub fn node(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut pre = b"terp-hs-elig-node-v1".to_vec();
    pre.extend_from_slice(left);
    pre.extend_from_slice(right);
    sha256(&pre)
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolDrop {
    /// Neither base64(base58) nor base58 decoding produced bytes.
    Encoding,
    /// Decoded, but not exactly 32 bytes.
    Length,
    /// 32 bytes that are not a valid ed25519 point (e.g. a PDA): cannot sign.
    OffCurve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolForm {
    Base64OfBase58,
    Base58,
}

/// Decode one snapshot entry. The documented storage form is base64 of the
/// base58 string; plain base58 is accepted as a fallback and reported.
pub fn decode_sol(entry: &str) -> Result<([u8; 32], SolForm), SolDrop> {
    let entry = entry.trim();
    let (raw, form) = match base64::engine::general_purpose::STANDARD
        .decode(entry)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| bs58::decode(s.trim()).into_vec().ok())
    {
        Some(raw) => (raw, SolForm::Base64OfBase58),
        None => (
            bs58::decode(entry).into_vec().map_err(|_| SolDrop::Encoding)?,
            SolForm::Base58,
        ),
    };
    let pk: [u8; 32] = raw.try_into().map_err(|_| SolDrop::Length)?;
    if ed25519_dalek::VerifyingKey::from_bytes(&pk).is_err() {
        return Err(SolDrop::OffCurve);
    }
    Ok((pk, form))
}

pub fn decode_evm(entry: &str) -> Option<[u8; 20]> {
    let t = entry.trim().strip_prefix("0x").or_else(|| entry.trim().strip_prefix("0X"))?;
    let raw = hex::decode(t.to_ascii_lowercase()).ok()?;
    raw.try_into().ok()
}

/// Fixed-depth tree over sorted identities; all levels kept for path lookups.
pub struct EligTree {
    pub depth: u32,
    pub entries: Vec<(Identity, u64)>,
    levels: Vec<Vec<[u8; 32]>>,
}

impl EligTree {
    /// Aggregates duplicate identities (sum of amounts) and sorts by identity key.
    pub fn build(rows: impl IntoIterator<Item = (Identity, u64)>) -> Result<Self, String> {
        let mut agg: BTreeMap<String, (Identity, u64)> = BTreeMap::new();
        for (id, amount) in rows {
            if amount == 0 {
                continue;
            }
            let slot = agg.entry(id.key()).or_insert((id, 0));
            slot.1 = slot.1.checked_add(amount).ok_or("amount overflow")?;
        }
        let entries: Vec<(Identity, u64)> = agg.into_values().collect();
        if entries.is_empty() {
            return Err("empty".into());
        }
        let mut depth = 1u32;
        while (1usize << depth) < entries.len() {
            depth += 1;
        }
        if depth > MAX_DEPTH {
            return Err(format!("{} identities exceed depth {MAX_DEPTH}", entries.len()));
        }
        Ok(Self::with_depth(entries, depth))
    }

    pub fn with_depth(entries: Vec<(Identity, u64)>, depth: u32) -> Self {
        let mut level0 = vec![[0u8; 32]; 1usize << depth];
        for (i, (id, amount)) in entries.iter().enumerate() {
            level0[i] = leaf(id, *amount);
        }
        let mut levels = vec![level0];
        for _ in 0..depth {
            let prev = levels.last().unwrap();
            let next = prev.chunks(2).map(|p| node(&p[0], &p[1])).collect();
            levels.push(next);
        }
        Self {
            depth,
            entries,
            levels,
        }
    }

    pub fn root(&self) -> [u8; 32] {
        self.levels[self.depth as usize][0]
    }

    pub fn path(&self, index: usize) -> Vec<[u8; 32]> {
        (0..self.depth as usize)
            .map(|l| self.levels[l][(index >> l) ^ 1])
            .collect()
    }

    pub fn index_of(&self, id: &Identity) -> Option<usize> {
        self.entries.iter().position(|(e, _)| e == id)
    }
}

pub fn open(leaf: [u8; 32], index: usize, path: &[[u8; 32]]) -> [u8; 32] {
    let mut n = leaf;
    for (l, s) in path.iter().enumerate() {
        n = if (index >> l) & 1 == 0 { node(&n, s) } else { node(s, &n) };
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evm_leaf_encoding_is_unchanged() {
        // Live loopback root from the two-leaf EVM eligibility file (relay test vector).
        let a: [u8; 20] = hex::decode("7e5f4552091a69125d5dfcb7b8c2659029395bdf").unwrap().try_into().unwrap();
        let b: [u8; 20] = hex::decode("2b5ad5c4795c026514f8317c7a215e218dccd6cf").unwrap().try_into().unwrap();
        let t = EligTree::with_depth(vec![(Identity::Evm(a), 15), (Identity::Evm(b), 15)], 1);
        assert!(hex::encode(t.root()).starts_with("ddfd4cb0"));
    }

    #[test]
    fn sol_and_evm_leaves_differ_and_paths_open() {
        let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let pk = sk.verifying_key().to_bytes();
        let mut a = [0u8; 20];
        a.copy_from_slice(&pk[..20]);
        assert_ne!(sol_leaf(&pk, 5), evm_leaf(&a, 5));
        let t = EligTree::build(vec![(Identity::Sol(pk), 5), (Identity::Evm(a), 5), (Identity::Sol(pk), 1)]).unwrap();
        assert_eq!(t.entries.len(), 2);
        let i = t.index_of(&Identity::Sol(pk)).unwrap();
        assert_eq!(t.entries[i].1, 6);
        assert_eq!(open(sol_leaf(&pk, 6), i, &t.path(i)), t.root());
    }

    #[test]
    fn decode_forms_and_drops() {
        let sk = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
        let pk = sk.verifying_key().to_bytes();
        let b58 = bs58::encode(pk).into_string();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&b58);
        assert_eq!(decode_sol(&b64).unwrap(), (pk, SolForm::Base64OfBase58));
        assert_eq!(decode_sol(&b58).unwrap(), (pk, SolForm::Base58));
        assert_eq!(decode_sol("0OIl").unwrap_err(), SolDrop::Encoding);
        assert_eq!(decode_sol(&bs58::encode([1u8; 31]).into_string()).unwrap_err(), SolDrop::Length);
        // Find a 32-byte string that is not a curve point.
        let off = (0u8..=255).map(|b| { let mut x = [b; 32]; x[31] &= 0x7f; x })
            .find(|x| ed25519_dalek::VerifyingKey::from_bytes(x).is_err()).unwrap();
        assert_eq!(decode_sol(&bs58::encode(off).into_string()).unwrap_err(), SolDrop::OffCurve);
    }
}
