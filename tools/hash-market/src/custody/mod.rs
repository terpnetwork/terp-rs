//! Pluggable signing backend for vote extensions.
//!
//! The `Custody` trait abstracts over key management so the server can use
//! a local key file during development or delegate to TKMS in production.

pub mod consensus;
#[cfg(feature = "frost")]
pub mod frost;
pub mod hrs;
pub mod local;
#[cfg(feature = "frost")]
pub mod presig;
#[cfg(feature = "frost")]
pub mod shard_peer;
pub mod tkms;

use async_trait::async_trait;

/// A signing backend that can produce secp256k1 or ed25519 signatures.
#[async_trait]
pub trait Custody: Send + Sync {
    /// Sign the provided message bytes, returning a compact signature.
    async fn sign(&self, msg: &[u8]) -> anyhow::Result<Vec<u8>>;

    /// Return the public key bytes (compressed SEC1 for secp256k1, 32 bytes for ed25519).
    fn public_key(&self) -> &[u8];

    /// Human-readable label for logging.
    fn label(&self) -> &str;
}

/// Blanket impl so `Box<dyn Custody>` itself satisfies `Custody`.
#[async_trait]
impl Custody for Box<dyn Custody> {
    async fn sign(&self, msg: &[u8]) -> anyhow::Result<Vec<u8>> {
        (**self).sign(msg).await
    }

    fn public_key(&self) -> &[u8] {
        (**self).public_key()
    }

    fn label(&self) -> &str {
        (**self).label()
    }
}
