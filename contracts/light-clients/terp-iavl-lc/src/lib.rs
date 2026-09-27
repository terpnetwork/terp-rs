//! ICS-23 membership for Terp 08-wasm. Production hashes only through
//! `ics23::apply_leaf` / `apply_inner` (`HashOp` on each op).
//!
//! Feature `bench`: DIY IAVL node hashing and a wrong-alg cross-check.

mod host;

#[cfg(feature = "bench")]
mod iavl;

use host::Ics23Host;
use prost::Message;

#[cfg(feature = "bench")]
pub use iavl::{
    hash_inner, hash_leaf, hash_value, verify_existence, ExistenceProof, InnerStep,
};

/// Hash algorithm used by one IAVL tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HashAlg {
    Sha256,
    Blake3,
}

impl HashAlg {
    /// Store-name policy matching Go `iavl.HasherOptionForStore`.
    #[must_use]
    pub fn for_store(name: &str) -> Self {
        match name {
            "ibc" | "transfer" | "icahost" | "icacontroller"
            | "packetfowardmiddleware" | "packetforwardmiddleware"
            | "ibc-hooks" | "hooks-for-ibc" | "08-wasm" | "capability" => Self::Sha256,
            _ => Self::Blake3,
        }
    }

    /// `hybrid` (default): IBC stores SHA-256, else BLAKE3.
    /// `blake3` / `sha256`: every store.
    #[must_use]
    pub fn for_store_mode(name: &str, mode: &str) -> Self {
        match mode {
            "blake3" | "full" | "a" => Self::Blake3,
            "sha256" | "c" => Self::Sha256,
            _ => Self::for_store(name),
        }
    }

