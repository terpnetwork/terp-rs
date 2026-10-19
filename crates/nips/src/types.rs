use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::error::NipError;

// ==================== Primitive Types ====================

/// A 32-byte lowercase hex-encoded public key
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PubKey(String);

impl PubKey {
    pub fn new(hex: impl Into<String>) -> Result<Self, NipError> {
        let hex = hex.into();
        if hex.len() != 64 {
            return Err(NipError::InvalidPubKeyLength(hex.len()));
        }
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(NipError::InvalidHexFormat);
        }
        Ok(Self(hex.to_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }

    pub fn to_bytes(&self) -> Result<[u8; 32], NipError> {
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(&self.0, &mut bytes).map_err(NipError::HexDecodeError)?;
        Ok(bytes)
    }
}

impl fmt::Display for PubKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for PubKey {
    type Err = NipError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl AsRef<str> for PubKey {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// A 32-byte lowercase hex-encoded event ID
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventId(String);

impl EventId {
    pub fn new(hex: impl Into<String>) -> Result<Self, NipError> {
        let hex = hex.into();
        if hex.len() != 64 {
            return Err(NipError::InvalidEventIdLength(hex.len()));
        }
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(NipError::InvalidHexFormat);
        }
        Ok(Self(hex.to_lowercase()))
    }

    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        Self(hex::encode(bytes))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn to_bytes(&self) -> Result<[u8; 32], NipError> {
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(&self.0, &mut bytes).map_err(NipError::HexDecodeError)?;
        Ok(bytes)
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for EventId {
    type Err = NipError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

/// A 64-byte lowercase hex-encoded Schnorr signature
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature(String);

impl Signature {
    pub fn new(hex: impl Into<String>) -> Result<Self, NipError> {
        let hex = hex.into();
        if hex.len() != 128 {
            return Err(NipError::InvalidSignatureLength(hex.len()));
        }
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(NipError::InvalidHexFormat);
        }
        Ok(Self(hex.to_lowercase()))
    }

    pub fn from_bytes(bytes: &[u8; 64]) -> Self {
        Self(hex::encode(bytes))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn to_bytes(&self) -> Result<[u8; 64], NipError> {
        let mut bytes = [0u8; 64];
        hex::decode_to_slice(&self.0, &mut bytes).map_err(NipError::HexDecodeError)?;
        Ok(bytes)
    }
}

/// Unix timestamp in seconds
pub type UnixTimestamp = u64;

/// Tag array - each tag is a vector of strings
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag(pub Vec<String>);

impl Tag {
    // ==================== NIP-73 Helpers ====================

    /// Add an `i` + `k` tag pair for external content ID (NIP-73)
    pub fn i(id: impl Into<String>, kind: impl Into<String>) -> (Self, Self) {
        (
            Tag(vec!["i".to_string(), id.into()]),
            Tag(vec!["k".to_string(), kind.into()]),
        )
    }

    /// Add `i` tag with optional URL hint
    pub fn i_with_hint(
        id: impl Into<String>,
        kind: impl Into<String>,
        url_hint: Option<String>,
    ) -> Vec<Self> {
        let mut tags = vec![
            Tag(vec!["i".to_string(), id.into()]),
            Tag(vec!["k".to_string(), kind.into()]),
        ];
        if let Some(url) = url_hint {
            tags.push(Tag(vec!["i".to_string(), url])); // second arg to i-tag
        }
        tags
    }
    pub fn new(items: Vec<String>) -> Self {
        Self(items)
    }

    pub fn empty() -> Self {
        Self(Vec::new())
    }

    /// Create an "e" tag (event reference)
    pub fn e(event_id: impl Into<String>) -> Self {
        Self(vec![String::from("e"), event_id.into()])
    }

    /// Create an "e" tag with relay URL
    pub fn e_with_relay(event_id: impl Into<String>, relay: impl Into<String>) -> Self {
        Self(vec![String::from("e"), event_id.into(), relay.into()])
    }

    /// Create an "e" tag with relay URL and author pubkey
    pub fn e_full(
        event_id: impl Into<String>,
        relay: impl Into<String>,
        author: impl Into<String>,
    ) -> Self {
        Self(vec![
            String::from("e"),
            event_id.into(),
            relay.into(),
            author.into(),
        ])
    }

    /// Create a "p" tag (pubkey reference)
    pub fn p(pubkey: impl Into<String>) -> Self {
        Self(vec![String::from("p"), pubkey.into()])
    }

    /// Create a "p" tag with relay URL
    pub fn p_with_relay(pubkey: impl Into<String>, relay: impl Into<String>) -> Self {
        Self(vec![String::from("p"), pubkey.into(), relay.into()])
    }

    /// Create an "a" tag (addressable event reference)
    pub fn a(kind: u16, pubkey: impl Into<String>, d: impl Into<String>) -> Self {
        Self(vec![
            String::from("a"),
            format!("{}:{}:{}", kind, pubkey.into(), d.into()),
        ])
    }

    /// Create an "a" tag with relay URL
    pub fn a_with_relay(
        kind: u16,
        pubkey: impl Into<String>,
        d: impl Into<String>,
        relay: impl Into<String>,
    ) -> Self {
        Self(vec![
            String::from("a"),
            format!("{}:{}:{}", kind, pubkey.into(), d.into()),
            relay.into(),
        ])
    }

    /// Create a "d" tag (replaceable/addressable event identifier)
    pub fn d(value: impl Into<String>) -> Self {
        Self(vec![String::from("d"), value.into()])
    }

    /// Create a "t" tag (hashtag)
    pub fn t(value: impl Into<String>) -> Self {
        Self(vec![String::from("t"), value.into()])
    }

    /// Create an "alt" tag (alternative description)
    pub fn alt(value: impl Into<String>) -> Self {
        Self(vec![String::from("alt"), value.into()])
    }

    pub fn k(kind: impl Into<u16>) -> Self {
        Self(vec!["k".to_string(), kind.into().to_string()])
    }

    /// Create a "g" tag (geohash)
    pub fn g(value: impl Into<String>) -> Self {
        Self(vec![String::from("g"), value.into()])
    }

    /// Create an "r" tag (reference/link)
    pub fn r(value: impl Into<String>) -> Self {
        Self(vec![String::from("r"), value.into()])
    }

    /// Get the tag name (first element)
    pub fn name(&self) -> Option<&str> {
        self.0.first().map(|s| s.as_str())
    }

    /// Get the tag value (second element)
    pub fn value(&self) -> Option<&str> {
        self.0.get(1).map(|s| s.as_str())
    }

    /// Get additional elements after the value
    pub fn additional(&self) -> &[String] {
        if self.0.len() > 2 { &self.0[2..] } else { &[] }
    }

    pub fn into_inner(self) -> Vec<String> {
        self.0
    }

    pub fn as_slice(&self) -> &[String] {
        &self.0
    }
}

impl From<Vec<String>> for Tag {
    fn from(v: Vec<String>) -> Self {
        Self(v)
    }
}

impl From<Tag> for Vec<String> {
    fn from(tag: Tag) -> Self {
        tag.0
    }
}

impl AsRef<[String]> for Tag {
    fn as_ref(&self) -> &[String] {
        &self.0
    }
}

/// Event kind - integer between 0 and 65535
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Kind(u16);

impl Kind {
    // NIP-01 kinds
    pub const METADATA: Kind = Kind(0);
    pub const TEXT_NOTE: Kind = Kind(1);
    pub const RECOMMEND_RELAY: Kind = Kind(2);
    pub const CONTACT_LIST: Kind = Kind(3);
    pub const DM: Kind = Kind(4);
    pub const DELETION: Kind = Kind(5);
    pub const REPOST: Kind = Kind(6);
    pub const REACTION: Kind = Kind(7);

    // NIP-15 Marketplace kinds
    pub const SET_STALL: Kind = Kind(30017);
    pub const SET_PRODUCT: Kind = Kind(30018);
    pub const MARKETPLACE_UI: Kind = Kind(30019);
    pub const AUCTION_PRODUCT: Kind = Kind(30020);
    pub const BID: Kind = Kind(1021);
    pub const BID_CONFIRMATION: Kind = Kind(1022);

    // NIP-52 Calendar kinds
    pub const DATE_EVENT: Kind = Kind(31922);
    pub const TIME_EVENT: Kind = Kind(31923);
    pub const CALENDAR: Kind = Kind(31924);
    pub const RSVP: Kind = Kind(31925);

    pub fn new(value: u16) -> Self {
        Kind(value)
    }

    pub fn value(&self) -> u16 {
        self.0
    }

    /// Regular events: kinds 1000-9999, 4-44, 1, 2
    pub fn is_regular(self) -> bool {
        (1000..10000).contains(&self.0) || (4..45).contains(&self.0) || self.0 == 1 || self.0 == 2
    }

    /// Replaceable events: kinds 10000-19999, 0, 3
    pub fn is_replaceable(self) -> bool {
        (10000..20000).contains(&self.0) || self.0 == 0 || self.0 == 3
    }

    /// Ephemeral events: kinds 20000-29999
    pub fn is_ephemeral(self) -> bool {
        (20000..30000).contains(&self.0)
    }

    /// Addressable events: kinds 30000-39999
    pub fn is_addressable(self) -> bool {
        (30000..40000).contains(&self.0)
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u16> for Kind {
    fn from(value: u16) -> Self {
        Kind(value)
    }
}

impl From<Kind> for u16 {
    fn from(kind: Kind) -> Self {
        kind.0
    }
}

/// Relay URL
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RelayUrl(String);

impl RelayUrl {
    pub fn new(url: impl Into<String>) -> Result<Self, NipError> {
        let url = url.into();
        if !url.starts_with("wss://") && !url.starts_with("ws://") {
            return Err(NipError::InvalidRelayUrl(url));
        }
        Ok(Self(url))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RelayUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== PubKey Tests ====================

    #[test]
    fn test_pubkey_valid() {
        let hex = "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
        let pk = PubKey::new(hex).unwrap();
        assert_eq!(pk.as_str(), hex);
        assert_eq!(pk.into_inner(), hex);
    }

    #[test]
    fn test_pubkey_invalid_length() {
        let err = PubKey::new("tooshort").unwrap_err();
        match err {
            NipError::InvalidPubKeyLength(len) => assert_eq!(len, 8),
            _ => panic!("Expected InvalidPubKeyLength"),
        }
    }

    #[test]
    fn test_pubkey_invalid_hex() {
        let err = PubKey::new("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz")
            .unwrap_err();
        match err {
            NipError::InvalidHexFormat => {}
            _ => panic!("Expected InvalidHexFormat"),
        }
    }

    #[test]
    fn test_pubkey_display() {
        let hex = "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
        let pk = PubKey::new(hex).unwrap();
        assert_eq!(format!("{}", pk), hex);
    }

    #[test]
    fn test_pubkey_fromstr() {
        let hex = "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
        let pk: PubKey = hex.parse().unwrap();
        assert_eq!(pk.as_str(), hex);
    }

    #[test]
    fn test_pubkey_fromstr_invalid() {
        let err: NipError = "tooshort".parse::<PubKey>().unwrap_err();
        match err {
            NipError::InvalidPubKeyLength(len) => assert_eq!(len, 8),
            _ => panic!("Expected InvalidPubKeyLength"),
        }
    }

    #[test]
    fn test_pubkey_to_bytes() {
        let hex = "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
        let pk = PubKey::new(hex).unwrap();
        let bytes = pk.to_bytes().unwrap();
        assert_eq!(bytes.len(), 32);
        // Verify round-trip
        assert_eq!(hex::encode(bytes), hex);
    }

    #[test]
    fn test_pubkey_as_ref_str() {
        let hex = "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
        let pk = PubKey::new(hex).unwrap();
        let s: &str = pk.as_ref();
        assert_eq!(s, hex);
    }

    #[test]
    fn test_pubkey_clone_eq_hash() {
        let hex1 = "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
        let hex2 = "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
        let pk1 = PubKey::new(hex1).unwrap();
        let pk2 = PubKey::new(hex2).unwrap();
        assert_eq!(pk1, pk2);
        assert_eq!(format!("{:?}", pk1), format!("{:?}", pk2));
    }

    // ==================== EventId Tests ====================

    #[test]
    fn test_eventid_valid() {
        let hex = "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
        let eid = EventId::new(hex).unwrap();
        assert_eq!(eid.as_str(), hex);
    }

    #[test]
    fn test_eventid_invalid_length() {
        let err = EventId::new("short").unwrap_err();
        match err {
            NipError::InvalidEventIdLength(len) => assert_eq!(len, 5),
            _ => panic!("Expected InvalidEventIdLength"),
        }
    }

    #[test]
    fn test_eventid_invalid_hex() {
        let err = EventId::new("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz")
            .unwrap_err();
        match err {
            NipError::InvalidHexFormat => {}
            _ => panic!("Expected InvalidHexFormat"),
        }
    }

    #[test]
    fn test_eventid_from_bytes() {
        let bytes: [u8; 32] = [0u8; 32];
        let eid = EventId::from_bytes(&bytes);
        assert_eq!(
            eid.as_str(),
            "0000000000000000000000000000000000000000000000000000000000000000"
        );
    }

    #[test]
    fn test_eventid_display() {
        let hex = "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
        let eid = EventId::new(hex).unwrap();
        assert_eq!(format!("{}", eid), hex);
    }

    #[test]
    fn test_eventid_fromstr() {
        let hex = "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
        let eid: EventId = hex.parse().unwrap();
        assert_eq!(eid.as_str(), hex);
    }

    #[test]
    fn test_eventid_to_bytes() {
        let hex = "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
        let eid = EventId::new(hex).unwrap();
        let bytes = eid.to_bytes().unwrap();
        assert_eq!(bytes.len(), 32);
        assert_eq!(hex::encode(bytes), hex);
    }

    #[test]
    fn test_eventid_from_bytes_roundtrip() {
        let bytes: [u8; 32] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b,
            0x1c, 0x1d, 0x1e, 0x1f,
        ];
        let eid = EventId::from_bytes(&bytes);
        let decoded = eid.to_bytes().unwrap();
        assert_eq!(decoded, bytes);
    }

    // ==================== Signature Tests ====================

    #[test]
    fn test_signature_valid() {
        let hex = "abcd".repeat(32); // 128 chars
        let sig = Signature::new(&hex).unwrap();
        assert_eq!(sig.as_str(), &hex);
    }

    #[test]
    fn test_signature_invalid_length() {
        let err = Signature::new("short").unwrap_err();
        match err {
            NipError::InvalidSignatureLength(len) => assert_eq!(len, 5),
            _ => panic!("Expected InvalidSignatureLength"),
        }
    }

    #[test]
    fn test_signature_from_bytes() {
        let bytes: [u8; 64] = [0xab; 64];
        let sig = Signature::from_bytes(&bytes);
        let expected = hex::encode(bytes);
        assert_eq!(sig.as_str(), expected);
    }

    #[test]
    fn test_signature_to_bytes() {
        let hex = "abcd".repeat(32);
        let sig = Signature::new(&hex).unwrap();
        let bytes = sig.to_bytes().unwrap();
        assert_eq!(bytes.len(), 64);
        assert_eq!(hex::encode(bytes), hex);
    }

    #[test]
    fn test_signature_from_bytes_roundtrip() {
        let bytes: [u8; 64] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b,
            0x1c, 0x1d, 0x1e, 0x1f, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29,
            0x2a, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37,
            0x38, 0x39, 0x3a, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f,
        ];
        let sig = Signature::from_bytes(&bytes);
        let decoded = sig.to_bytes().unwrap();
        assert_eq!(decoded, bytes);
    }

    // ==================== Kind Tests ====================

    #[test]
    fn test_kind_constants() {
        assert_eq!(Kind::METADATA.value(), 0);
        assert_eq!(Kind::TEXT_NOTE.value(), 1);
        assert_eq!(Kind::DELETION.value(), 5);
        assert_eq!(Kind::SET_STALL.value(), 30017);
        assert_eq!(Kind::SET_PRODUCT.value(), 30018);
        assert_eq!(Kind::DATE_EVENT.value(), 31922);
        assert_eq!(Kind::TIME_EVENT.value(), 31923);
        assert_eq!(Kind::CALENDAR.value(), 31924);
        assert_eq!(Kind::RSVP.value(), 31925);
    }

    #[test]
    fn test_kind_display() {
        assert_eq!(format!("{}", Kind::TEXT_NOTE), "1");
        assert_eq!(format!("{}", Kind::SET_STALL), "30017");
    }

    #[test]
    fn test_kind_new() {
        let k = Kind::new(42);
        assert_eq!(k.value(), 42);
    }

    #[test]
    fn test_kind_is_regular() {
        // Kind 1 is regular
        assert!(Kind::TEXT_NOTE.is_regular());
        // Kind 2 is regular
        assert!(Kind::RECOMMEND_RELAY.is_regular());
        // Kinds 4-44 are regular
        assert!(Kind::DM.is_regular());
        assert!(Kind::REACTION.is_regular());
        assert!(Kind::new(44).is_regular());
        // Kinds 1000-9999 are regular
        assert!(Kind::new(1000).is_regular());
        assert!(Kind::new(9999).is_regular());
        // Kind 0 is NOT regular
        assert!(!Kind::METADATA.is_regular());
        // Kind 3 is NOT regular
        assert!(!Kind::CONTACT_LIST.is_regular());
        // Kind 30000 is NOT regular
        assert!(!Kind::new(30000).is_regular());
    }

    #[test]
    fn test_kind_is_replaceable() {
        // Kind 0 is replaceable
        assert!(Kind::METADATA.is_replaceable());
        // Kind 3 is replaceable
        assert!(Kind::CONTACT_LIST.is_replaceable());
        // Kinds 10000-19999 are replaceable
        assert!(Kind::new(10000).is_replaceable());
        assert!(Kind::new(19999).is_replaceable());
        // Kind 1 is NOT replaceable
        assert!(!Kind::TEXT_NOTE.is_replaceable());
        // Kind 30000 is NOT replaceable
        assert!(!Kind::new(30000).is_replaceable());
    }

    #[test]
    fn test_kind_is_ephemeral() {
        // Kinds 20000-29999 are ephemeral
        assert!(Kind::new(20000).is_ephemeral());
        assert!(Kind::new(25000).is_ephemeral());
        assert!(Kind::new(29999).is_ephemeral());
        // Kind 0 is NOT ephemeral
        assert!(!Kind::METADATA.is_ephemeral());
        // Kind 30000 is NOT ephemeral
        assert!(!Kind::new(30000).is_ephemeral());
    }

    #[test]
    fn test_kind_is_addressable() {
        // Kinds 30000-39999 are addressable
        assert!(Kind::SET_STALL.is_addressable());
        assert!(Kind::SET_PRODUCT.is_addressable());
        assert!(Kind::DATE_EVENT.is_addressable());
        assert!(Kind::TIME_EVENT.is_addressable());
        assert!(Kind::CALENDAR.is_addressable());
        assert!(Kind::RSVP.is_addressable());
        assert!(Kind::new(30000).is_addressable());
        assert!(Kind::new(39999).is_addressable());
        // Kind 0 is NOT addressable
        assert!(!Kind::METADATA.is_addressable());
        // Kind 20000 is NOT addressable
        assert!(!Kind::new(20000).is_addressable());
    }

    #[test]
    fn test_kind_from_u16() {
        let k: Kind = 42u16.into();
        assert_eq!(k.value(), 42);
    }

    #[test]
    fn test_kind_into_u16() {
        let v: u16 = Kind::TEXT_NOTE.into();
        assert_eq!(v, 1);
    }

    #[test]
    fn test_kind_clone_copy_eq_ord() {
        let k1 = Kind::new(5);
        let k2 = Kind::new(5);
        let k3 = Kind::new(10);
        assert_eq!(k1, k2);
        assert!(k1 < k3);
        assert!(k3 > k1);
    }

    // ==================== Tag Tests ====================

    #[test]
    fn test_tag_new() {
        let t = Tag::new(vec!["a".into(), "b".into()]);
        assert_eq!(t.name(), Some("a"));
        assert_eq!(t.value(), Some("b"));
    }

    #[test]
    fn test_tag_empty() {
        let t = Tag::empty();
        assert_eq!(t.name(), None);
        assert_eq!(t.value(), None);
    }

    #[test]
    fn test_tag_e() {
        let t = Tag::e("event123");
        assert_eq!(t.name(), Some("e"));
        assert_eq!(t.value(), Some("event123"));
        assert!(t.additional().is_empty());
    }

    #[test]
    fn test_tag_e_with_relay() {
        let t = Tag::e_with_relay("event123", "wss://relay.example.com");
        assert_eq!(t.name(), Some("e"));
        assert_eq!(t.value(), Some("event123"));
        assert_eq!(t.additional(), &["wss://relay.example.com".to_string()]);
    }

    #[test]
    fn test_tag_e_full() {
        let t = Tag::e_full("event123", "wss://relay.example.com", "pubkey123");
        assert_eq!(t.name(), Some("e"));
        assert_eq!(t.value(), Some("event123"));
        assert_eq!(t.additional().len(), 2);
        assert_eq!(t.additional()[0], "wss://relay.example.com");
        assert_eq!(t.additional()[1], "pubkey123");
    }

    #[test]
    fn test_tag_p() {
        let t = Tag::p("pubkey123");
        assert_eq!(t.name(), Some("p"));
        assert_eq!(t.value(), Some("pubkey123"));
    }

    #[test]
    fn test_tag_p_with_relay() {
        let t = Tag::p_with_relay("pubkey123", "wss://relay.example.com");
        assert_eq!(t.name(), Some("p"));
        assert_eq!(t.value(), Some("pubkey123"));
        assert_eq!(t.additional(), &["wss://relay.example.com".to_string()]);
    }

    #[test]
    fn test_tag_a() {
        let t = Tag::a(30017, "pubkey123", "stall1");
        assert_eq!(t.name(), Some("a"));
        assert_eq!(t.value(), Some("30017:pubkey123:stall1"));
    }

    #[test]
    fn test_tag_a_with_relay() {
        let t = Tag::a_with_relay(30017, "pubkey123", "stall1", "wss://relay.example.com");
        assert_eq!(t.name(), Some("a"));
        assert_eq!(t.value(), Some("30017:pubkey123:stall1"));
        assert_eq!(t.additional(), &["wss://relay.example.com".to_string()]);
    }

    #[test]
    fn test_tag_d() {
        let t = Tag::d("my-event");
        assert_eq!(t.name(), Some("d"));
        assert_eq!(t.value(), Some("my-event"));
    }

    #[test]
    fn test_tag_t() {
        let t = Tag::t("nostr");
        assert_eq!(t.name(), Some("t"));
        assert_eq!(t.value(), Some("nostr"));
    }

    #[test]
    fn test_tag_alt() {
        let t = Tag::alt("alternative description");
        assert_eq!(t.name(), Some("alt"));
        assert_eq!(t.value(), Some("alternative description"));
    }

    #[test]
    fn test_tag_g() {
        let t = Tag::g("geohash123");
        assert_eq!(t.name(), Some("g"));
        assert_eq!(t.value(), Some("geohash123"));
    }

    #[test]
    fn test_tag_r() {
        let t = Tag::r("https://example.com");
        assert_eq!(t.name(), Some("r"));
        assert_eq!(t.value(), Some("https://example.com"));
    }

    #[test]
    fn test_tag_into_inner() {
        let items = vec!["e".into(), "event123".into()];
        let t = Tag::new(items.clone());
        assert_eq!(t.into_inner(), items);
    }

    #[test]
    fn test_tag_as_slice() {
        let items = vec!["e".into(), "event123".into()];
        let t = Tag::new(items.clone());
        assert_eq!(t.as_slice(), items.as_slice());
    }

    #[test]
    fn test_tag_name_none() {
        let t = Tag::empty();
        assert_eq!(t.name(), None);
    }

    #[test]
    fn test_tag_value_none() {
        let t = Tag::new(vec!["only_name".into()]);
        assert_eq!(t.value(), None);
    }

    #[test]
    fn test_tag_additional_empty() {
        let t = Tag::new(vec!["e".into(), "event123".into()]);
        assert!(t.additional().is_empty());
    }

    #[test]
    fn test_tag_from_vec_string() {
        let items = vec!["e".into(), "event123".into()];
        let t: Tag = items.clone().into();
        assert_eq!(t.as_slice(), items.as_slice());
    }

    #[test]
    fn test_tag_into_vec_string() {
        let t = Tag::e("event123");
        let v: Vec<String> = t.into();
        assert_eq!(v, vec!["e".to_string(), "event123".to_string()]);
    }

    #[test]
    fn test_tag_as_ref_slice() {
        let t = Tag::e("event123");
        let s: &[String] = t.as_ref();
        assert_eq!(s, &["e".to_string(), "event123".to_string()]);
    }

    // ==================== RelayUrl Tests ====================

    #[test]
    fn test_relay_url_wss_valid() {
        let url = RelayUrl::new("wss://relay.example.com").unwrap();
        assert_eq!(url.as_str(), "wss://relay.example.com");
    }

    #[test]
    fn test_relay_url_ws_valid() {
        let url = RelayUrl::new("ws://localhost:8080").unwrap();
        assert_eq!(url.as_str(), "ws://localhost:8080");
    }

    #[test]
    fn test_relay_url_invalid() {
        let err = RelayUrl::new("https://example.com").unwrap_err();
        match err {
            NipError::InvalidRelayUrl(url) => assert_eq!(url, "https://example.com"),
            _ => panic!("Expected InvalidRelayUrl"),
        }
    }

    #[test]
    fn test_relay_url_display() {
        let url = RelayUrl::new("wss://relay.example.com").unwrap();
        assert_eq!(format!("{}", url), "wss://relay.example.com");
    }

    #[test]
    fn test_relay_url_clone_eq_hash() {
        let url1 = RelayUrl::new("wss://relay.example.com").unwrap();
        let url2 = RelayUrl::new("wss://relay.example.com").unwrap();
        assert_eq!(url1, url2);
    }
}
