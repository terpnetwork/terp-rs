use crate::{
    NipKind, NipMetadata, RawNostrEvent, Tag,
    error::{NipError, NipResult},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::Digest as _;

// ==================== NIP-01 Kinds ====================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Nip01Kind {
    Metadata,
    TextNote,
    RecommendRelay,
    ContactList,
    EncryptedDm,
    Deletion,
    Repost,
    Reaction,
}

impl NipKind for Nip01Kind {
    fn kind_value(&self) -> u16 {
        match self {
            Nip01Kind::Metadata => 0,
            Nip01Kind::TextNote => 1,
            Nip01Kind::RecommendRelay => 2,
            Nip01Kind::ContactList => 3,
            Nip01Kind::EncryptedDm => 4,
            Nip01Kind::Deletion => 5,
            Nip01Kind::Repost => 6,
            Nip01Kind::Reaction => 7,
        }
    }
}

// ==================== NIP-01 Metadata Types ====================

/// Kind 0: User metadata
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UserMetadata {
    pub name: Option<String>,
    pub about: Option<String>,
    pub picture: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl NipMetadata for UserMetadata {
    type Kind = Nip01Kind;

    fn kind(&self) -> Self::Kind {
        Nip01Kind::Metadata
    }

    fn validate(&self) -> NipResult<()> {
        // At least one field should be set
        if self.name.is_none()
            && self.about.is_none()
            && self.picture.is_none()
            && self.extra.is_empty()
        {
            return Err(NipError::Validation(
                "User metadata must have at least one field".into(),
            ));
        }
        Ok(())
    }

    fn to_tags(&self) -> Vec<Tag> {
        Vec::new() // Kind 0 events don't require specific tags
    }

    fn content(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    fn from_raw_event(event: &RawNostrEvent) -> NipResult<Self> {
        if event.kind != Nip01Kind::Metadata.kind_value() {
            return Err(NipError::Nip01(format!(
                "Expected kind 0, got {}",
                event.kind
            )));
        }
        serde_json::from_str(&event.content).map_err(NipError::Serialization)
    }
}

/// Kind 1: Text note
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TextNote {
    pub content: String,
    pub event_references: Vec<EventReference>,
    pub pubkey_references: Vec<PubKeyReference>,
    pub hashtags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EventReference {
    pub event_id: String,
    pub relay_url: Option<String>,
    pub marker: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PubKeyReference {
    pub pubkey: String,
    pub relay_url: Option<String>,
}

impl NipMetadata for TextNote {
    type Kind = Nip01Kind;

    fn kind(&self) -> Self::Kind {
        Nip01Kind::TextNote
    }

    fn validate(&self) -> NipResult<()> {
        if self.content.is_empty() {
            return Err(NipError::Validation(
                "Text note content cannot be empty".into(),
            ));
        }
        Ok(())
    }

    fn to_tags(&self) -> Vec<Tag> {
        let mut tags = Vec::new();

        for reference in &self.event_references {
            let mut tag = vec![String::from("e"), reference.event_id.clone()];
            if let Some(ref relay) = reference.relay_url {
                tag.push(relay.clone());
                if let Some(ref marker) = reference.marker {
                    tag.push(marker.clone());
                }
            }
            tags.push(Tag::new(tag));
        }

        for reference in &self.pubkey_references {
            let mut tag = vec![String::from("p"), reference.pubkey.clone()];
            if let Some(ref relay) = reference.relay_url {
                tag.push(relay.clone());
            }
            tags.push(Tag::new(tag));
        }

        for hashtag in &self.hashtags {
            tags.push(Tag::t(hashtag));
        }

        tags
    }

    fn content(&self) -> String {
        self.content.clone()
    }

    fn from_raw_event(event: &RawNostrEvent) -> NipResult<Self> {
        if event.kind != Nip01Kind::TextNote.kind_value() {
            return Err(NipError::Nip01(format!(
                "Expected kind 1, got {}",
                event.kind
            )));
        }

        let mut event_references = Vec::new();
        let mut pubkey_references = Vec::new();
        let mut hashtags = Vec::new();

        for tag in &event.tags {
            if tag.len() < 2 {
                continue;
            }
            match tag[0].as_str() {
                "e" => {
                    let reference = EventReference {
                        event_id: tag[1].clone(),
                        relay_url: tag.get(2).cloned(),
                        marker: tag.get(3).cloned(),
                    };
                    event_references.push(reference);
                }
                "p" => {
                    let reference = PubKeyReference {
                        pubkey: tag[1].clone(),
                        relay_url: tag.get(2).cloned(),
                    };
                    pubkey_references.push(reference);
                }
                "t" => {
                    hashtags.push(tag[1].clone());
                }
                _ => {}
            }
        }

        Ok(TextNote {
            content: event.content.clone(),
            event_references,
            pubkey_references,
            hashtags,
        })
    }
}

/// Kind 5: Deletion event
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Deletion {
    pub event_ids: Vec<String>,
    pub reason: Option<String>,
}

impl NipMetadata for Deletion {
    type Kind = Nip01Kind;

    fn kind(&self) -> Self::Kind {
        Nip01Kind::Deletion
    }

    fn validate(&self) -> NipResult<()> {
        if self.event_ids.is_empty() {
            return Err(NipError::Validation(
                "Deletion must specify at least one event ID".into(),
            ));
        }
        Ok(())
    }

    fn to_tags(&self) -> Vec<Tag> {
        self.event_ids.iter().map(|id| Tag::e(id)).collect()
    }

    fn content(&self) -> String {
        self.reason.clone().unwrap_or_default()
    }

    fn from_raw_event(event: &RawNostrEvent) -> NipResult<Self> {
        if event.kind != Nip01Kind::Deletion.kind_value() {
            return Err(NipError::Nip01(format!(
                "Expected kind 5, got {}",
                event.kind
            )));
        }

        let event_ids: Vec<String> = event
            .tags
            .iter()
            .filter(|tag| tag.first().map(|n| n.as_str()) == Some("e"))
            .filter_map(|tag| tag.get(1).cloned())
            .collect();

        let reason = if event.content.is_empty() {
            None
        } else {
            Some(event.content.clone())
        };

        Ok(Deletion { event_ids, reason })
    }
}

// ==================== Helper Functions ====================

/// Parse a NIP-01 event from raw JSON
pub fn parse_event(json: &str) -> NipResult<RawNostrEvent> {
    serde_json::from_str(json).map_err(NipError::Serialization)
}

/// Serialize a NIP-01 event to JSON
pub fn serialize_event(event: &RawNostrEvent) -> NipResult<String> {
    serde_json::to_string(event).map_err(NipError::Serialization)
}

/// Compute event ID according to NIP-01
pub fn compute_event_id(
    pubkey: &str,
    created_at: u64,
    kind: u16,
    tags: &[Vec<String>],
    content: &str,
) -> NipResult<String> {
    let serialized = RawNostrEvent::serialize_for_id(pubkey, created_at, kind, tags, content);
    let hash = sha2::Sha256::digest(serialized.as_bytes());
    Ok(hex::encode(hash))
}

#[cfg(test)]
mod tests {
    use crate::{NipKind, NipMetadata, RawNostrEvent};

    use super::*;

    // ==================== Nip01Kind kind_value() ====================

    #[test]
    fn test_nip01kind_kind_value_metadata() {
        assert_eq!(Nip01Kind::Metadata.kind_value(), 0);
    }

    #[test]
    fn test_nip01kind_kind_value_text_note() {
        assert_eq!(Nip01Kind::TextNote.kind_value(), 1);
    }

    #[test]
    fn test_nip01kind_kind_value_recommend_relay() {
        assert_eq!(Nip01Kind::RecommendRelay.kind_value(), 2);
    }

    #[test]
    fn test_nip01kind_kind_value_contact_list() {
        assert_eq!(Nip01Kind::ContactList.kind_value(), 3);
    }

    #[test]
    fn test_nip01kind_kind_value_encrypted_dm() {
        assert_eq!(Nip01Kind::EncryptedDm.kind_value(), 4);
    }

    #[test]
    fn test_nip01kind_kind_value_deletion() {
        assert_eq!(Nip01Kind::Deletion.kind_value(), 5);
    }

    #[test]
    fn test_nip01kind_kind_value_repost() {
        assert_eq!(Nip01Kind::Repost.kind_value(), 6);
    }

    #[test]
    fn test_nip01kind_kind_value_reaction() {
        assert_eq!(Nip01Kind::Reaction.kind_value(), 7);
    }

    // ==================== UserMetadata ====================

    #[test]
    fn test_user_metadata_create_with_all_fields() {
        let mut extra = serde_json::Map::new();
        extra.insert(
            "display_name".into(),
            serde_json::Value::String("Alice".into()),
        );
        extra.insert(
            "nip05".into(),
            serde_json::Value::String("alice@example.com".into()),
        );

        let meta = UserMetadata {
            name: Some("Alice".into()),
            about: Some("A test user".into()),
            picture: Some("https://example.com/avatar.png".into()),
            extra,
        };
        assert_eq!(meta.name.as_deref(), Some("Alice"));
        assert_eq!(meta.about.as_deref(), Some("A test user"));
        assert_eq!(
            meta.picture.as_deref(),
            Some("https://example.com/avatar.png")
        );
        assert!(meta.extra.contains_key("display_name"));
    }

    #[test]
    fn test_user_metadata_validate_passes() {
        let meta = UserMetadata {
            name: Some("Alice".into()),
            about: None,
            picture: None,
            extra: serde_json::Map::new(),
        };
        assert!(meta.validate().is_ok());
    }

    #[test]
    fn test_user_metadata_validate_fails_empty() {
        let meta = UserMetadata {
            name: None,
            about: None,
            picture: None,
            extra: serde_json::Map::new(),
        };
        let result = meta.validate();
        assert!(result.is_err());
        if let Err(crate::error::NipError::Validation(msg)) = result {
            assert!(msg.contains("at least one field"));
        } else {
            panic!("Expected Validation error");
        }
    }

    #[test]
    fn test_user_metadata_to_tags_returns_empty_vec() {
        let meta = UserMetadata {
            name: Some("Alice".into()),
            about: None,
            picture: None,
            extra: serde_json::Map::new(),
        };
        let tags = meta.to_tags();
        assert!(tags.is_empty());
    }

    #[test]
    fn test_user_metadata_content_returns_json() {
        let meta = UserMetadata {
            name: Some("Alice".into()),
            about: Some("A test user".into()),
            picture: None,
            extra: serde_json::Map::new(),
        };
        let content = meta.content();
        assert!(content.contains("\"name\""));
        assert!(content.contains("Alice"));
        assert!(content.contains("\"about\""));
        assert!(content.contains("A test user"));
    }

    #[test]
    fn test_user_metadata_from_raw_event_correct_kind() {
        let mut extra = serde_json::Map::new();
        extra.insert(
            "display_name".into(),
            serde_json::Value::String("Alice".into()),
        );
        let meta = UserMetadata {
            name: Some("Alice".into()),
            about: None,
            picture: None,
            extra,
        };
        let json_content = meta.content();

        let event = RawNostrEvent {
            id: "abc".into(),
            pubkey: "pubkey123".into(),
            created_at: 1700000000,
            kind: 0,
            tags: vec![],
            content: json_content,
            sig: "sig".into(),
        };

        let parsed = UserMetadata::from_raw_event(&event).expect("should parse successfully");
        assert_eq!(parsed.name.as_deref(), Some("Alice"));
        assert_eq!(
            parsed.extra.get("display_name").and_then(|v| v.as_str()),
            Some("Alice")
        );
    }

    #[test]
    fn test_user_metadata_from_raw_event_incorrect_kind() {
        let event = RawNostrEvent {
            id: "abc".into(),
            pubkey: "pubkey123".into(),
            created_at: 1700000000,
            kind: 1,
            tags: vec![],
            content: "{}".into(),
            sig: "sig".into(),
        };

        let result = UserMetadata::from_raw_event(&event);
        assert!(result.is_err());
    }

    // ==================== TextNote ====================

    #[test]
    fn test_text_note_create_with_refs() {
        let note = TextNote {
            content: "Hello world".into(),
            event_references: vec![
                EventReference {
                    event_id: "evt1".into(),
                    relay_url: None,
                    marker: None,
                },
                EventReference {
                    event_id: "evt2".into(),
                    relay_url: Some("wss://relay.example.com".into()),
                    marker: Some("reply".into()),
                },
            ],
            pubkey_references: vec![
                PubKeyReference {
                    pubkey: "pk1".into(),
                    relay_url: None,
                },
                PubKeyReference {
                    pubkey: "pk2".into(),
                    relay_url: Some("wss://relay2.example.com".into()),
                },
            ],
            hashtags: vec!["nostr".into(), "test".into()],
        };
        assert_eq!(note.content, "Hello world");
        assert_eq!(note.event_references.len(), 2);
        assert_eq!(note.pubkey_references.len(), 2);
        assert_eq!(note.hashtags.len(), 2);
    }

    #[test]
    fn test_text_note_validate_passes() {
        let note = TextNote {
            content: "Hello world".into(),
            event_references: vec![],
            pubkey_references: vec![],
            hashtags: vec![],
        };
        assert!(note.validate().is_ok());
    }

    #[test]
    fn test_text_note_validate_fails_empty_content() {
        let note = TextNote {
            content: "".into(),
            event_references: vec![],
            pubkey_references: vec![],
            hashtags: vec![],
        };
        let result = note.validate();
        assert!(result.is_err());
        if let Err(crate::error::NipError::Validation(msg)) = result {
            assert!(msg.contains("cannot be empty"));
        } else {
            panic!("Expected Validation error");
        }
    }

    #[test]
    fn test_text_note_to_tags_structure() {
        let note = TextNote {
            content: "Hello".into(),
            event_references: vec![
                EventReference {
                    event_id: "evt1".into(),
                    relay_url: None,
                    marker: None,
                },
                EventReference {
                    event_id: "evt2".into(),
                    relay_url: Some("wss://relay.example.com".into()),
                    marker: None,
                },
                EventReference {
                    event_id: "evt3".into(),
                    relay_url: Some("wss://relay.example.com".into()),
                    marker: Some("reply".into()),
                },
            ],
            pubkey_references: vec![
                PubKeyReference {
                    pubkey: "pk1".into(),
                    relay_url: None,
                },
                PubKeyReference {
                    pubkey: "pk2".into(),
                    relay_url: Some("wss://relay2.example.com".into()),
                },
            ],
            hashtags: vec!["nostr".into()],
        };

        let tags = note.to_tags();

        // Find event tags
        let e_tags: Vec<&Tag> = tags.iter().filter(|t| t.name() == Some("e")).collect();
        assert_eq!(e_tags.len(), 3);

        // First e-tag: event_id only, no relay/marker
        assert_eq!(e_tags[0].value(), Some("evt1"));
        assert!(e_tags[0].additional().is_empty());

        // Second e-tag: event_id + relay, no marker
        assert_eq!(e_tags[1].value(), Some("evt2"));
        let add1 = e_tags[1].additional();
        assert_eq!(add1.len(), 1);
        assert_eq!(add1[0], "wss://relay.example.com");

        // Third e-tag: event_id + relay + marker
        assert_eq!(e_tags[2].value(), Some("evt3"));
        let add2 = e_tags[2].additional();
        assert_eq!(add2.len(), 2);
        assert_eq!(add2[0], "wss://relay.example.com");
        assert_eq!(add2[1], "reply");

        // Find p tags
        let p_tags: Vec<&Tag> = tags.iter().filter(|t| t.name() == Some("p")).collect();
        assert_eq!(p_tags.len(), 2);
        assert_eq!(p_tags[0].value(), Some("pk1"));
        assert!(p_tags[0].additional().is_empty());
        assert_eq!(p_tags[1].value(), Some("pk2"));
        assert_eq!(p_tags[1].additional(), &["wss://relay2.example.com"]);

        // Find t tags
        let t_tags: Vec<&Tag> = tags.iter().filter(|t| t.name() == Some("t")).collect();
        assert_eq!(t_tags.len(), 1);
        assert_eq!(t_tags[0].value(), Some("nostr"));
    }

    #[test]
    fn test_text_note_from_raw_event_proper_tag_extraction() {
        let tags: Vec<Vec<String>> = vec![
            vec!["e".into(), "evt1".into()],
            vec![
                "e".into(),
                "evt2".into(),
                "wss://relay.example.com".into(),
                "reply".into(),
            ],
            vec!["p".into(), "pk1".into()],
            vec!["p".into(), "pk2".into(), "wss://relay2.example.com".into()],
            vec!["t".into(), "nostr".into()],
            vec!["unknown".into(), "val".into()], // should be ignored
        ];

        let event = RawNostrEvent {
            id: "abc".into(),
            pubkey: "pubkey".into(),
            created_at: 1700000000,
            kind: 1,
            tags,
            content: "Hello world".into(),
            sig: "sig".into(),
        };

        let note = TextNote::from_raw_event(&event).expect("should parse");
        assert_eq!(note.content, "Hello world");
        assert_eq!(note.event_references.len(), 2);
        assert_eq!(note.event_references[0].event_id, "evt1");
        assert_eq!(note.event_references[0].relay_url, None);
        assert_eq!(note.event_references[0].marker, None);
        assert_eq!(note.event_references[1].event_id, "evt2");
        assert_eq!(
            note.event_references[1].relay_url.as_deref(),
            Some("wss://relay.example.com")
        );
        assert_eq!(note.event_references[1].marker.as_deref(), Some("reply"));
        assert_eq!(note.pubkey_references.len(), 2);
        assert_eq!(note.pubkey_references[0].pubkey, "pk1");
        assert_eq!(note.pubkey_references[0].relay_url, None);
        assert_eq!(note.pubkey_references[1].pubkey, "pk2");
        assert_eq!(
            note.pubkey_references[1].relay_url.as_deref(),
            Some("wss://relay2.example.com")
        );
        assert_eq!(note.hashtags, vec!["nostr"]);
    }

    #[test]
    fn test_text_note_from_raw_event_incorrect_kind() {
        let event = RawNostrEvent {
            id: "abc".into(),
            pubkey: "pubkey".into(),
            created_at: 1700000000,
            kind: 0,
            tags: vec![],
            content: "hello".into(),
            sig: "sig".into(),
        };
        let result = TextNote::from_raw_event(&event);
        assert!(result.is_err());
    }

    // ==================== Deletion ====================

    #[test]
    fn test_deletion_create_with_event_ids_and_reason() {
        let del = Deletion {
            event_ids: vec!["evt1".into(), "evt2".into()],
            reason: Some("spam".into()),
        };
        assert_eq!(del.event_ids.len(), 2);
        assert_eq!(del.reason.as_deref(), Some("spam"));
    }

    #[test]
    fn test_deletion_validate_passes() {
        let del = Deletion {
            event_ids: vec!["evt1".into()],
            reason: None,
        };
        assert!(del.validate().is_ok());
    }

    #[test]
    fn test_deletion_validate_fails_empty_ids() {
        let del = Deletion {
            event_ids: vec![],
            reason: None,
        };
        let result = del.validate();
        assert!(result.is_err());
        if let Err(crate::error::NipError::Validation(msg)) = result {
            assert!(msg.contains("at least one event ID"));
        } else {
            panic!("Expected Validation error");
        }
    }

    #[test]
    fn test_deletion_to_tags_produces_e_tags() {
        let del = Deletion {
            event_ids: vec!["evt1".into(), "evt2".into()],
            reason: None,
        };
        let tags = del.to_tags();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].name(), Some("e"));
        assert_eq!(tags[0].value(), Some("evt1"));
        assert_eq!(tags[1].name(), Some("e"));
        assert_eq!(tags[1].value(), Some("evt2"));
    }

    #[test]
    fn test_deletion_from_raw_event() {
        let tags: Vec<Vec<String>> = vec![
            vec!["e".into(), "evt1".into()],
            vec!["p".into(), "pk1".into()], // should be filtered out
            vec!["e".into(), "evt2".into()],
        ];
        let event = RawNostrEvent {
            id: "abc".into(),
            pubkey: "pubkey".into(),
            created_at: 1700000000,
            kind: 5,
            tags,
            content: "spam".into(),
            sig: "sig".into(),
        };
        let del = Deletion::from_raw_event(&event).expect("should parse");
        assert_eq!(del.event_ids, vec!["evt1", "evt2"]);
        assert_eq!(del.reason.as_deref(), Some("spam"));
    }

    #[test]
    fn test_deletion_from_raw_event_empty_reason() {
        let tags: Vec<Vec<String>> = vec![vec!["e".into(), "evt1".into()]];
        let event = RawNostrEvent {
            id: "abc".into(),
            pubkey: "pubkey".into(),
            created_at: 1700000000,
            kind: 5,
            tags,
            content: "".into(),
            sig: "sig".into(),
        };
        let del = Deletion::from_raw_event(&event).expect("should parse");
        assert_eq!(del.event_ids, vec!["evt1"]);
        assert!(del.reason.is_none());
    }

    #[test]
    fn test_deletion_from_raw_event_incorrect_kind() {
        let event = RawNostrEvent {
            id: "abc".into(),
            pubkey: "pubkey".into(),
            created_at: 1700000000,
            kind: 1,
            tags: vec![],
            content: "".into(),
            sig: "sig".into(),
        };
        let result = Deletion::from_raw_event(&event);
        assert!(result.is_err());
    }

    // ==================== parse_event roundtrip ====================

    #[test]
    fn test_parse_event_roundtrip() {
        let json = r#"{"id":"abc123","pubkey":"pk123","created_at":1700000000,"kind":1,"tags":[["e","evt1"]],"content":"hello","sig":"sig456"}"#;
        let event = parse_event(json).expect("should parse");
        assert_eq!(event.id, "abc123");
        assert_eq!(event.pubkey, "pk123");
        assert_eq!(event.created_at, 1700000000);
        assert_eq!(event.kind, 1);
        assert_eq!(event.tags.len(), 1);
        assert_eq!(event.tags[0][0], "e");
        assert_eq!(event.tags[0][1], "evt1");
        assert_eq!(event.content, "hello");
        assert_eq!(event.sig, "sig456");
    }

    #[test]
    fn test_parse_event_invalid_json() {
        let result = parse_event("not valid json");
        assert!(result.is_err());
    }

    // ==================== serialize_event roundtrip ====================

    #[test]
    fn test_serialize_event_roundtrip() {
        let event = RawNostrEvent {
            id: "id123".into(),
            pubkey: "pk123".into(),
            created_at: 1700000000,
            kind: 0,
            tags: vec![vec!["d".into(), "test".into()]],
            content: "hello".into(),
            sig: "sig789".into(),
        };
        let json = serialize_event(&event).expect("should serialize");
        let parsed = parse_event(&json).expect("should parse back");
        assert_eq!(parsed.id, event.id);
        assert_eq!(parsed.pubkey, event.pubkey);
        assert_eq!(parsed.created_at, event.created_at);
        assert_eq!(parsed.kind, event.kind);
        assert_eq!(parsed.tags, event.tags);
        assert_eq!(parsed.content, event.content);
        assert_eq!(parsed.sig, event.sig);
    }

    // ==================== compute_event_id deterministic ====================

    #[test]
    fn test_compute_event_id_deterministic() {
        let pubkey = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459f";
        let created_at = 1700000000;
        let kind = 1u16;
        let tags: &[Vec<String>] = &[vec!["p".into(), "abc123".into()]];
        let content = "hello";

        let id1 =
            compute_event_id(pubkey, created_at, kind, tags, content).expect("should succeed");
        let id2 =
            compute_event_id(pubkey, created_at, kind, tags, content).expect("should succeed");

        assert_eq!(id1, id2, "event ID should be deterministic");
        assert_eq!(id1.len(), 64, "SHA-256 hex should be 64 chars");
        assert!(id1.chars().all(|c| c.is_ascii_hexdigit()), "should be hex");
    }

    #[test]
    fn test_compute_event_id_different_data_different_id() {
        let pubkey = "pk";
        let created_at = 1700000000;
        let kind = 1u16;
        let tags: &[Vec<String>] = &[];
        let content_a = "hello";
        let content_b = "world";

        let id_a =
            compute_event_id(pubkey, created_at, kind, tags, content_a).expect("should succeed");
        let id_b =
            compute_event_id(pubkey, created_at, kind, tags, content_b).expect("should succeed");

        assert_ne!(id_a, id_b, "different content should produce different IDs");
    }
}
