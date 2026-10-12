//! Opt-in re-zerve deposit transcript (feature `re-zerve`).
//!
//! Local domain-separated SHA-256. **Not** a Zcash inclusion proof.
//! Product crate remains `~/abstract/re-zerve`; this is the HashMerchant ingress seam.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DepositError {
    EmptyRoot,
    EmptyProof,
    Mismatch,
    FrostBinding,
    ZeroAmount,
}

impl std::fmt::Display for DepositError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DepositError::EmptyRoot => write!(f, "empty state root"),
            DepositError::EmptyProof => write!(f, "empty deposit proof"),
            DepositError::Mismatch => write!(f, "state root / proof mismatch"),
            DepositError::FrostBinding => write!(f, "frost group binding mismatch"),
            DepositError::ZeroAmount => write!(f, "zero amount"),
        }
    }
}

pub fn tagged_hash(tag: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(tag);
    h.update(&[tag.len() as u8]);
    for p in parts {
        h.update(&(p.len() as u64).to_le_bytes());
        h.update(p);
    }
    h.finalize().into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepositAttestation {
    pub transcript_root: [u8; 32],
    pub deposit_note_commitment: [u8; 32],
    pub amount_zat: u64,
    pub frost_group_id: [u8; 32],
    pub inclusion_proof: Vec<u8>,
}

impl DepositAttestation {
    pub fn prove(
        transcript_root: [u8; 32],
        deposit_note_commitment: [u8; 32],
        amount_zat: u64,
        frost_group_id: [u8; 32],
    ) -> Self {
        let inclusion_proof = tagged_hash(
            b"hashmerchant-deposit-v1",
            &[
                &transcript_root,
                &deposit_note_commitment,
                &amount_zat.to_le_bytes(),
                &frost_group_id,
            ],
        )
        .to_vec();
        Self {
            transcript_root,
            deposit_note_commitment,
            amount_zat,
            frost_group_id,
            inclusion_proof,
        }
    }

    pub fn verify(&self, expected_frost_group: &[u8; 32]) -> Result<(), DepositError> {
        if self.amount_zat == 0 {
            return Err(DepositError::ZeroAmount);
        }
        if self.transcript_root == [0u8; 32] {
            return Err(DepositError::EmptyRoot);
        }
        if self.inclusion_proof.is_empty() {
            return Err(DepositError::EmptyProof);
        }
        if &self.frost_group_id != expected_frost_group {
            return Err(DepositError::FrostBinding);
        }
        let expected = tagged_hash(
            b"hashmerchant-deposit-v1",
            &[
                &self.transcript_root,
                &self.deposit_note_commitment,
                &self.amount_zat.to_le_bytes(),
                &self.frost_group_id,
            ],
        );
        if self.inclusion_proof.as_slice() != expected.as_slice() {
            return Err(DepositError::Mismatch);
        }
        Ok(())
    }
}

pub struct HashmerchantIngress {
    pub frost_group_id: [u8; 32],
}

impl HashmerchantIngress {
    pub fn new(frost_group_id: [u8; 32]) -> Self {
        Self { frost_group_id }
    }

    pub fn accept(&self, att: &DepositAttestation) -> Result<(), DepositError> {
        att.verify(&self.frost_group_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prove_verify_roundtrip() {
        let g = [7u8; 32];
        let att = DepositAttestation::prove([1u8; 32], [2u8; 32], 100, g);
        HashmerchantIngress::new(g).accept(&att).unwrap();
    }

    #[test]
    fn wrong_group_rejected() {
        let att = DepositAttestation::prove([1u8; 32], [2u8; 32], 100, [7u8; 32]);
        assert!(HashmerchantIngress::new([8u8; 32]).accept(&att).is_err());
    }
}
