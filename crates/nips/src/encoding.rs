//! Serialization helpers for NIP-01 canonical event encoding.
//!
//! The NIP-01 specification defines a strict canonical JSON format for
//! computing event IDs. This module provides the low-level building blocks
//! for that encoding.
//!
//! Most consumers will use the derive-based serialization from serde
//! directly. These helpers are for cases where you need manual control
//! over the canonical byte representation.

use crate::error::NipResult;

/// Parse a hex-encoded Nostr event ID into its raw 32-byte form.
pub fn parse_hex_id(hex: &str) -> NipResult<[u8; 32]> {
    let mut bytes = [0u8; 32];
    hex::decode_to_slice(hex, &mut bytes)
        .map_err(Into::into)
        .map(|_| bytes)
}

/// Parse a hex-encoded Nostr public key into its raw 32-byte form.
pub fn parse_hex_pubkey(hex: &str) -> NipResult<[u8; 32]> {
    parse_hex_id(hex)
}

/// Parse a hex-encoded Nostr signature into its raw 64-byte form.
pub fn parse_hex_signature(hex: &str) -> NipResult<[u8; 64]> {
    let mut bytes = [0u8; 64];
    hex::decode_to_slice(hex, &mut bytes)
        .map_err(Into::into)
        .map(|_| bytes)
}

/// Encode raw 32 bytes as a lowercase hex string (Nostr event ID / pubkey format).
pub fn encode_hex_id(bytes: &[u8; 32]) -> String {
    hex::encode(bytes)
}

/// Encode raw 64 bytes as a lowercase hex string (Nostr signature format).
pub fn encode_hex_signature(bytes: &[u8; 64]) -> String {
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip_id() {
        let original = [0xabu8; 32];
        let hex = encode_hex_id(&original);
        assert_eq!(hex.len(), 64);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
        let decoded = parse_hex_id(&hex).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_roundtrip_signature() {
        let original = [0xcd_u8; 64];
        let hex = encode_hex_signature(&original);
        assert_eq!(hex.len(), 128);
        let decoded = parse_hex_signature(&hex).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_invalid_hex_errors() {
        assert!(parse_hex_id("xyz").is_err());
    }

    #[test]
    fn test_parse_hex_pubkey_delegates_to_id() {
        let hex = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
        let result = parse_hex_pubkey(hex).unwrap();
        assert_eq!(result.len(), 32);
    }
}
