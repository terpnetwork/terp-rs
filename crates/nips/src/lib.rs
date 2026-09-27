//! # cw721-nips — Nostr NIP library with cw721-compatible metadata extensions
//!
//! Core library for working with Nostr event types (NIP-01) and higher-level
//! NIP specifications (NIP-15 marketplace, NIP-52 calendar events) on-chain.
//!
//! ## NIP-01 Primitives
//!
//! | NIP-01 Field   | Rust Type         | Description                                   |
//! |----------------|-------------------|-----------------------------------------------|
//! | `id`           | `String`          | 32-byte hex SHA-256 of the canonical event    |
//! | `pubkey`       | `String`          | 32-byte hex secp256k1 public key              |
//! | `created_at`   | `u64`             | Unix timestamp (seconds)                      |
//! | `kind`         | `u16`             | Event kind (0–65535)                          |
//! | `tags`         | `Vec<Vec<String>>`| Array of string arrays                        |
//! | `content`      | `String`          | Arbitrary string content                      |
//! | `sig`          | `String`          | 64-byte hex Schnorr signature                 |
//!
//! The canonical event ID is computed as:
//! ```ignore
//! SHA-256(JSON.stringify([0, pubkey, created_at, kind, tags, content]))
//! ```
//! See [`RawNostrEvent::compute_id`] and [`event::build_event`].
//!
//! ## Metadata Extensions
//!
//! The [`NostrExt`] trait and [`NostrExtension`] enum provide a unified
//! interface for on-chain vs off-chain Nostr metadata — see [`metadata`].

pub mod encoding;
pub mod error;
pub mod event;
pub mod metadata;
pub mod nips;
pub mod buds;
/// Cashu NUT sketches — off by default (`--features nuts`). NUT-02 stays a stub.
#[cfg(feature = "nuts")]
pub mod nuts;
pub mod tags;
pub mod types;
/// DAO text-record / storage registration for mesh + NIP heuristics.
pub mod dao_record;

#[cfg(feature = "cw")]
pub mod cw;

// Re-exports
pub use error::{NipError, NipResult};
pub use event::*;
pub use metadata::{EmptyNostrExtension, NostrExt, NostrExtension};
use schemars::JsonSchema;
use sha2::Digest as _;
pub use tags::TagBuilder;
pub use types::*;
pub use dao_record::{
    place_bid_wasm_hint, purchase_wasm_hint, nip15_filter_heuristic, DaoRecordAction,
    DaoRecordWasmHint, MeshDescriptorV1, MeshMarketplacePatch, Nip15PlaneMode, NipDaoPlane,
    NipNamespacedRecord, NostrFilterHeuristic, WasmExecuteHint, DAO_MESH_RECORD_KEY,
};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::nips::nip73::Nip73Ext as _;

// ==================== Core Traits ====================

/// Marker trait for NIP event kinds
pub trait NipKind:
    Copy + Clone + Eq + std::hash::Hash + std::fmt::Debug + Send + Sync + 'static
{
    fn kind_value(&self) -> u16;

    fn is_regular(&self) -> bool {
        let v = self.kind_value();
        (1000..10000).contains(&v) || (4..45).contains(&v) || v == 1 || v == 2
    }

    fn is_replaceable(&self) -> bool {
        let v = self.kind_value();
        (10000..20000).contains(&v) || v == 0 || v == 3
    }

    fn is_ephemeral(&self) -> bool {
        let v = self.kind_value();
        (20000..30000).contains(&v)
    }

    fn is_addressable(&self) -> bool {
        let v = self.kind_value();
        (30000..40000).contains(&v)
    }
}

