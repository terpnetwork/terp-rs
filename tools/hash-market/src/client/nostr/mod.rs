//! HashMerchant Nostr types: kind:30070 root events, NIP-87 discovery, NIP-77 negentropy,
//! dao-calendar NIP-52 chain→relay egress, and marketplace NIP-15 egress + ingress-intent.
//!
//! All types in this module are defined in terms of the existing `msg::HashRoot`
//! and `cw721-nips` Nostr event primitives.

pub mod calendar_egress;
pub mod discovery;
pub mod marketplace_egress;
pub mod marketplace_ingress;
pub mod negentropy;
pub mod roots;

pub use calendar_egress::{
    chain_attrs_to_nostr, metadata_to_nip52_event, offchain_to_nip52, onchain_to_nip52,
    parse_calendar_action, CalendarChainAction, CalendarMetaView, Nip01Event,
};
pub use marketplace_egress::{
    chain_action_to_nostr, parse_marketplace_action, product_to_nip15,
    purchase_confirmation_to_nostr, MarketplaceChainAction, ProductMetaView, KIND_SET_PRODUCT,
    KIND_SET_STALL,
};
pub use marketplace_ingress::{
    authorize_auto_execute, parse_purchase_intent, parse_purchase_intent_value, IngressError,
    IngressExecutePolicy, PurchaseIntent,
};

use crate::msg::HashRoot;

/// Re-export HashRoot as HashMerchantPayload for use in the client trait.
pub type HashMerchantPayload = HashRoot;

/// The Nostr event kind for HashMerchant root attestations.
pub const HASHMERCHANT_ROOT_KIND: u16 = 30070;

/// Event kind for NIP-87-style hashmerchant relay discovery.
pub const HASHMERCHANT_DISCOVERY_KIND: u16 = 38172;

/// Kind enum for type-safe usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HashMerchantKind {
    Root,
    Discovery,
}

impl HashMerchantKind {
    pub fn kind_value(&self) -> u16 {
        match self {
            HashMerchantKind::Root => HASHMERCHANT_ROOT_KIND,
            HashMerchantKind::Discovery => HASHMERCHANT_DISCOVERY_KIND,
        }
    }

    pub fn from_kind(v: u16) -> Option<Self> {
        match v {
            HASHMERCHANT_ROOT_KIND => Some(HashMerchantKind::Root),
            HASHMERCHANT_DISCOVERY_KIND => Some(HashMerchantKind::Discovery),
            _ => None,
        }
    }
}

/// Info about a discovered hashmerchant relay (from NIP-87 query).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HashMerchantRelayInfo {
    /// Nostr relay URL (wss://) for the hashmerchant relay.
    pub relay_url: String,
    /// Chain UIDs this relay supports.
    pub chains: Vec<String>,
    /// Hash algorithms supported.
    pub algos: Vec<String>,
    /// Websocket URL for root event sync.
    pub ws_url: Option<String>,
    /// Webhook URL for root confirmations.
    pub webhook_url: Option<String>,
    /// BUD CID for relay metadata.
    pub bud_cid: Option<String>,
    /// Pubkey of the relay operator.
    pub pubkey: String,
}

/// Negentropy sync filter for NIP-77 reconciliation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NegentropyFilter {
    /// Event kinds to sync.
    pub kinds: Vec<u16>,
    /// Chain UIDs to filter by.
    pub chains: Vec<String>,
    /// Optional time range (since unix timestamp).
    pub since: Option<u64>,
    /// Optional time range (until unix timestamp).
    pub until: Option<u64>,
}

impl NegentropyFilter {
    pub fn for_chain(chain_uid: &str) -> Self {
        Self {
            kinds: vec![HASHMERCHANT_ROOT_KIND],
            chains: vec![chain_uid.to_string()],
            since: None,
            until: None,
        }
    }
}

/// Error type for hashmerchant client operations.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("WebSocket error: {0}")]
    Ws(String),
    #[error("Nostr relay error: {0}")]
    Nostr(String),
    #[error("IPFS RPC error: {0}")]
    Ipfs(String),
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("Negentropy error: {0}")]
    Negentropy(String),
    #[error("{0}")]
    Other(String),
}

impl From<anyhow::Error> for ClientError {
    fn from(e: anyhow::Error) -> Self {
        ClientError::Other(e.to_string())
    }
}

/// Result type for hashmerchant client operations.
pub type ClientResult<T> = Result<T, ClientError>;
