//! Nostr metadata extension traits and types for NFT/collection metadata.
//!
//! Provides a unified interface for Nostr metadata that can be stored
//! either on-chain (full event data) or off-chain (CID pointer to IPFS).
//!
//! # Two scenarios
//!
//! - **OnChain**: Full `RawNostrEvent` stored directly in contract state.
//! - **OffChain**: A minimal on-chain pointer (CID + kind) pointing to the
//!   full Nostr event JSON on IPFS or another content-addressed store.
//!
//! The [`NostrExt`] trait is what CosmWasm contracts bind against in their
//! generic type parameters, instead of depending directly on cosmwasm_std.

use crate::RawNostrEvent;
use crate::error::NipResult;

/// Interface for Nostr metadata in NFT/collection extensions.
///
/// Generic code binds against this trait instead of cosmwasm_std types.
pub trait NostrExt:
    serde::Serialize + serde::de::DeserializeOwned + Clone + std::fmt::Debug + 'static
{
    /// Whether the full Nostr event data lives on-chain or off-chain.
    fn location(&self) -> bool;

    /// The Nostr event kind (e.g. `31922` for a date-based calendar event).
    fn kind(&self) -> u16;

    /// Optional d-tag identifier for addressable events (kind 30000–39999).
    fn d_tag(&self) -> Option<&str>;

    /// Promote this extension into a full NIP-01 `RawNostrEvent` for relay submission.
    /// Caller provides the event author pubkey and creation timestamp.
    fn into_event(self, pubkey: &str, created_at: u64) -> NipResult<RawNostrEvent>;
}

/// Single unified metadata extension type for both on-chain and off-chain Nostr data.
///
/// JSON-serialized with a `"location"` tag so consumers can read the variant
/// without prior knowledge:
///
/// ```json
/// {"location": "on_chain", "id": "...", "pubkey": "...", ...}
/// {"location": "off_chain", "cid": "Qm...", "kind": 31922, ...}
/// ```
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(tag = "location", rename_all = "snake_case")]
pub enum NostrExtension {
    /// Full Nostr event data stored on-chain.
    OnChain(RawNostrEvent),
    /// Minimal on-chain pointer; the full Nostr event JSON lives at the
    /// given CID. The `kind` field is stored on-chain for efficient filtering.
    OffChain {
        /// IPFS CID pointing to the Nostr event JSON.
        cid: String,
        /// Nostr event kind (queryable on-chain).
        kind: u16,
        /// Optional IPFS gateway URL for content resolution.
        #[serde(skip_serializing_if = "Option::is_none")]
        gateway: Option<String>,
    },
}

// ── NostrExt impl for RawNostrEvent (always on-chain) ──

impl NostrExt for RawNostrEvent {
    fn location(&self) -> bool {
        true
    }

    fn kind(&self) -> u16 {
        self.kind
    }

    fn d_tag(&self) -> Option<&str> {
        self.tags
            .iter()
            .find(|t| t.first().map(|n| n.as_str()) == Some("d"))
            .and_then(|t| t.get(1))
            .map(|s| s.as_str())
    }

    fn into_event(self, _pubkey: &str, _created_at: u64) -> NipResult<RawNostrEvent> {
        Ok(self)
    }
}

// ── NostrExt impl for the unified enum ──

impl NostrExt for NostrExtension {
    fn location(&self) -> bool {
        match self {
            NostrExtension::OnChain(..) => true,
            NostrExtension::OffChain { .. } => false,
        }
    }

    fn kind(&self) -> u16 {
        match self {
            NostrExtension::OnChain(e) => e.kind,
            NostrExtension::OffChain { kind, .. } => *kind,
        }
    }

    fn d_tag(&self) -> Option<&str> {
        match self {
            NostrExtension::OnChain(e) => e.d_tag(),
            NostrExtension::OffChain { .. } => None,
        }
    }

    fn into_event(self, pubkey: &str, created_at: u64) -> NipResult<RawNostrEvent> {
        match self {
            NostrExtension::OnChain(e) => e.into_event(pubkey, created_at),
            NostrExtension::OffChain { cid, kind, gateway } => {
                let tags = if let Some(gw) = gateway {
                    vec![vec!["gateway".to_string(), gw]]
                } else {
                    vec![]
                };
                let mut event = RawNostrEvent {
                    id: String::new(),
                    pubkey: pubkey.to_string(),
                    created_at,
                    kind,
                    tags,
                    content: cid,
                    sig: String::new(),
                };
                event.id = event.compute_id()?;
                Ok(event)
            }
        }
    }
}

// ── Empty extension (no Nostr data) ──

/// Placeholder type for contracts with no Nostr metadata.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct EmptyNostrExtension;

impl NostrExt for EmptyNostrExtension {
    fn location(&self) -> bool {
        true
    }