    fn ics23_spec(self) -> ics23::ProofSpec {
        match self {
            Self::Sha256 => ics23::iavl_spec(),
            Self::Blake3 => ics23::blake3_iavl_spec(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProofError {
    #[error("root mismatch")]
    RootMismatch,
    #[error("invalid sibling length")]
    Sibling,
    #[error("ics23 decode")]
    Ics23Decode,
    #[error("ics23 {0} spec rejected proof")]
    Ics23Reject(&'static str),
    #[error("ics23 wrong-alg unexpectedly accepted")]
    Ics23WrongAlg,
    #[error("ics23 chain: {0}")]
    Ics23Chain(&'static str),
}

/// IBC `ibc.core.commitment.v1.MerkleProof`: repeated `CommitmentProof` only.
#[derive(Clone, PartialEq, prost::Message)]
struct IbcMerkleProof {
    #[prost(message, repeated, tag = "1")]
    proofs: Vec<ics23::CommitmentProof>,
}

fn decode_proofs(bz: &[u8]) -> Result<Vec<ics23::CommitmentProof>, ProofError> {
    if let Ok(mp) = IbcMerkleProof::decode(bz) {
        if !mp.proofs.is_empty() {
            return Ok(mp.proofs);
        }
    }
    let one = ics23::CommitmentProof::decode(bz).map_err(|_| ProofError::Ics23Decode)?;
    Ok(vec![one])
}

fn reject_label(alg: HashAlg) -> &'static str {
    match alg {
        HashAlg::Blake3 => "blake3",
        HashAlg::Sha256 => "sha256",
    }
}

/// Infer IAVL hash alg from an ICS-23 proof's leaf `HashOp`.
pub fn hash_alg_from_iavl_proof(proof: &ics23::CommitmentProof) -> Option<HashAlg> {
    let leaf = match &proof.proof {
        Some(ics23::commitment_proof::Proof::Exist(ex)) => ex.leaf.as_ref()?,
        Some(ics23::commitment_proof::Proof::Nonexist(ne)) => ne
            .left
            .as_ref()
            .and_then(|e| e.leaf.as_ref())
            .or_else(|| ne.right.as_ref().and_then(|e| e.leaf.as_ref()))?,
        _ => return None,
    };
    if leaf.hash == ics23::HashOp::Blake3 as i32 {
        Some(HashAlg::Blake3)
    } else if leaf.hash == ics23::HashOp::Sha256 as i32 {
        Some(HashAlg::Sha256)
    } else {
        None
    }
}

fn resolve_mode<'a>(mode: &'a str, first_proof: &ics23::CommitmentProof) -> &'a str {
    if mode.is_empty() || mode == "auto" {
        match hash_alg_from_iavl_proof(first_proof) {
            Some(HashAlg::Blake3) => "blake3",
            Some(HashAlg::Sha256) => "sha256",
            None => "hybrid",
        }
    } else {
        mode
    }
}

fn iavl_and_simple_specs(
    store_name: &str,
    mode: &str,
    n: usize,
) -> Result<Vec<ics23::ProofSpec>, ProofError> {
    if n == 0 {
        return Err(ProofError::Ics23Chain("empty proof"));
    }
    let iavl = HashAlg::for_store_mode(store_name, mode).ics23_spec();
    let mut specs = Vec::with_capacity(n);
    specs.push(iavl);
    for _ in 1..n {
        specs.push(ics23::tendermint_spec());
    }
    Ok(specs)
}

fn store_name_from_path(path: &[Vec<u8>]) -> String {
    path.first()
        .map(|k| String::from_utf8_lossy(k).into_owned())
        .unwrap_or_else(|| "ibc".into())
}

fn layer_key<'a>(merkle_path: &'a [Vec<u8>], i: usize) -> Result<&'a [u8], ProofError> {
    let key_idx = merkle_path
        .len()
        .checked_sub(1)
        .and_then(|n| n.checked_sub(i))
        .ok_or(ProofError::Ics23Chain("path index"))?;
    Ok(&merkle_path[key_idx])
}

fn exist_layers(
    specs: &[ics23::ProofSpec],
    merkle_path: &[Vec<u8>],
    proofs: &[ics23::CommitmentProof],
    value: &[u8],
    app_root: &[u8],
) -> Result<(), ProofError> {
    if proofs.len() != specs.len() || merkle_path.len() != proofs.len() {
        return Err(ProofError::Ics23Chain("proof/path length"));
    }
    let mut current = value.to_vec();
    let mut root = Vec::new();
    for (i, proof) in proofs.iter().enumerate() {
        let key = layer_key(merkle_path, i)?;
        root = ics23::membership_root::<Ics23Host>(proof, &specs[i], key, &current)
            .ok_or(ProofError::Ics23Reject(if i == 0 { "iavl" } else { "tendermint" }))?;
        current = root.clone();
    }
    if root != app_root {
        return Err(ProofError::RootMismatch);
    }
    Ok(())
}

/// Two-level ICS-23 membership against the consensus app hash (IAVL + simple).
pub fn verify_chained_membership(
    mode: &str,
    merkle_path: &[Vec<u8>],
    proof_bz: &[u8],
    value: &[u8],
    app_root: &[u8],
) -> Result<(), ProofError> {
    if value.is_empty() {
        return Err(ProofError::Ics23Chain("empty value"));
    }
    let proofs = decode_proofs(proof_bz)?;
    let store = store_name_from_path(merkle_path);
    let mode = resolve_mode(mode, &proofs[0]);
    let specs = iavl_and_simple_specs(&store, mode, proofs.len())?;
    exist_layers(&specs, merkle_path, &proofs, value, app_root)?;
    #[cfg(feature = "bench")]
    {
        let iavl_key = merkle_path.last().map(|k| k.as_slice()).unwrap_or(value);
        cross_check_wrong_alg(
            HashAlg::for_store_mode(&store, mode),
            &proofs[0],
            iavl_key,
            value,
        )?;
    }
    Ok(())
}

/// Two-level ICS-23 non-membership (non-exist at IAVL, exist up the simple tree).
pub fn verify_chained_nonmembership(
    mode: &str,
    merkle_path: &[Vec<u8>],
    proof_bz: &[u8],
    app_root: &[u8],
) -> Result<(), ProofError> {
    let proofs = decode_proofs(proof_bz)?;
    if proofs.is_empty() || merkle_path.len() != proofs.len() {
        return Err(ProofError::Ics23Chain("proof/path length"));
    }
    let store = store_name_from_path(merkle_path);
    let mode = resolve_mode(mode, &proofs[0]);
    let specs = iavl_and_simple_specs(&store, mode, proofs.len())?;
    let key = merkle_path.last().ok_or(ProofError::Ics23Chain("empty path"))?;
    let subroot = match &proofs[0].proof {
        Some(ics23::commitment_proof::Proof::Nonexist(_)) => {
            let p = &proofs[0];
            let sub = nonexist_subroot(p)?;
            if !ics23::verify_non_membership::<Ics23Host>(p, &specs[0], &sub, key) {
                return Err(ProofError::Ics23Reject(
                    reject_label(HashAlg::for_store_mode(&store, mode)),
                ));
            }
            sub
        }
        _ => return Err(ProofError::Ics23Chain("expected non-existence proof")),
    };
    if proofs.len() == 1 {
        return if subroot == app_root {
            Ok(())
        } else {
            Err(ProofError::RootMismatch)
        };
    }
    exist_layers(
        &specs[1..],
        &merkle_path[..merkle_path.len() - 1],
        &proofs[1..],
        &subroot,
        app_root,
    )
}

fn nonexist_subroot(proof: &ics23::CommitmentProof) -> Result<Vec<u8>, ProofError> {
    match &proof.proof {
        Some(ics23::commitment_proof::Proof::Nonexist(np)) => {
            if let Some(left) = &np.left {
                return ics23::calculate_existence_root::<Ics23Host>(left)
                    .map_err(|_| ProofError::Ics23Chain("nonexist left root"));
            }
            if let Some(right) = &np.right {
                return ics23::calculate_existence_root::<Ics23Host>(right)
                    .map_err(|_| ProofError::Ics23Chain("nonexist right root"));
            }
            Err(ProofError::Ics23Chain("empty nonexist"))
        }
        _ => Err(ProofError::Ics23Chain("expected nonexist")),
    }
}

/// Single `CommitmentProof` existence (ConfirmIcs23 / lab).
pub fn verify_ics23_existence(
    alg: HashAlg,
    root: &[u8],
    proof_bz: &[u8],
    key: &[u8],
    value: &[u8],
) -> Result<(), ProofError> {
    let proofs = decode_proofs(proof_bz)?;
    let proof = proofs.first().ok_or(ProofError::Ics23Decode)?;
    let got = ics23::membership_root::<Ics23Host>(proof, &alg.ics23_spec(), key, value)
        .ok_or(ProofError::Ics23Reject(reject_label(alg)))?;
    if got != root {
        return Err(ProofError::RootMismatch);
    }
    #[cfg(feature = "bench")]
    cross_check_wrong_alg(alg, proof, key, value)?;
    Ok(())
}

/// Single `CommitmentProof` non-existence (ConfirmIcs23 / lab).
pub fn verify_ics23_nonexistence(
    alg: HashAlg,
    root: &[u8],
    proof_bz: &[u8],
    key: &[u8],
) -> Result<(), ProofError> {
    let proofs = decode_proofs(proof_bz)?;
    let proof = proofs.first().ok_or(ProofError::Ics23Decode)?;
    let root_v = root.to_vec();
    if !ics23::verify_non_membership::<Ics23Host>(proof, &alg.ics23_spec(), &root_v, key) {
        return Err(ProofError::Ics23Reject(reject_label(alg)));
    }
    #[cfg(feature = "bench")]
    {
        let other = match alg {
            HashAlg::Blake3 => HashAlg::Sha256,
            HashAlg::Sha256 => HashAlg::Blake3,
        };
        if ics23::verify_non_membership::<Ics23Host>(proof, &other.ics23_spec(), &root_v, key) {
            return Err(ProofError::Ics23WrongAlg);
        }
    }
    Ok(())
}

#[cfg(feature = "bench")]
fn cross_check_wrong_alg(
    alg: HashAlg,
    proof: &ics23::CommitmentProof,
    key: &[u8],
    value: &[u8],
) -> Result<(), ProofError> {
    let other = match alg {
        HashAlg::Blake3 => HashAlg::Sha256,
        HashAlg::Sha256 => HashAlg::Blake3,
    };
    if ics23::membership_root::<Ics23Host>(proof, &other.ics23_spec(), key, value).is_some() {
        return Err(ProofError::Ics23WrongAlg);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_policy() {
        assert_eq!(HashAlg::for_store("ibc"), HashAlg::Sha256);
        assert_eq!(HashAlg::for_store("bank"), HashAlg::Blake3);
    }

    fn exist_proof(hash: i32) -> ics23::CommitmentProof {
        ics23::CommitmentProof {
            proof: Some(ics23::commitment_proof::Proof::Exist(ics23::ExistenceProof {
                key: b"k".to_vec(),
                value: b"v".to_vec(),
                leaf: Some(ics23::LeafOp {
                    hash,
                    ..Default::default()
                }),
                path: vec![],
            })),
        }
    }

    #[test]
    fn hash_op_from_proof_selects_spec() {
        assert_eq!(
            hash_alg_from_iavl_proof(&exist_proof(ics23::HashOp::Blake3 as i32)),
            Some(HashAlg::Blake3)
        );
        assert_eq!(
            hash_alg_from_iavl_proof(&exist_proof(ics23::HashOp::Sha256 as i32)),
            Some(HashAlg::Sha256)
        );
        assert_eq!(
            resolve_mode("auto", &exist_proof(ics23::HashOp::Blake3 as i32)),
            "blake3"
        );
        assert_eq!(
            resolve_mode("hybrid", &exist_proof(ics23::HashOp::Blake3 as i32)),
            "hybrid"
        );
    }

    #[test]
    fn decode_single_commitment_proof() {
        let p = exist_proof(ics23::HashOp::Blake3 as i32);
        let bz = p.encode_to_vec();
        let got = decode_proofs(&bz).unwrap();
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn decode_ibc_merkle_wrapper() {
        let p = exist_proof(ics23::HashOp::Sha256 as i32);
        let mp = IbcMerkleProof {
            proofs: vec![p.clone(), p],
        };
        let got = decode_proofs(&mp.encode_to_vec()).unwrap();
        assert_eq!(got.len(), 2);
    }
}

#[cfg(all(test, feature = "bench"))]
mod bench_tests {
    use super::*;
    use crate::iavl::{hash_leaf, hash_value, verify_existence, ExistenceProof};

    #[test]
    fn blake3_empty_matches_go() {
        let h = HashAlg::Blake3.digest(&[]);
        const WANT: [u8; 32] = [
            0xaf, 0x13, 0x49, 0xb9, 0xf5, 0xf9, 0xa1, 0xa6, 0xa0, 0x40, 0x4d, 0xea, 0x36, 0xdc,
            0xc9, 0x49, 0x9b, 0xcb, 0x25, 0xc9, 0xad, 0xc1, 0x12, 0xb7, 0xcc, 0x9a, 0x93, 0xca,
            0xe4, 0x1f, 0x32, 0x62,
        ];
        assert_eq!(h, WANT);
    }

    #[test]
    fn sha256_empty_matches_go() {
        let h = HashAlg::Sha256.digest(&[]);
        const WANT: [u8; 32] = [
            0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f,
            0xb9, 0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b,
            0x78, 0x52, 0xb8, 0x55,
        ];
        assert_eq!(h, WANT);
    }

    #[test]
    fn single_leaf_proof() {
        let alg = HashAlg::Blake3;
        let key = b"k";
        let value = b"v";
        let vh = hash_value(alg, value);
        let root = hash_leaf(alg, 0, 1, 1, key, &vh);
        let proof = ExistenceProof {
            height: 0,
            size: 1,
            version: 1,
            key: key.to_vec(),
            value: value.to_vec(),
            path: vec![],
        };
        verify_existence(alg, &root, &proof).unwrap();
    }
}
