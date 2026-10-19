//! Vote extension handling for ABCI++.
//!
//! The sidecar produces `VoteExtensionHashData` payloads and signs them
//! with a domain-separated scheme so they cannot be replayed across chains
//! or heights.
//!
//! ## Domain separation
//!
//! ```text
//! SHA256(domain_sep || chain_id || height_be8 || ext_bytes)
//! ```
//!
//! where `domain_sep = b"terp/hashmerchant/ve/v1"`.

use crate::custody::Custody;
use crate::msg::VoteExtensionHashData;
use k256::ecdsa::signature::hazmat::PrehashVerifier as _;
use sha2::{Digest, Sha256};
const DOMAIN_SEP: &[u8] = b"terp/hashmerchant/ve/v1";

/// Handles signing and verifying vote extension payloads.
pub struct VoteExtensionHandler {
    custody: Box<dyn Custody>,
}

impl VoteExtensionHandler {
    pub fn new(custody: Box<dyn Custody>) -> Self {
        Self { custody }
    }

    /// Encode and sign a vote extension for the given chain/height.
    pub async fn sign_extension(
        &self,
        chain_id: &str,
        height: u64,
        data: &VoteExtensionHashData,
    ) -> anyhow::Result<SignedVoteExtension> {
        let ext_bytes = data.encode();
        let digest = Self::domain_hash(chain_id, height, &ext_bytes);
        let signature = self.custody.sign(&digest).await?;

        Ok(SignedVoteExtension {
            extension: ext_bytes,
            signature,
            public_key: self.custody.public_key().to_vec(),
        })
    }

    /// Verify a signed vote extension.
    pub fn verify_extension(
        &self,
        chain_id: &str,
        height: u64,
        signed: &SignedVoteExtension,
    ) -> anyhow::Result<VoteExtensionHashData> {
        let digest = Self::domain_hash(chain_id, height, &signed.extension);

        // Verify using k256 (secp256k1)
        use k256::ecdsa::{Signature, VerifyingKey, signature::hazmat::PrehashVerifier};
        let vk = VerifyingKey::from_sec1_bytes(&signed.public_key)?;
        let sig = Signature::try_from(signed.signature.as_slice())?;
        vk.verify_prehash(&digest, &sig)?;

        VoteExtensionHashData::decode(&signed.extension)
    }

    /// Compute the domain-separated hash: SHA256(domain_sep || chain_id || height_be8 || ext_bytes).
    fn domain_hash(chain_id: &str, height: u64, ext_bytes: &[u8]) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(DOMAIN_SEP);
        hasher.update(chain_id.as_bytes());
        hasher.update(height.to_be_bytes());
        hasher.update(ext_bytes);
        hasher.finalize().to_vec()
    }

    /// Access the underlying custody's public key.
    pub fn public_key(&self) -> &[u8] {
        self.custody.public_key()
    }
}

/// A signed vote extension ready for transmission.
#[derive(Debug, Clone)]
pub struct SignedVoteExtension {
    /// Protobuf-encoded `VoteExtensionHashData`
    pub extension: Vec<u8>,
    /// Domain-separated signature over the extension
    pub signature: Vec<u8>,
    /// Signer's public key (compressed SEC1)
    pub public_key: Vec<u8>,
}
