//! Native claim sidecar.
//!
//! Each submission is a Ligerito proof. This path verifies that proof against
//! the circuit shape of the claimed leaf index, checks the MetaMask signature
//! against the leaf key beside the proof, and inserts the nullifier once.
//! The batch circuit keeps folding those tuples. It does not replay the
//! Ligerito proof. The Flock circuit does not check the signature.

use std::collections::HashSet;

use headstash_wire::{
    canonical_batch, recover_pubkey_x, sign_batch, verify_batch, BatchPayout, WireError,
};

use crate::{verify_claim, Claim, ClaimProof, VerifyError};

#[derive(Debug)]
pub enum SidecarError {
    Wire(WireError),
    LeafKey,
    Root,
    Spent,
    Proof(VerifyError),
}

pub struct Sidecar {
    root: [u32; 8],
    spent: HashSet<[u32; 8]>,
    accepted: Vec<BatchPayout>,
    rejected: u32,
    secret: [u8; 32],
}

pub struct SignedBatch {
    pub root: [u8; 32],
    pub rejected: u32,
    pub payouts: Vec<BatchPayout>,
    pub public: [u8; 33],
    pub signature: [u8; 64],
    pub message: Vec<u8>,
}

impl Sidecar {
    pub fn new(root: [u32; 8], secret: [u8; 32]) -> Self {
        Self {
            root,
            spent: HashSet::new(),
            accepted: Vec::new(),
            rejected: 0,
            secret,
        }
    }

    /// Accept one native proof whose signature recovers `leaf_epk_x`.
    ///
    /// `leaf_epk_x` is the key recovered from the holder's signature. The
    /// proof publishes a Blake3 commitment of that key, not the key. `index`
    /// selects the Merkle wiring the verifier rebuilds. `rseed` stays with
    /// the holder. The batch that goes on chain does not carry the key.
    pub fn accept<const D: usize>(
        &mut self,
        index: u32,
        leaf_epk_x: &[u8; 32],
        message: &[u8],
        signature: &[u8],
        proved: &ClaimProof,
    ) -> Result<Claim, SidecarError> {
        let recovered = recover_pubkey_x(message, signature).map_err(SidecarError::Wire)?;
        if recovered != *leaf_epk_x {
            self.rejected += 1;
            return Err(SidecarError::LeafKey);
        }
        let claim = match verify_claim::<D>(index, proved) {
            Ok(claim) => claim,
            Err(err) => {
                self.rejected += 1;
                return Err(SidecarError::Proof(err));
            }
        };
        let published_key = crate::public_key_com(&proved.public)
            .ok_or(SidecarError::Proof(VerifyError::Public))?;
        if published_key != crate::key_com(&crate::tree::epk_words(leaf_epk_x)) {
            self.rejected += 1;
            return Err(SidecarError::LeafKey);
        }
        if claim.root != self.root {
            self.rejected += 1;
            return Err(SidecarError::Root);
        }
        if !self.spent.insert(claim.nullifier) {
            self.rejected += 1;
            return Err(SidecarError::Spent);
        }
        self.accepted.push(BatchPayout {
            destination: destination_string(&claim.destination),
            amount: claim.amount as u128,
            nullifier: words_to_bytes(&claim.nullifier),
        });
        Ok(claim)
    }

    pub fn seal(self) -> SignedBatch {
        let folded = crate::batch::egress(
            &self
                .accepted
                .iter()
                .map(|payout| crate::batch::Payout {
                    nullifier: bytes_to_words(&payout.nullifier),
                    destination: destination_words(&payout.destination),
                    amount: payout.amount as u32,
                })
                .collect::<Vec<_>>(),
            self.rejected,
        );
        let root = words_to_bytes(&folded.nullifier_root);
        let message = canonical_batch(&root, self.rejected, &self.accepted);
        let signature = sign_batch(&self.secret, &message);
        let public = headstash_wire::batch_public(&self.secret);
        verify_batch(&public, &message, &signature).expect("sidecar signs its own batch");
        SignedBatch {
            root,
            rejected: self.rejected,
            payouts: self.accepted,
            public,
            signature,
            message,
        }
    }
}

fn destination_string(words: &[u32; 5]) -> String {
    words.iter().map(|word| format!("{word:08x}")).collect()
}

fn destination_words(text: &str) -> [u32; 5] {
    let mut out = [0u32; 5];
    let bytes = text.as_bytes();
    for (i, slot) in out.iter_mut().enumerate() {
        let start = i * 8;
        if start >= bytes.len() {
            break;
        }
        let end = (start + 8).min(bytes.len());
        let mut buf = [0u8; 8];
        buf[..end - start].copy_from_slice(&bytes[start..end]);
        *slot = u32::from_str_radix(std::str::from_utf8(&buf).unwrap_or("0"), 16).unwrap_or(0);
    }
    out
}

pub(crate) fn words_to_bytes(words: &[u32; 8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, word) in words.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&word.to_le_bytes());
    }
    out
}

fn bytes_to_words(bytes: &[u8; 32]) -> [u32; 8] {
    let mut out = [0u32; 8];
    for i in 0..8 {
        out[i] = u32::from_le_bytes(bytes[4 * i..4 * i + 4].try_into().unwrap());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FIXTURE_DEPTH;
    use headstash_wire::sign_personal;

    fn opening(rseed0: u32, leaf_x: &[u8; 32]) -> crate::Opening<FIXTURE_DEPTH> {
        let mut siblings = [[0u32; 8]; FIXTURE_DEPTH];
        siblings[0] = [21, 22, 23, 24, 25, 26, 27, 28];
        siblings[1] = [31, 32, 33, 34, 35, 36, 37, 38];
        crate::Opening {
            rseed: [rseed0, 2, 3, 4, 5, 6, 7, 8],
            epk_x: crate::tree::epk_words(leaf_x),
            nd: [13, 14],
            v: 15,
            fdi: 1,
            index: 1,
            siblings,
            destination: [0x1111, 0x2222, 0x3333, 0x4444, 0x5555],
        }
    }

    #[test]
    fn one_nullifier_seals_a_signed_batch_and_a_second_spend_is_rejected() {
        let message = b"headstash-claim-v1";
        let sig = sign_personal(&[4u8; 32], message).unwrap();
        let leaf_x = recover_pubkey_x(message, &sig).unwrap();
        let opening = opening(1, &leaf_x);
        let proved = crate::prove_claim(&opening);
        let claim = crate::derive(&opening);
        let mut side = Sidecar::new(claim.root, [8u8; 32]);
        side.accept::<FIXTURE_DEPTH>(opening.index, &leaf_x, message, &sig, &proved)
            .unwrap();
        let again = side
            .accept::<FIXTURE_DEPTH>(opening.index, &leaf_x, message, &sig, &proved)
            .unwrap_err();
        assert!(matches!(again, SidecarError::Spent));

        let wrong = sign_personal(&[5u8; 32], message).unwrap();
        let wrong_x = recover_pubkey_x(message, &wrong).unwrap();
        let mismatched = side
            .accept::<FIXTURE_DEPTH>(opening.index, &wrong_x, message, &sig, &proved)
            .unwrap_err();
        assert!(matches!(mismatched, SidecarError::LeafKey));

        let sealed = side.seal();
        assert_eq!(sealed.payouts.len(), 1);
        assert_eq!(sealed.rejected, 2);
        verify_batch(&sealed.public, &sealed.message, &sealed.signature).unwrap();
    }
}
