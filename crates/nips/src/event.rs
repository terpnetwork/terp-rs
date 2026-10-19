//! NIP-01 event construction and canonical serialization.
//!
//! Per [NIP-01](https://github.com/nostr-protocol/nips/blob/master/01.md),
//! the event ID is computed as:
//!
//! ```text
//! EventID = SHA-256(JSON.stringify([0, pubkey, created_at, kind, tags, content]))
//! ```
//!
//! The leading `0` is the NIP-01 protocol version marker. Tags are serialized
//! as JSON arrays of strings. The resulting 32-byte hash is hex-encoded.

use crate::{RawNostrEvent, error::NipResult};
use sha2::Digest;

/// Build a NIP-01 event with the given fields and compute its canonical ID.
pub fn build_event(
    pubkey: &str,
    created_at: u64,
    kind: u16,
    tags: Vec<Vec<String>>,
    content: String,
) -> NipResult<RawNostrEvent> {
    let mut event = RawNostrEvent {
        id: String::new(),
        pubkey: pubkey.to_string(),
        created_at,
        kind,
        tags,
        content,
        sig: String::new(),
    };
    event.id = event.compute_id()?;
    Ok(event)
}

/// Serialize event fields for ID computation (NIP-01 canonical format).
///
/// Returns the JSON string that should be hashed with SHA-256.
pub fn serialize_for_id(
    pubkey: &str,
    created_at: u64,
    kind: u16,
    tags: &[Vec<String>],
    content: &str,
) -> String {
    RawNostrEvent::serialize_for_id(pubkey, created_at, kind, tags, content)
}

/// Compute the NIP-01 event ID for the given fields
/// (SHA-256 of the canonical serialization).
pub fn compute_event_id(
    pubkey: &str,
    created_at: u64,
    kind: u16,
    tags: &[Vec<String>],
    content: &str,
) -> NipResult<String> {
    let serialized = serialize_for_id(pubkey, created_at, kind, tags, content);
    let hash = sha2::Sha256::digest(serialized.as_bytes());
    Ok(hex::encode(hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_event() {
        let event = build_event(
            "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459f",
            1644272765,
            1,
            vec![],
            "hello world".to_string(),
        )
        .unwrap();

        assert_eq!(
            event.pubkey,
            "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459f"
        );
        assert_eq!(event.created_at, 1644272765);
        assert_eq!(event.kind, 1);
        assert_eq!(event.content, "hello world");
        assert_eq!(event.id.len(), 64); // SHA-256 hex
        assert!(event.id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_different_data_different_id() {
        let id1 = compute_event_id("pk1", 100, 1, &[], "a").unwrap();
        let id2 = compute_event_id("pk1", 100, 1, &[], "b").unwrap();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_same_data_same_id() {
        let id1 = compute_event_id("pk", 100, 1, &[vec!["e".into(), "abc".into()]], "x").unwrap();
        let id2 = compute_event_id("pk", 100, 1, &[vec!["e".into(), "abc".into()]], "x").unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_canonical_serialization_format() {
        // The NIP-01 canonical format is: [0, pubkey, created_at, kind, tags, content]
        let serialized = serialize_for_id("pk", 100, 1, &[vec!["e".into(), "id".into()]], "hi");
        // Should start with the protocol version marker
        assert!(serialized.starts_with("[0,"));
    }
}
