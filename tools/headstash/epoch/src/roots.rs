//! Roots the cw-headstash sudo recomputes.
//!
//! `pending_root` sorts digests first. `epoch::pending_root` does not, so a
//! later sudo must sort before it hashes or it will disagree with this
//! signature. `accepted_root` matches `epoch::accepted_root`: the destination
//! is the UTF-8 bech32 string, with a little-endian `u16` length.

use sha2::{Digest, Sha256};

pub const CHAIN_UID: &str = "headstash-claim";
pub const ALGO_PENDING: &str = "headstash-pending";
pub const ALGO_ACCEPTED: &str = "headstash-accepted";
pub const BATCH_SOURCE_ID: &str = "headstash-batch";
pub const BATCH_SCOPE: &str = "headstash-batch";
pub const BATCH_ATTEST_HEIGHT: u64 = 1;
pub const BATCH_ATTEST_TIMESTAMP: i64 = 1;

const PENDING_DOMAIN: &[u8] = b"terp-headstash-pending-v1";
const ACCEPTED_DOMAIN: &[u8] = b"terp-headstash-accepted-v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayoutRow {
    pub nullifier: [u8; 32],
    pub denom: [u8; 32],
    pub destination: String,
    pub amount: u64,
}

/// One signed vote. A zero `vk_hash` publishes `batch_hash` as the
/// attestation value. Any other hash appends those 32 bytes. `attested`
/// stays the aggregate of the 32-byte batch hash, so an older contract
/// still pays. `foreign_height` is the chain height that picked the algo.
/// The attestation height stays [`BATCH_ATTEST_HEIGHT`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedVote {
    pub algo: String,
    pub batch_hash: [u8; 32],
    pub vk_hash: [u8; 32],
    pub attested: [u8; 32],
    pub custody_signature: [u8; 64],
    pub custody_public: [u8; 32],
    pub attestation_height: u64,
    pub attestation_timestamp: i64,
    pub foreign_height: u64,
}

impl SealedVote {
    pub fn custody_ok(&self) -> bool {
        verify_custody(
            &self.custody_public,
            &self.custody_signature,
            &self.attestation_value(),
        )
    }

    pub fn attestation_value(&self) -> Vec<u8> {
        attestation_value(&self.batch_hash, &self.vk_hash)
    }
}

/// Bytes the OracleSource signs and the module copies into the sudo hint.
/// A zero verifying-key hash stays 32 bytes so flock votes do not grow.
pub fn attestation_value(batch_hash: &[u8; 32], vk_hash: &[u8; 32]) -> Vec<u8> {
    if vk_hash.iter().all(|byte| *byte == 0) {
        return batch_hash.to_vec();
    }
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(batch_hash);
    out.extend_from_slice(vk_hash);
    out
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let dig = Sha256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&dig);
    out
}

/// Contract preimage over digests this sidecar has sorted.
pub fn pending_root(digests: &[[u8; 32]]) -> [u8; 32] {
    let mut ordered = digests.to_vec();
    ordered.sort();
    hash_pending(&ordered)
}

/// The contract's own order. Exposed so a test can show the sort matters.
pub fn pending_root_unsorted(digests: &[[u8; 32]]) -> [u8; 32] {
    hash_pending(digests)
}

fn hash_pending(digests: &[[u8; 32]]) -> [u8; 32] {
    let mut pre = Vec::with_capacity(PENDING_DOMAIN.len() + 4 + digests.len() * 32);
    pre.extend_from_slice(PENDING_DOMAIN);
    pre.extend_from_slice(&(digests.len() as u32).to_le_bytes());
    for digest in digests {
        pre.extend_from_slice(digest);
    }
    sha256(&pre)
}

pub fn accepted_root(allocation_root: &[u8; 32], rows: &[PayoutRow]) -> [u8; 32] {
    let mut ordered = rows.to_vec();
    ordered.sort_by(|a, b| a.nullifier.cmp(&b.nullifier));
    let mut pre = Vec::new();
    pre.extend_from_slice(ACCEPTED_DOMAIN);
    pre.extend_from_slice(allocation_root);
    pre.extend_from_slice(&(ordered.len() as u32).to_le_bytes());
    for row in &ordered {
        pre.extend_from_slice(&row.nullifier);
        pre.extend_from_slice(&row.denom);
        let dest = row.destination.as_bytes();
        pre.extend_from_slice(&(dest.len() as u16).to_le_bytes());
        pre.extend_from_slice(dest);
        pre.extend_from_slice(&row.amount.to_le_bytes());
    }
    sha256(&pre)
}

/// SHA-256 of `source_id || batch_hash || height_le` for the one attestation.
pub fn attested_root(batch_hash: &[u8; 32]) -> [u8; 32] {
    let mut pre = Vec::with_capacity(BATCH_SOURCE_ID.len() + 32 + 8);
    pre.extend_from_slice(BATCH_SOURCE_ID.as_bytes());
    pre.extend_from_slice(batch_hash);
    pre.extend_from_slice(&BATCH_ATTEST_HEIGHT.to_le_bytes());
    sha256(&pre)
}