/// Core trait for NIP metadata types
///
/// This trait defines the interface that all NIP-specific metadata types
/// must implement. It provides:
/// - Type-safe kind association
/// - Validation according to NIP-specific rules
/// - Conversion to/from event tags and content
/// - Parsing from raw Nostr events
pub trait NipMetadata: Serialize + DeserializeOwned + Clone + Send + Sync + 'static {
    /// The kind type for this NIP
    type Kind: NipKind;

    /// Get the kind for this metadata instance
    fn kind(&self) -> Self::Kind;

    /// Validate metadata according to NIP-specific rules
    fn validate(&self) -> NipResult<()>;

    /// Convert metadata to event tags
    fn to_tags(&self) -> Vec<Tag>;

    /// Convert metadata to event content
    fn content(&self) -> String;

    /// Parse metadata from a raw Nostr event
    fn from_raw_event(event: &RawNostrEvent) -> NipResult<Self>;

    /// Get the d-tag identifier for addressable events
    fn d_tag(&self) -> Option<String> {
        None
    }
}

/// A raw Nostr event as received from relays
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct RawNostrEvent {
    pub id: String,
    pub pubkey: String,
    pub created_at: u64,
    pub kind: u16,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub sig: String,
}

impl RawNostrEvent {
    /// Compute the event ID according to NIP-01
    pub fn compute_id(&self) -> NipResult<String> {
        let serialized = Self::serialize_for_id(
            &self.pubkey,
            self.created_at,
            self.kind,
            &self.tags,
            &self.content,
        );
        let hash = sha2::Sha256::digest(serialized.as_bytes());
        Ok(hex::encode(hash))
    }

    /// Serialize event data for ID computation
    pub fn serialize_for_id(
        pubkey: &str,
        created_at: u64,
        kind: u16,
        tags: &[Vec<String>],
        content: &str,
    ) -> String {
        let serialized = serde_json::json!([0, pubkey, created_at, kind, tags, content]);
        serialized.to_string()
    }

    /// Verify the event signature (requires cryptographic verification)
    pub fn verify_signature(&self) -> NipResult<bool> {
        // This would require schnorr signature verification
        // Implementation depends on crypto library choice
        Err(NipError::Crypto(
            "Signature verification not implemented".into(),
        ))
    }

    /// Parse tags by name
    pub fn tags_by_name(&self, name: &str) -> Vec<&Vec<String>> {
        self.tags
            .iter()
            .filter(|tag| tag.first().map(|n| n.as_str()) == Some(name))
            .collect()
    }

    /// Get the d-tag value for addressable events
    pub fn d_tag(&self) -> Option<&str> {
        self.tags
            .iter()
            .find(|tag| tag.first().map(|n| n.as_str()) == Some("d"))
            .and_then(|tag| tag.get(1))
            .map(|s| s.as_str())
    }
}

/// Builder for constructing Nostr events
pub struct NostrEventBuilder;

