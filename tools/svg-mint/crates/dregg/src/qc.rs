//! Ed25519 M-of-N checkpoints. Same verify family as `cosmos-lock` `count_oracle_sigs`.
//! HTTP without a verifying QC is not a cell turn.

use ed25519_dalek::{Keypair, PublicKey, Signature, Signer, Verifier};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const CKPT_DOMAIN: &[u8] = b"svg-mint-fed-ckpt-v1";

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum QcError {
    #[error("unsigned")]
    Unsigned,
    #[error("dummy receipt")]
    DummyReceipt,
    #[error("threshold not met: got {got} need {need}")]
    Threshold { got: u32, need: u32 },
    #[error("bad key")]
    BadKey,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OracleSig {
    pub pubkey: [u8; 32],
    pub signature: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Checkpoint {
    pub epoch: u64,
    pub cell_root: [u8; 32],
    pub n_deposit: [u8; 32],
    pub turn_tag: String,
    /// blake3 of this checkpoint body (excluding signatures). Packet receipt.
    pub digest: [u8; 32],
}

impl Checkpoint {
    pub fn new(epoch: u64, cell_root: [u8; 32], n_deposit: [u8; 32], turn_tag: &str) -> Self {
        let mut body = Vec::new();
        body.extend_from_slice(CKPT_DOMAIN);
        body.extend_from_slice(&epoch.to_be_bytes());
        body.extend_from_slice(&cell_root);
        body.extend_from_slice(&n_deposit);
        let tag = turn_tag.as_bytes();
        body.extend_from_slice(&(tag.len() as u64).to_be_bytes());
        body.extend_from_slice(tag);
        let digest = *blake3::hash(&body).as_bytes();
        Self {
            epoch,
            cell_root,
            n_deposit,
            turn_tag: turn_tag.into(),
            digest,
        }
    }
}

pub fn is_dummy_receipt(r: &[u8; 32]) -> bool {
    r.iter().all(|b| *b == r[0])
}

pub fn sign(kp: &Keypair, digest: &[u8; 32]) -> OracleSig {
    let sig = kp.sign(digest);
    OracleSig {
        pubkey: kp.public.to_bytes(),
        signature: sig.to_bytes().to_vec(),
    }
}

pub fn count_valid(digest: &[u8; 32], sigs: &[OracleSig], allowed: &[[u8; 32]], threshold: u32) -> Result<u32, QcError> {
    if sigs.is_empty() {
        return Err(QcError::Unsigned);
    }
    let mut got = 0u32;
    let mut seen: Vec<[u8; 32]> = Vec::new();
    for s in sigs {
        if !allowed.iter().any(|k| k == &s.pubkey) {
            continue;
        }
        if seen.iter().any(|k| k == &s.pubkey) {
            continue;
        }
        let pk = PublicKey::from_bytes(&s.pubkey).map_err(|_| QcError::BadKey)?;
        if s.signature.len() != 64 {
            continue;
        }
        let mut sb = [0u8; 64];
        sb.copy_from_slice(&s.signature);
        let sig = Signature::from_bytes(&sb).map_err(|_| QcError::BadKey)?;
        if pk.verify(digest, &sig).is_ok() {
            seen.push(s.pubkey);
            got += 1;
        }
    }
    if got < threshold {
        return Err(QcError::Threshold { got, need: threshold });
    }
    Ok(got)
}

pub fn require_receipt(receipt: &[u8; 32], digest: &[u8; 32], sigs: &[OracleSig], allowed: &[[u8; 32]], threshold: u32) -> Result<(), QcError> {
    if is_dummy_receipt(receipt) {
        return Err(QcError::DummyReceipt);
    }
    if receipt != digest {
        return Err(QcError::Unsigned);
    }
    count_valid(digest, sigs, allowed, threshold).map(|_| ())
}

pub fn generate_operator() -> Keypair {
    use rand::rngs::OsRng;
    Keypair::generate(&mut OsRng)
}
