//! Local file-based custody — development / single-validator setups.

use super::Custody;
use anyhow::{Context, Result};
use async_trait::async_trait;
use k256::ecdsa::signature::hazmat::PrehashSigner as _;

/// Secp256k1 local custody using `k256`.
pub struct LocalSecp256k1 {
    sk: k256::ecdsa::SigningKey,
    pk_bytes: Vec<u8>,
}

impl LocalSecp256k1 {
    /// Create from a 32-byte hex-encoded private key.
    pub fn from_hex(sk_hex: &str) -> Result<Self> {
        let bytes = hex::decode(sk_hex.trim()).context("invalid hex for secp256k1 key")?;
        let sk = k256::ecdsa::SigningKey::from_slice(bytes.as_slice())
            .context("invalid secp256k1 private key")?;
        let pk_bytes = sk.verifying_key().to_sec1_bytes().to_vec();
        Ok(Self { sk, pk_bytes })
    }

    /// Generate a random key (useful for testing).
    pub fn generate() -> Self {
        use rand_core::RngCore;
        let mut raw = [0u8; 32];
        let sk = loop {
            rand_core::OsRng.fill_bytes(&mut raw);
            if let Ok(sk) = k256::ecdsa::SigningKey::from_slice(&raw) {
                break sk;
            }
        };
        let pk_bytes = sk.verifying_key().to_sec1_bytes().to_vec();
        Self { sk, pk_bytes }
    }
}

#[async_trait]
impl Custody for LocalSecp256k1 {
    async fn sign(&self, msg: &[u8]) -> Result<Vec<u8>> {
        use k256::ecdsa::Signature;
        let sig: Signature = self.sk.sign_prehash(msg)?;
        Ok(sig.to_bytes().to_vec())
    }

    fn public_key(&self) -> &[u8] {
        &self.pk_bytes
    }

    fn label(&self) -> &str {
        "local-secp256k1"
    }
}

/// Ed25519 local custody. The workspace dalek is 1.x, which has no `SigningKey`.
#[cfg(feature = "ed25519-v2")]
pub struct LocalEd25519 {
    sk: ed25519_dalek::SigningKey,
    pk_bytes: Vec<u8>,
}

#[cfg(feature = "ed25519-v2")]
impl LocalEd25519 {
    /// Create from a 32-byte hex-encoded seed.
    pub fn from_hex(seed_hex: &str) -> Result<Self> {
        let bytes = hex::decode(seed_hex.trim()).context("invalid hex for ed25519 seed")?;
        let seed: [u8; 32] = bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("ed25519 seed must be 32 bytes"))?;
        let sk = ed25519_dalek::SigningKey::from_bytes(&seed);
        let pk_bytes = sk.verifying_key().to_bytes().to_vec();
        Ok(Self { sk, pk_bytes })
    }

    /// Generate a random key.
    pub fn generate() -> Self {
        use rand_core::OsRng;
        let sk = ed25519_dalek::SigningKey::generate(&mut OsRng);
        let pk_bytes = sk.verifying_key().to_bytes().to_vec();
        Self { sk, pk_bytes }
    }
}

#[cfg(feature = "ed25519-v2")]
#[async_trait]
impl Custody for LocalEd25519 {
    async fn sign(&self, msg: &[u8]) -> Result<Vec<u8>> {
        use ed25519_dalek::Signer;
        let sig = self.sk.sign(msg);
        Ok(sig.to_bytes().to_vec())
    }

    fn public_key(&self) -> &[u8] {
        &self.pk_bytes
    }

    fn label(&self) -> &str {
        "local-ed25519"
    }
}