    fn kind(&self) -> u16 {
        0
    }

    fn d_tag(&self) -> Option<&str> {
        None
    }

    fn into_event(self, _pubkey: &str, _created_at: u64) -> NipResult<RawNostrEvent> {
        Err(crate::error::NipError::MissingField(
            "No Nostr metadata available".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RawNostrEvent;

    #[test]
    fn test_raw_nostr_event_is_always_on_chain() {
        let event = RawNostrEvent {
            id: String::new(),
            pubkey: "ab".repeat(32),
            created_at: 1000,
            kind: 1,
            tags: vec![],
            content: "hello".into(),
            sig: String::new(),
        };
        assert!(event.location());
        assert_eq!(event.kind(), 1);
        assert!(event.d_tag().is_none());
    }

    #[test]
    fn test_on_chain_variant() {
        let event = RawNostrEvent {
            id: "a".repeat(64),
            pubkey: "b".repeat(64),
            created_at: 1000,
            kind: 31922,
            tags: vec![vec!["d".into(), "my-event".into()]],
            content: "description".into(),
            sig: "c".repeat(128),
        };
        let ext = NostrExtension::OnChain(event.clone());
        assert!(ext.location());
        assert_eq!(ext.kind(), 31922);
        assert_eq!(ext.d_tag(), Some("my-event"));
    }

    #[test]
    fn test_off_chain_variant() {
        let ext = NostrExtension::OffChain {
            cid: "QmTest123".into(),
            kind: 31923,
            gateway: Some("https://ipfs.io/ipfs/".into()),
        };
        assert!(!ext.location());
        assert_eq!(ext.kind(), 31923);
        assert!(ext.d_tag().is_none());
    }

    #[test]
    fn test_empty_extension_errors() {
        let empty = EmptyNostrExtension;
        assert!(empty.into_event("pk", 0).is_err());
    }

    #[test]
    fn test_serde_tag_discrimination() {
        let onchain = NostrExtension::OnChain(RawNostrEvent {
            id: "id".into(),
            pubkey: "pk".into(),
            created_at: 0,
            kind: 1,
            tags: vec![],
            content: "".into(),
            sig: "sig".into(),
        });
        let json = serde_json::to_string(&onchain).unwrap();
        assert!(json.contains(r#""location":"on_chain""#));

        let offchain = NostrExtension::OffChain {
            cid: "QmX".into(),
            kind: 31922,
            gateway: None,
        };
        let json = serde_json::to_string(&offchain).unwrap();
        assert!(json.contains(r#""location":"off_chain""#));
    }

    #[test]
    fn test_raw_nostr_event_with_d_tag() {
        let event = RawNostrEvent {
            id: "a".repeat(64),
            pubkey: "b".repeat(64),
            created_at: 1000,
            kind: 30000,
            tags: vec![vec!["d".into(), "my-unique-id".into()]],
            content: "test".into(),
            sig: "c".repeat(128),
        };
        assert_eq!(event.d_tag(), Some("my-unique-id"));
    }

    #[test]
    fn test_on_chain_into_event() {
        let event = RawNostrEvent {
            id: "".into(),
            pubkey: "pk".into(),
            created_at: 100,
            kind: 1,
            tags: vec![],
            content: "hello".into(),
            sig: "".into(),
        };
        let ext = NostrExtension::OnChain(event);
        let result = ext.into_event("new-pk", 200).unwrap();
        assert_eq!(result.pubkey, "pk");
        assert_eq!(result.created_at, 100);
    }

    #[test]
    fn test_off_chain_with_gateway() {
        let ext = NostrExtension::OffChain {
            cid: "QmCID".into(),
            kind: 1,
            gateway: Some("https://gateway.example.com/".into()),
        };
        let event = ext
            .into_event(
                "pk1234pk1234pk1234pk1234pk1234pk1234pk1234pk1234pk1234pk12",
                500,
            )
            .unwrap();
        assert_eq!(event.content, "QmCID");
        assert_eq!(event.kind, 1);
        assert!(
            event
                .tags
                .iter()
                .any(|t| t.first() == Some(&"gateway".to_string()))
        );
    }

    #[test]
    fn test_empty_extension_full_coverage() {
        let empty = EmptyNostrExtension;
        assert!(empty.location());
        assert_eq!(empty.kind(), 0);
        assert!(empty.d_tag().is_none());
    }

    #[test]
    fn test_off_chain_into_event_computes_id() {
        let ext = NostrExtension::OffChain {
            cid: "QmCID".into(),
            kind: 1,
            gateway: None,
        };
        let event = ext
            .into_event(
                "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
                1000,
            )
            .unwrap();
        assert_eq!(
            event.pubkey,
            "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234"
        );
        assert_eq!(event.kind, 1);
        assert_eq!(event.content, "QmCID");
        assert_eq!(event.id.len(), 64);
    }
}
