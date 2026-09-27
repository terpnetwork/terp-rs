//! Consensus signing (K_cons / Ed25519). Not VE custody (K_op / secp256k1).

use async_trait::async_trait;

/// Signs CometBFT canonical vote/proposal bytes. Implementations must not
/// expose the Ed25519 scalar on any TCP/Noise path.
#[async_trait]
pub trait ConsensusSigner: Send + Sync {
    async fn sign_consensus(&self, msg: &[u8]) -> anyhow::Result<[u8; 64]>;

    /// Same as [`sign_consensus`]; `step` is FilePV inventory hint only.
    async fn sign_consensus_for(&self, msg: &[u8], _step: i8) -> anyhow::Result<[u8; 64]> {
        self.sign_consensus(msg).await
    }

    fn group_pubkey(&self) -> &[u8];
    fn label(&self) -> &str;
}
