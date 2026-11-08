//! DIY IAVL node hashing. Feature `bench` only — production RecvPacket uses
//! ics23 `apply_leaf` / `apply_inner` (`HashOp` on each op).

use crate::{HashAlg, ProofError};
use sha2::{Digest, Sha256};

impl HashAlg {
    pub(crate) fn digest(self, preimage: &[u8]) -> [u8; 32] {
        match self {
            Self::Sha256 => Sha256::digest(preimage).into(),
            Self::Blake3 => *blake3::hash(preimage).as_bytes(),
        }
    }
}

/// One inner step from leaf toward root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InnerStep {
    pub height: i8,
    pub size: i64,
    pub version: i64,
    pub sibling: Vec<u8>,
    pub sibling_is_left: bool,
}

/// Existence proof for one key in a single IAVL tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExistenceProof {
    pub height: i8,
    pub size: i64,
    pub version: i64,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub path: Vec<InnerStep>,
}

fn put_uvarint(mut u: u64) -> Vec<u8> {
    let mut out = Vec::new();
    while u >= 0x80 {
        out.push(((u as u8) & 0x7f) | 0x80);
        u >>= 7;
    }
    out.push(u as u8);
    out
}

fn put_varint(v: i64) -> Vec<u8> {
    let mut ux = (v as u64) << 1;
    if v < 0 {
        ux = !ux;
    }
    put_uvarint(ux)
}

fn len_prefixed(p: &[u8]) -> Vec<u8> {
    let mut out = put_uvarint(p.len() as u64);
    out.extend_from_slice(p);
    out
}

#[must_use]
pub fn hash_value(alg: HashAlg, value: &[u8]) -> [u8; 32] {
    alg.digest(value)
}

#[must_use]
pub fn hash_leaf(
    alg: HashAlg,
    height: i8,
    size: i64,
    version: i64,
    key: &[u8],
    value_hash: &[u8],
) -> [u8; 32] {
    let mut pre = Vec::new();
    pre.extend(put_varint(i64::from(height)));
    pre.extend(put_varint(size));
    pre.extend(put_varint(version));
    pre.extend(len_prefixed(key));
    pre.extend(len_prefixed(value_hash));
    alg.digest(&pre)
}

#[must_use]
pub fn hash_inner(
    alg: HashAlg,
    height: i8,
    size: i64,
    version: i64,
    left: &[u8],
    right: &[u8],
) -> [u8; 32] {
    let mut pre = Vec::new();
    pre.extend(put_varint(i64::from(height)));
    pre.extend(put_varint(size));
    pre.extend(put_varint(version));
    pre.extend(len_prefixed(left));
    pre.extend(len_prefixed(right));
    alg.digest(&pre)
}

pub fn verify_existence(
    alg: HashAlg,
    root: &[u8; 32],
    proof: &ExistenceProof,
) -> Result<(), ProofError> {
    let vh = hash_value(alg, &proof.value);
    let mut cur = hash_leaf(
        alg,
        proof.height,
        proof.size,
        proof.version,
        &proof.key,
        &vh,
    );
    for step in &proof.path {
        if step.sibling.len() != 32 {
            return Err(ProofError::Sibling);
        }
        let (left, right) = if step.sibling_is_left {
            (step.sibling.as_slice(), cur.as_slice())
        } else {
            (cur.as_slice(), step.sibling.as_slice())
        };
        cur = hash_inner(alg, step.height, step.size, step.version, left, right);
    }
    if &cur != root {
        return Err(ProofError::RootMismatch);
    }
    Ok(())
}
