//! NIP-65 — Relay List Metadata.
//!
//! Defines a replaceable event `kind:10002` that advertises a user's preferred
//! read and write relays. Each relay entry carries an optional `read` or `write`
//! marker; unmaked relays are both read and write.
//!
//! Spec: https://nips.nostr.com/65

use crate::NipMetadata;
use crate::error::{NipError, NipResult};
use serde::{Deserialize, Serialize};

/// A single relay entry in a user's relay list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayEntry {
    /// Relay URL (e.g. "wss://relay.example.com").
    pub url: String,
    /// Optional marker: "read", "write", or None (both).
    pub marker: Option<RelayMarker>,
}

/// Whether a relay is for reading, writing, or both (None).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RelayMarker {
    #[serde(rename = "read")]
    Read,
    #[serde(rename = "write")]
    Write,
}

impl RelayMarker {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

impl std::fmt::Display for RelayMarker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// NIP-65 Relay List Metadata (kind:10002).
///
/// Lists a user's preferred relays for reading, writing, or both.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayListMetadata {
    /// List of relay entries with optional read/write markers.
    pub relays: Vec<RelayEntry>,
    /// Optional text content (typically empty).
    #[serde(default)]
    pub content: String,
}

impl RelayListMetadata {
    /// Create a new relay list from a set of relay entries.
    ///
    /// Spec recommends 2–4 relays per category for optimal discoverability.
    pub fn new(relays: Vec<RelayEntry>) -> Self {
        Self {
            relays,
            content: String::new(),
        }
    }

    /// Create a single relay entry with the given URL and optional marker.
    pub fn relay(url: impl Into<String>, marker: Option<RelayMarker>) -> RelayEntry {
        RelayEntry {
            url: url.into(),
            marker,
        }
    }

    /// Add a relay entry. Keeps the list small per NIP-65 guidance (2-4 per category).
    pub fn add_relay(&mut self, relay: RelayEntry) {
        self.relays.push(relay);
    }

    /// Get all read relays (those with `Read` marker or no marker).
    pub fn read_relays(&self) -> Vec<&RelayEntry> {
        self.relays
            .iter()
            .filter(|r| {
                r.marker
                    .as_ref()
                    .is_none_or(|m| matches!(m, RelayMarker::Read))
            })
            .collect()
    }

    /// Get all write relays (those with `Write` marker or no marker).
    pub fn write_relays(&self) -> Vec<&RelayEntry> {
        self.relays
            .iter()
            .filter(|r| {
                r.marker
                    .as_ref()
                    .is_none_or(|m| matches!(m, RelayMarker::Write))
            })
            .collect()
    }

    /// Get relays with no marker (both read and write).
    pub fn both_relays(&self) -> Vec<&RelayEntry> {
        self.relays.iter().filter(|r| r.marker.is_none()).collect()
    }

    /// Validate the relay list.
    ///
    /// Per NIP-65: URLs should be valid wss:// or ws:// relay addresses.
    pub fn validate(&self) -> NipResult<()> {
        for relay in &self.relays {
            if !relay.url.starts_with("wss://") && !relay.url.starts_with("ws://") {
                return Err(NipError::Validation(format!(
                    "Invalid relay URL: {} (must start with wss:// or ws://)",
                    relay.url
                )));
            }
        }
        Ok(())
    }
}

// ── NipKind ─────────────────────────────────────────────────────────────────

/// Kind for RelayListMetadata — always `10002`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RelayListKind;

impl crate::NipKind for RelayListKind {
    fn kind_value(&self) -> u16 {
        10002
    }

    fn is_replaceable(&self) -> bool {
        true // kind 10002 is in the replaceable range
    }
}

// ── NipMetadata impl ─────────────────────────────────────────────────────────

impl NipMetadata for RelayListMetadata {
    type Kind = RelayListKind;

    fn kind(&self) -> Self::Kind {
        RelayListKind
    }

    fn validate(&self) -> NipResult<()> {
        self.validate()
    }

    fn to_tags(&self) -> Vec<crate::Tag> {
        self.relays
            .iter()
            .map(|r| {
                let mut tag = vec!["r".to_string(), r.url.clone()];
                if let Some(ref marker) = r.marker {
                    tag.push(marker.to_string());
                }
                crate::Tag::new(tag)
            })
            .collect()
    }

    fn content(&self) -> String {
        self.content.clone()
    }