/// Message the OracleSource ed25519 key signs. Timestamp is not in the aggregate.
pub fn custody_message(value: &[u8]) -> Vec<u8> {
    let domain = sha256(format!("hashmerchant/custody/{BATCH_SCOPE}/{BATCH_SOURCE_ID}").as_bytes());
    let mut payload = Vec::with_capacity(32 + value.len() + 16);
    payload.extend_from_slice(&domain);
    payload.extend_from_slice(value);
    payload.extend_from_slice(&BATCH_ATTEST_HEIGHT.to_le_bytes());
    payload.extend_from_slice(&(BATCH_ATTEST_TIMESTAMP as u64).to_le_bytes());
    payload
}

pub fn sign_custody(key: &ed25519_dalek::SigningKey, value: &[u8]) -> [u8; 64] {
    use ed25519_dalek::Signer;
    key.sign(&custody_message(value)).to_bytes()
}

pub fn verify_custody(public: &[u8; 32], signature: &[u8; 64], value: &[u8]) -> bool {
    verify_ed25519(public, signature, &custody_message(value))
}

pub fn verify_ed25519(public: &[u8; 32], signature: &[u8; 64], message: &[u8]) -> bool {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let Ok(vk) = VerifyingKey::from_bytes(public) else {
        return false;
    };
    let Ok(sig) = Signature::from_slice(signature) else {
        return false;
    };
    vk.verify(message, &sig).is_ok()
}

pub fn receipt_message(
    ctx: &[u8; 32],
    root: &[u8; 32],
    nullifier: &[u8; 32],
    destination: &str,
    amount: u64,
    digest: &[u8; 32],
) -> Vec<u8> {
    format!(
        "headstash-receipt-v1:{}:{}:{}:{}:{}:{}",
        hex::encode(ctx),
        hex::encode(root),
        hex::encode(nullifier),
        destination,
        amount,
        hex::encode(digest),
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_root_sorts_before_the_contract_preimage() {
        let hi = [2u8; 32];
        let lo = [1u8; 32];
        assert_eq!(pending_root(&[hi, lo]), pending_root(&[lo, hi]));
        assert_ne!(pending_root(&[hi, lo]), pending_root_unsorted(&[hi, lo]));

        let mut pre = PENDING_DOMAIN.to_vec();
        pre.extend_from_slice(&2u32.to_le_bytes());
        pre.extend_from_slice(&lo);
        pre.extend_from_slice(&hi);
        assert_eq!(pending_root(&[hi, lo]), sha256(&pre));
    }

    #[test]
    fn accepted_root_hashes_the_bech32_string() {
        let root = [7u8; 32];
        let denom = [9u8; 32];
        let row = PayoutRow {
            nullifier: [1u8; 32],
            denom,
            destination: "terp1a".into(),
            amount: 5,
        };
        let swapped = PayoutRow {
            nullifier: [2u8; 32],
            ..row.clone()
        };
        assert_eq!(
            accepted_root(&root, &[row.clone(), swapped.clone()]),
            accepted_root(&root, &[swapped, row.clone()])
        );

        let mut pre = ACCEPTED_DOMAIN.to_vec();
        pre.extend_from_slice(&root);
        pre.extend_from_slice(&1u32.to_le_bytes());
        pre.extend_from_slice(&row.nullifier);
        pre.extend_from_slice(&row.denom);
        let dest = b"terp1a";
        pre.extend_from_slice(&(dest.len() as u16).to_le_bytes());
        pre.extend_from_slice(dest);
        pre.extend_from_slice(&5u64.to_le_bytes());
        assert_eq!(accepted_root(&root, &[row]), sha256(&pre));

        let mut raw20 = pre.clone();
        // A 20-byte address encoding is a different preimage.
        raw20.truncate(ACCEPTED_DOMAIN.len() + 32 + 4 + 32 + 32);
        raw20.extend_from_slice(&20u16.to_le_bytes());
        raw20.extend_from_slice(&[0u8; 20]);
        raw20.extend_from_slice(&5u64.to_le_bytes());
        assert_ne!(
            accepted_root(
                &root,
                &[PayoutRow {
                    nullifier: [1u8; 32],
                    denom,
                    destination: "terp1a".into(),
                    amount: 5,
                }]
            ),
            sha256(&raw20)
        );
    }

    #[test]
    fn attested_root_is_one_attestation_and_the_custody_message_verifies() {
        let batch = [4u8; 32];
        let mut pre = BATCH_SOURCE_ID.as_bytes().to_vec();
        pre.extend_from_slice(&batch);
        pre.extend_from_slice(&1u64.to_le_bytes());
        assert_eq!(attested_root(&batch), sha256(&pre));

        let key = ed25519_dalek::SigningKey::from_bytes(&[8u8; 32]);
        assert_eq!(attestation_value(&batch, &[0u8; 32]), batch.to_vec());
        let mut wide = batch.to_vec();
        wide.extend_from_slice(&[9u8; 32]);
        assert_eq!(attestation_value(&batch, &[9u8; 32]), wide);

        let sig = sign_custody(&key, &batch);
        let public = key.verifying_key().to_bytes();
        assert!(verify_custody(&public, &sig, &batch));
        assert!(!verify_custody(&public, &sig, &[5u8; 32]));
        let wide_sig = sign_custody(&key, &wide);
        assert!(verify_custody(&public, &wide_sig, &wide));
        assert!(!verify_custody(&public, &sig, &wide));
    }
}