impl NostrEventBuilder {
    /// Build a raw Nostr event from metadata.
    /// NOTE: DOES NOT SIGN EVENT, ONLY BUILDS.
    pub fn build<M: NipMetadata>(
        metadata: &M,
        pubkey: &PubKey,
        created_at: UnixTimestamp,
    ) -> NipResult<RawNostrEvent> {
        metadata.validate()?;

        let mut tags = metadata.to_tags();

        // NIP-73: Add external content ID tags
        tags = metadata.with_external_ids_tags(tags);

        // Add d-tag for addressable events
        if let Some(d_tag) = metadata.d_tag() {
            tags.push(Tag::d(d_tag));
        }

        let kind = metadata.kind().kind_value();
        let content = metadata.content();

        let serialized = RawNostrEvent::serialize_for_id(
            pubkey.as_str(),
            created_at,
            kind,
            &tags
                .iter()
                .map(|t| t.clone().into_inner())
                .collect::<Vec<_>>(),
            &content,
        );

        let hash = sha2::Sha256::digest(serialized.as_bytes());
        let id = hex::encode(hash);

        Ok(RawNostrEvent {
            id,
            pubkey: pubkey.as_str().to_string(),
            created_at,
            kind,
            tags: tags.into_iter().map(|t| t.into_inner()).collect(),
            content,
            sig: String::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    // ==================== Test Kind Enum ====================

    /// A test kind enum that can represent any NIP kind value for testing
    /// the NipKind trait methods across all kind ranges.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum TestKind {
        Raw(u16),
    }

    impl NipKind for TestKind {
        fn kind_value(&self) -> u16 {
            match self {
                TestKind::Raw(v) => *v,
            }
        }
    }

    // ==================== Test Metadata Type ====================

    /// A simple metadata type for testing NostrEventBuilder::build
    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestMetadata {
        name: String,
        identifier: String,
    }

    impl NipMetadata for TestMetadata {
        type Kind = TestKind;

        fn kind(&self) -> Self::Kind {
            TestKind::Raw(0) // kind 0 (metadata / replaceable)
        }

        fn validate(&self) -> NipResult<()> {
            if self.name.is_empty() {
                return Err(NipError::Validation("name cannot be empty".into()));
            }
            Ok(())
        }

        fn to_tags(&self) -> Vec<Tag> {
            vec![Tag::t("test")]
        }

        fn content(&self) -> String {
            serde_json::to_string(self).unwrap_or_default()
        }

        fn d_tag(&self) -> Option<String> {
            Some(self.identifier.clone())
        }

        fn from_raw_event(event: &RawNostrEvent) -> NipResult<Self> {
            serde_json::from_str(&event.content).map_err(NipError::Serialization)
        }
    }

    // ==================== NipKind Tests ====================

    #[test]
    fn test_nipkind_is_regular() {
        // Regular kinds: 1000..10000, 4..45, 1, 2
        // Should return true
        assert!(TestKind::Raw(1).is_regular(), "kind 1 should be regular");
        assert!(TestKind::Raw(2).is_regular(), "kind 2 should be regular");
        assert!(TestKind::Raw(4).is_regular(), "kind 4 should be regular");
        assert!(TestKind::Raw(44).is_regular(), "kind 44 should be regular");
        assert!(
            TestKind::Raw(1000).is_regular(),
            "kind 1000 should be regular"
        );
        assert!(
            TestKind::Raw(5000).is_regular(),
            "kind 5000 should be regular"
        );
        assert!(
            TestKind::Raw(9999).is_regular(),
            "kind 9999 should be regular"
        );
        assert!(
            TestKind::Raw(1021).is_regular(),
            "kind 1021 (Bid) should be regular"
        );
        assert!(
            TestKind::Raw(1022).is_regular(),
            "kind 1022 (BidConfirmation) should be regular"
        );

        // Should return false
        assert!(
            !TestKind::Raw(0).is_regular(),
            "kind 0 should NOT be regular"
        );
        assert!(
            !TestKind::Raw(3).is_regular(),
            "kind 3 should NOT be regular"
        );
        assert!(
            !TestKind::Raw(45).is_regular(),
            "kind 45 should NOT be regular"
        );
        assert!(
            !TestKind::Raw(100).is_regular(),
            "kind 100 should NOT be regular"
        );
        assert!(
            !TestKind::Raw(10000).is_regular(),
            "kind 10000 should NOT be regular"
        );
        assert!(
            !TestKind::Raw(20000).is_regular(),
            "kind 20000 should NOT be regular"
        );
        assert!(
            !TestKind::Raw(30000).is_regular(),
            "kind 30000 should NOT be regular"
        );
    }

    #[test]
    fn test_nipkind_is_replaceable() {
        // Replaceable kinds: 10000..20000, 0, 3
        assert!(
            TestKind::Raw(0).is_replaceable(),
            "kind 0 should be replaceable"
        );
        assert!(
            TestKind::Raw(3).is_replaceable(),
            "kind 3 should be replaceable"
        );
        assert!(
            TestKind::Raw(10000).is_replaceable(),
            "kind 10000 should be replaceable"
        );
        assert!(
            TestKind::Raw(15000).is_replaceable(),
            "kind 15000 should be replaceable"
        );
        assert!(
            TestKind::Raw(19999).is_replaceable(),
            "kind 19999 should be replaceable"
        );

        assert!(
            !TestKind::Raw(1).is_replaceable(),
            "kind 1 should NOT be replaceable"
        );
        assert!(
            !TestKind::Raw(4).is_replaceable(),
            "kind 4 should NOT be replaceable"
        );
        assert!(
            !TestKind::Raw(9999).is_replaceable(),
            "kind 9999 should NOT be replaceable"
        );
        assert!(
            !TestKind::Raw(20000).is_replaceable(),
            "kind 20000 should NOT be replaceable"
        );
        assert!(
            !TestKind::Raw(30000).is_replaceable(),
            "kind 30000 should NOT be replaceable"
        );
    }

    #[test]
    fn test_nipkind_is_ephemeral() {
        // Ephemeral kinds: 20000..30000
        assert!(
            TestKind::Raw(20000).is_ephemeral(),
            "kind 20000 should be ephemeral"
        );
        assert!(
            TestKind::Raw(25000).is_ephemeral(),
            "kind 25000 should be ephemeral"
        );
        assert!(
            TestKind::Raw(29999).is_ephemeral(),
            "kind 29999 should be ephemeral"
        );

        assert!(
            !TestKind::Raw(0).is_ephemeral(),
            "kind 0 should NOT be ephemeral"
        );
        assert!(
            !TestKind::Raw(19999).is_ephemeral(),
            "kind 19999 should NOT be ephemeral"
        );
        assert!(
            !TestKind::Raw(30000).is_ephemeral(),
            "kind 30000 should NOT be ephemeral"
        );
    }

    #[test]
    fn test_nipkind_is_addressable() {
        // Addressable kinds: 30000..40000
        assert!(
            TestKind::Raw(30000).is_addressable(),
            "kind 30000 should be addressable"
        );
        assert!(
            TestKind::Raw(30017).is_addressable(),
            "kind 30017 (SetStall) should be addressable"
        );
        assert!(
            TestKind::Raw(30018).is_addressable(),
            "kind 30018 (SetProduct) should be addressable"
        );
        assert!(
            TestKind::Raw(31922).is_addressable(),
            "kind 31922 (DateEvent) should be addressable"
        );
        assert!(
            TestKind::Raw(31923).is_addressable(),
            "kind 31923 (TimeEvent) should be addressable"
        );
        assert!(
            TestKind::Raw(39999).is_addressable(),
            "kind 39999 should be addressable"
        );

        assert!(
            !TestKind::Raw(0).is_addressable(),
            "kind 0 should NOT be addressable"
        );
        assert!(
            !TestKind::Raw(29999).is_addressable(),
            "kind 29999 should NOT be addressable"
        );
        assert!(
            !TestKind::Raw(40000).is_addressable(),
            "kind 40000 should NOT be addressable"
        );
    }

    #[test]
    fn test_nipkind_kind_value() {
        assert_eq!(TestKind::Raw(0).kind_value(), 0);
        assert_eq!(TestKind::Raw(1).kind_value(), 1);
        assert_eq!(TestKind::Raw(65535).kind_value(), 65535);
    }

    #[test]
    fn prelude_kind_surface_matches_nip_modules() {
        use crate::nips::nip01::Nip01Kind;
        use crate::nips::nip15::Nip15Kind;
        use crate::nips::nip52::Nip52Kind;

        assert!(Nip01Kind::TextNote.is_regular());
        assert!(Nip01Kind::Metadata.is_replaceable());
        assert!(Nip15Kind::Bid.is_regular());
        assert!(Nip15Kind::SetProduct.is_addressable());
        assert!(Nip52Kind::DateEvent.is_addressable());
        assert_eq!(Nip15Kind::SetStall.kind_value(), 30017);
        assert_eq!(Nip52Kind::Calendar.kind_value(), 31924);
    }

    // ==================== RawNostrEvent Tests ====================

    const TEST_PUBKEY: &str = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459f";

    fn make_test_event(tags: Vec<Vec<String>>) -> RawNostrEvent {
        RawNostrEvent {
            id: String::new(),
            pubkey: TEST_PUBKEY.to_string(),
            created_at: 1700000000,
            kind: 1,
            tags,
            content: "hello".to_string(),
            sig: String::new(),
        }
    }

    #[test]
    fn test_tags_by_name_found() {
        let tags = vec![
            vec!["e".to_string(), "event1".to_string()],
            vec!["p".to_string(), "pubkey1".to_string()],
            vec!["e".to_string(), "event2".to_string()],
            vec!["t".to_string(), "hashtag".to_string()],
        ];
        let event = make_test_event(tags);

        let e_tags = event.tags_by_name("e");
        assert_eq!(e_tags.len(), 2);
        assert_eq!(e_tags[0][1], "event1");
        assert_eq!(e_tags[1][1], "event2");

        let p_tags = event.tags_by_name("p");
        assert_eq!(p_tags.len(), 1);
        assert_eq!(p_tags[0][1], "pubkey1");

        let t_tags = event.tags_by_name("t");
        assert_eq!(t_tags.len(), 1);
        assert_eq!(t_tags[0][1], "hashtag");
    }

    #[test]
    fn test_tags_by_name_not_found() {
        let tags = vec![
            vec!["e".to_string(), "event1".to_string()],
            vec!["p".to_string(), "pubkey1".to_string()],
        ];
        let event = make_test_event(tags);

        let result = event.tags_by_name("x");
        assert!(
            result.is_empty(),
            "should return empty for non-existing tag name"
        );

        let result = event.tags_by_name("d");
        assert!(
            result.is_empty(),
            "should return empty when no d-tag exists"
        );

        let result = event.tags_by_name("t");
        assert!(
            result.is_empty(),
            "should return empty when no t-tag exists"
        );
    }

    #[test]
    fn test_d_tag_found() {
        let tags = vec![
            vec!["d".to_string(), "my-identifier".to_string()],
            vec!["e".to_string(), "event1".to_string()],
        ];
        let event = make_test_event(tags);

        assert_eq!(event.d_tag(), Some("my-identifier"));
    }

    #[test]
    fn test_d_tag_not_found() {
        let tags = vec![
            vec!["e".to_string(), "event1".to_string()],
            vec!["p".to_string(), "pubkey1".to_string()],
        ];
        let event = make_test_event(tags);

        assert_eq!(
            event.d_tag(),
            None,
            "should return None when no d-tag exists"
        );
    }

    #[test]
    fn test_d_tag_empty_tags() {
        let event = make_test_event(vec![]);
        assert_eq!(event.d_tag(), None, "should return None for empty tags");
    }

    #[test]
    fn test_verify_signature_returns_error() {
        let event = make_test_event(vec![]);
        let result = event.verify_signature();
        assert!(result.is_err(), "verify_signature should return an error");
        match result {
            Err(NipError::Crypto(msg)) => {
                assert!(
                    msg.contains("not implemented"),
                    "error should mention not implemented"
                );
            }
            _ => panic!("Expected Crypto error, got {:?}", result),
        }
    }

    #[test]
    fn test_compute_id_deterministic() {
        let tags = vec![vec!["p".to_string(), "abc123".to_string()]];
        let event = make_test_event(tags);

        let id1 = event.compute_id().expect("compute_id should succeed");
        let id2 = event.compute_id().expect("compute_id should succeed");

        assert_eq!(id1, id2, "event ID should be deterministic");
        assert_eq!(id1.len(), 64, "SHA-256 hex should be 64 chars");
        assert!(
            id1.chars().all(|c| c.is_ascii_hexdigit()),
            "event ID should be hex-encoded"
        );
    }

    #[test]
    fn test_compute_id_different_content() {
        let event_a = RawNostrEvent {
            id: String::new(),
            pubkey: TEST_PUBKEY.to_string(),
            created_at: 1700000000,
            kind: 1,
            tags: vec![],
            content: "hello".to_string(),
            sig: String::new(),
        };
        let event_b = RawNostrEvent {
            content: "world".to_string(),
            ..event_a.clone()
        };

        let id_a = event_a.compute_id().expect("compute_id should succeed");
        let id_b = event_b.compute_id().expect("compute_id should succeed");

        assert_ne!(id_a, id_b, "different content should produce different IDs");
    }

    #[test]
    fn test_serialize_for_id_format() {
        let serialized = RawNostrEvent::serialize_for_id(
            TEST_PUBKEY,
            1700000000,
            1,
            &[vec!["p".to_string(), "abc".to_string()]],
            "hello",
        );
        // The serialized format should be a JSON array matching:
        // [0, pubkey, created_at, kind, tags, content]
        assert!(serialized.starts_with('['), "should be a JSON array");
        assert!(
            serialized.contains("0,"),
            "should start with protocol version 0"
        );
        assert!(serialized.contains(TEST_PUBKEY), "should contain pubkey");
        assert!(
            serialized.contains("1700000000"),
            "should contain created_at"
        );
        assert!(serialized.contains("\"p\""), "should contain tag name");
    }

    // ==================== NostrEventBuilder Tests ====================

    #[test]
    fn test_build_with_valid_metadata_and_d_tag() {
        let metadata = TestMetadata {
            name: "test-user".to_string(),
            identifier: "my-d-identifier".to_string(),
        };

        let pubkey = PubKey::new(TEST_PUBKEY).expect("valid pubkey");
        let created_at: UnixTimestamp = 1700000000;

        let event =
            NostrEventBuilder::build(&metadata, &pubkey, created_at).expect("build should succeed");

        // Verify event fields
        assert_eq!(event.pubkey, TEST_PUBKEY);
        assert_eq!(event.created_at, 1700000000);
        assert_eq!(event.kind, 0); // TestKind::Raw(0) -> kind_value() returns 0
        assert_eq!(event.content, serde_json::to_string(&metadata).unwrap());

        // Verify the d-tag was added from metadata.d_tag()
        assert!(
            event
                .tags
                .iter()
                .any(|t| t.first().map(|s| s.as_str()) == Some("d")
                    && t.get(1).map(|s| s.as_str()) == Some("my-d-identifier")),
            "event should contain the d-tag from metadata"
        );

        // Verify the custom tag from metadata.to_tags() was included
        assert!(
            event
                .tags
                .iter()
                .any(|t| t.first().map(|s| s.as_str()) == Some("t")
                    && t.get(1).map(|s| s.as_str()) == Some("test")),
            "event should contain the custom tag from metadata.to_tags()"
        );

        // Verify the event ID was computed (non-empty, 64 hex chars)
        assert!(!event.id.is_empty(), "event ID should not be empty");
        assert_eq!(event.id.len(), 64, "event ID should be 64 hex chars");
        assert!(
            event.id.chars().all(|c| c.is_ascii_hexdigit()),
            "event ID should be hex-encoded"
        );

        // Verify sig is empty (must be signed separately)
        assert!(event.sig.is_empty(), "sig should be empty after build");
    }

    #[test]
    fn test_build_with_metadata_validation_failure() {
        let metadata = TestMetadata {
            name: "".to_string(), // empty name causes validation failure
            identifier: "id".to_string(),
        };

        let pubkey = PubKey::new(TEST_PUBKEY).expect("valid pubkey");

        let result = NostrEventBuilder::build(&metadata, &pubkey, 1700000000);
        assert!(result.is_err(), "build should fail when validation fails");
    }
}