    fn from_raw_event(event: &crate::RawNostrEvent) -> NipResult<Self> {
        let mut relays = Vec::new();
        for tag in &event.tags {
            if tag.first().map(|s| s.as_str()) == Some("r") {
                let url = tag.get(1).cloned().unwrap_or_default();
                let marker = tag.get(2).and_then(|m| match m.as_str() {
                    "read" => Some(RelayMarker::Read),
                    "write" => Some(RelayMarker::Write),
                    _ => None,
                });
                relays.push(RelayEntry { url, marker });
            }
        }
        Ok(Self {
            relays,
            content: event.content.clone(),
        })
    }

    fn d_tag(&self) -> Option<String> {
        // Replaceable event — invariant identifier is empty string per NIP-26
        Some(String::new())
    }
}

// ── Builder and filter methods ──────────────────────────────────────────────

impl std::iter::FromIterator<RelayEntry> for RelayListMetadata {
    fn from_iter<I: IntoIterator<Item = RelayEntry>>(iter: I) -> Self {
        Self {
            relays: iter.into_iter().collect(),
            content: String::new(),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NipKind as _;

    #[test]
    fn test_relay_list_kind() {
        let kind = RelayListKind;
        assert_eq!(kind.kind_value(), 10002);
        assert!(kind.is_replaceable());
        assert!(!kind.is_regular());
    }

    #[test]
    fn test_relay_entry_creation() {
        let r = RelayListMetadata::relay("wss://relay.example.com", Some(RelayMarker::Read));
        assert_eq!(r.url, "wss://relay.example.com");
        assert_eq!(r.marker, Some(RelayMarker::Read));
    }

    #[test]
    fn test_relay_no_marker() {
        let r = RelayListMetadata::relay("wss://both.example.com", None);
        assert!(r.marker.is_none());
    }

    #[test]
    fn test_read_write_relays() {
        let list = RelayListMetadata::new(vec![
            RelayListMetadata::relay("wss://read.example.com", Some(RelayMarker::Read)),
            RelayListMetadata::relay("wss://write.example.com", Some(RelayMarker::Write)),
            RelayListMetadata::relay("wss://both.example.com", None),
        ]);

        assert_eq!(list.read_relays().len(), 2); // read + both
        assert_eq!(list.write_relays().len(), 2); // write + both
        assert_eq!(list.both_relays().len(), 1);
    }

    #[test]
    fn test_to_tags() {
        let list = RelayListMetadata::new(vec![
            RelayListMetadata::relay("wss://read.example.com", Some(RelayMarker::Read)),
            RelayListMetadata::relay("wss://write.example.com", Some(RelayMarker::Write)),
            RelayListMetadata::relay("wss://both.example.com", None),
        ]);

        let tags = list.to_tags();
        assert_eq!(tags.len(), 3);

        assert_eq!(tags[0].as_ref(), &["r", "wss://read.example.com", "read"]);
        assert_eq!(tags[1].as_ref(), &["r", "wss://write.example.com", "write"]);
        assert_eq!(tags[2].as_ref(), &["r", "wss://both.example.com"]);
    }

    #[test]
    fn test_from_raw_event() {
        let raw = crate::RawNostrEvent {
            id: String::new(),
            pubkey: "abc".into(),
            created_at: 1700000000,
            kind: 10002,
            tags: vec![
                vec!["r".into(), "wss://relay1.com".into(), "read".into()],
                vec!["r".into(), "wss://relay2.com".into(), "write".into()],
            ],
            content: String::new(),
            sig: String::new(),
        };

        let list = RelayListMetadata::from_raw_event(&raw).unwrap();
        assert_eq!(list.relays.len(), 2);
        assert_eq!(list.relays[0].url, "wss://relay1.com");
        assert_eq!(list.relays[0].marker, Some(RelayMarker::Read));
    }

    #[test]
    fn test_validate_valid_urls() {
        let list = RelayListMetadata::new(vec![RelayListMetadata::relay("wss://valid.com", None)]);
        assert!(list.validate().is_ok());
    }

    #[test]
    fn test_validate_invalid_urls() {
        let list =
            RelayListMetadata::new(vec![RelayListMetadata::relay("http://invalid.com", None)]);
        assert!(list.validate().is_err());
    }

    #[test]
    fn test_d_tag() {
        let list = RelayListMetadata::new(vec![]);
        assert_eq!(list.d_tag(), Some(String::new()));
    }

    #[test]
    fn test_collect_from_iter() {
        let entries = vec![
            RelayListMetadata::relay("wss://a.com", None),
            RelayListMetadata::relay("wss://b.com", None),
        ];
        let list: RelayListMetadata = entries.into_iter().collect();
        assert_eq!(list.relays.len(), 2);
    }
}
