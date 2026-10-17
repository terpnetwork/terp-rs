use alloc::vec::Vec;

use halo2_proofs::plonk;
use pasta_curves::vesta;
use rand::{CryptoRng, RngCore};
use tracing::debug;

use super::{Authorized, Bundle};
use crate::{
    circuit::VerifyingKey,
    primitives::redpallas::{self, Binding, SpendAuth},
};

/// A signature within an authorized Orchard bundle.
#[derive(Debug)]
struct BundleSignature {
    /// The signature item for validation.
    signature: redpallas::batch::Item<SpendAuth, Binding>,
}

/// Batch validation context for Headstash / Orchard proofs (+ optional RedPallas).
///
/// Use [`crate::circuit::Proof::add_to_batch`] to enqueue proofs. Contract
/// `ProcessHeadstash` still verifies sequentially via the CosmWasm ZK API; this
/// batch helper is for off-chain / multi-proof tooling.
#[derive(Debug, Default)]
pub struct BatchValidator {
    proofs: plonk::BatchVerifier<vesta::Affine>,
    signatures: Vec<BundleSignature>,
}

impl BatchValidator {
    /// Constructs a new batch validation context.
    pub fn new() -> Self {
        BatchValidator {
            proofs: plonk::BatchVerifier::new(),
            signatures: vec![],
        }
    }

    /// Access the underlying Halo2 batch verifier (for `Proof::add_to_batch`).
    pub fn proofs_mut(&mut self) -> &mut plonk::BatchVerifier<vesta::Affine> {
        &mut self.proofs
    }

    /// Number of queued signatures (proof count is internal to BatchVerifier).
    pub fn signature_count(&self) -> usize {
        self.signatures.len()
    }

    /// Batch-validate queued proofs (and RedPallas signatures if any).
    ///
    /// Empty batches are valid (no-op). Returns `true` iff all items verify.
    pub fn validate<R: RngCore + CryptoRng>(self, vk: &VerifyingKey, mut rng: R) -> bool {
        if !self.signatures.is_empty() {
            let mut validator = redpallas::batch::Verifier::new();
            for sig in self.signatures.iter() {
                validator.queue(sig.signature.clone());
            }
            if let Err(e) = validator.verify(&mut rng) {
                debug!("RedPallas batch validation failed: {}", e);
                return false;
            }
        }
        // Halo2 batch finalize: true if all enqueued proofs verify.
        self.proofs.finalize(&vk.params, &vk.vk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_batch_is_valid() {
        let vk = VerifyingKey::build();
        let batch = BatchValidator::new();
        // Empty Halo2 batch finalize is true in halo2_proofs batch verifier.
        assert!(batch.validate(&vk, rand::thread_rng()));
    }
}
