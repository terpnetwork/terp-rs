use crate::client::nostr::{
    ClientResult, HashMerchantPayload, HashMerchantRelayInfo, NegentropyFilter,
};

/// A confirmed foreign-chain Merkle root that clients react to.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RootConfirmation {
    /// The confirmed Merkle root data.
    pub payload: HashMerchantPayload,
    /// Optional Nostr event ID for this root.
    pub event_id: Option<String>,
    /// Optional IPFS CID for associated proof data.
    pub cid: Option<String>,
    /// Relay URL this root was received from.
    pub source: String,
    /// Unix timestamp when this confirmation was received.
    pub received_at: u64,
}

/// Uniform interface for consuming hashmerchant root attestations.
///
/// Implementations handle one or more transport modes:
/// - Nostr relay (NIP-77 negentropy sync)
/// - Local Kubo RPC (IPFS CID operations)
/// - HTTP webhook forwarding
#[async_trait::async_trait]
pub trait HashMerchantClient: Send + Sync {
    /// Called when a new Merkle root is confirmed.
    ///
    /// This is the primary callback — called by the relay, the webhook handler,
    /// or any other transport that delivers a confirmed root.
    async fn on_root_confirmed(&self, confirmation: RootConfirmation) -> ClientResult<()>;

    /// Subscribe to roots via negentropy reconciliation.
    ///
    /// Connects to the relay, performs NIP-77 sync, fetches new root events,
    /// and calls `on_root_confirmed` for each new root.
    async fn negentropy_sync(
        &self,
        relay: &str,
        filter: &NegentropyFilter,
    ) -> ClientResult<Vec<RootConfirmation>>;

    /// Discover hashmerchant relays via NIP-87 event query.
    async fn discover_relays(&self, chain_uid: &str) -> ClientResult<Vec<HashMerchantRelayInfo>>;

    /// Fetch a CID from IPFS via the local Kubo RPC (BUD protocol).
    async fn fetch_cid(&self, cid: &str) -> ClientResult<Vec<u8>>;

    /// Pin a CID on the local IPFS node.
    async fn pin_cid(&self, cid: &str) -> ClientResult<()>;
}
