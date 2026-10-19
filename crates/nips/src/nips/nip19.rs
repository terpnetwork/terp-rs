//! NIP-19 — bech32-encoded Nostr entities.
//!
//! Implements bech32 encoding/decoding (no external bech32 crate — pure Rust std).
//! Supports bare keys (npub, nsec, note) and TLV-encoded entities (nprofile, nevent, naddr).
//!
//! Spec: https://nips.nostr.com/19

use crate::error::{NipError, NipResult};
use hex;

// bech32 charset (BIP-0173)
const CHARSET: &[u8; 32] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";

fn char_to_value(c: u8) -> Option<u8> {
    CHARSET.iter().position(|&x| x == c).map(|p| p as u8)
}

// Generator coefficients for bech32 BCH checksum
const GEN: [u32; 5] = [0x3b6a_57b2, 0x2650_8e6d, 0x1ea1_19fa, 0x3d42_33dd, 0x2a14_62e3];

fn polymod(values: &[u8]) -> u32 {
    let mut chk = 1u32;
    for v in values {
        let top = chk >> 25;
        chk = ((chk & 0x1ff_ffff) << 5) ^ (*v as u32);
        for i in 0..5 {
            if (top >> i) & 1 == 1 {
                chk ^= GEN[i];
            }
        }
    }
    chk
}

fn hrp_expand(hrp: &str) -> Vec<u8> {
    let mut v = Vec::new();
    for &b in hrp.as_bytes() {
        v.push(b >> 5);
    }
    v.push(0);
    for &b in hrp.as_bytes() {
        v.push(b & 0x1f);
    }
    v
}

fn verify_checksum(hrp: &str, data: &[u8]) -> bool {
    polymod(&[&hrp_expand(hrp), data].concat()) == 1
}

fn create_checksum(hrp: &str, data: &[u8]) -> Vec<u8> {
    let values = [&hrp_expand(hrp), data, &[0u8; 6]].concat();
    let modval = polymod(&values) ^ 1;
    let mut checksum = Vec::with_capacity(6);
    for i in 0..6 {
        checksum.push(((modval >> (5 * (5 - i))) & 0x1f) as u8);
    }
    checksum
}

/// Convert 8-bit bytes to 5-bit groups (bech32 data).
fn bytes_to_5bit(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0;
    for &b in data {
        buffer = (buffer << 8) | (b as u32);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(((buffer >> bits) & 0x1f) as u8);
        }
    }
    if bits > 0 {
        out.push(((buffer << (5 - bits)) & 0x1f) as u8);
    }
    out
}

/// Convert 5-bit groups back to 8-bit bytes.
fn bytes_from_5bit(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0;
    for &b in data {
        buffer = (buffer << 5) | (b as u32);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    out
}

/// Encode a bech32 string from HRP and data bytes.
///
/// The data bytes are automatically converted to 5-bit groups
/// and a BCH checksum is appended.
pub fn encode_bech32(hrp: &str, data: &[u8]) -> String {
    let five_bit = bytes_to_5bit(data);
    let mut combined = five_bit.clone();
    combined.extend_from_slice(&create_checksum(hrp, &five_bit));
    let mut s = String::from(hrp);
    s.push('1');
    for &v in &combined {
        s.push(CHARSET[v as usize] as char);
    }
    s
}

/// Decode a bech32 string into HRP and data bytes.
///
/// Validates the checksum and returns the original data.
pub fn decode_bech32(s: &str) -> NipResult<(String, Vec<u8>)> {
    // Validate characters
    if s.len() > 5000 {
        return Err(NipError::Validation("bech32 string exceeds 5000 chars".into()));
    }
    if s.len() < 8 {
        return Err(NipError::Validation("bech32 string too short".into()));
    }

    // Find separator '1'
    let sep = s.rfind('1').ok_or_else(|| NipError::Validation("bech32: no separator '1'".into()))?;
    if sep < 1 || sep + 7 > s.len() {
        return Err(NipError::Validation("bech32: invalid separator position".into()));
    }

    let hrp = &s[..sep];
    let data_part = &s[sep + 1..];

    // Validate HRP
    if hrp.is_empty() || !hrp.bytes().all(|b| b.is_ascii_lowercase()) {
        return Err(NipError::Validation("bech32: invalid HRP".into()));
    }

    // Decode data
    let mut values = Vec::with_capacity(data_part.len());
    for &b in data_part.as_bytes() {
        match char_to_value(b) {
            Some(v) => values.push(v),
            None => return Err(NipError::Validation("bech32: invalid character".into())),
        }
    }

    // Verify checksum
    if !verify_checksum(hrp, &values) {
        return Err(NipError::Validation("bech32: invalid checksum".into()));
    }

    // Strip checksum (last 6 values)
    let data_len = values.len() - 6;
    let five_bit_data = &values[..data_len];
    let raw = bytes_from_5bit(five_bit_data);

    Ok((hrp.to_string(), raw))
}

// ═══════════════════════════════════════════════════════════════════════════
// NIP-19 Entity types
// ═══════════════════════════════════════════════════════════════════════════

/// A NIP-19 bech32-encoded entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nip19Entity {
    /// Public key (npub)
    Npub([u8; 32]),
    /// Private key (nsec)
    Nsec([u8; 32]),
    /// Note/event id (note)
    Note([u8; 32]),
    /// Profile with relay hints (nprofile)
    Nprofile {
        pubkey: [u8; 32],
        relays: Vec<String>,
    },
    /// Event with relay hints (nevent)
    Nevent {
        event_id: [u8; 32],
        relays: Vec<String>,
        author: Option<[u8; 32]>,
        kind: Option<u32>,
    },
    /// Addressable event coordinate (naddr)
    Naddr {
        identifier: String,
        pubkey: [u8; 32],
        kind: u32,
        relays: Vec<String>,
    },
}

impl Nip19Entity {
    /// Encode this entity to a bech32 string.
    pub fn encode(&self) -> String {
        match self {
            Self::Npub(pk) => encode_bech32("npub", pk),
            Self::Nsec(sk) => encode_bech32("nsec", sk),
            Self::Note(id) => encode_bech32("note", id),
            Self::Nprofile { pubkey, relays } => {
                let tlv = encode_tlv_profile(pubkey, relays);
                encode_bech32("nprofile", &tlv)
            }
            Self::Nevent { event_id, relays, author, kind } => {
                let tlv = encode_tlv_event(event_id, relays, author, kind);
                encode_bech32("nevent", &tlv)
            }
            Self::Naddr { identifier, pubkey, kind, relays } => {
                let tlv = encode_tlv_addr(identifier, pubkey, *kind, relays);
                encode_bech32("naddr", &tlv)
            }
        }
    }

    /// Decode a bech32 string into a Nip19Entity.
    pub fn decode(s: &str) -> NipResult<Self> {
        let (hrp, data) = decode_bech32(s)?;
        match hrp.as_str() {
            "npub" => {
                let bytes: [u8; 32] = data.try_into().map_err(|_| NipError::Validation("npub: expected 32 bytes".into()))?;
                Ok(Self::Npub(bytes))
            }
            "nsec" => {
                let bytes: [u8; 32] = data.try_into().map_err(|_| NipError::Validation("nsec: expected 32 bytes".into()))?;
                Ok(Self::Nsec(bytes))
            }
            "note" => {
                let bytes: [u8; 32] = data.try_into().map_err(|_| NipError::Validation("note: expected 32 bytes".into()))?;
                Ok(Self::Note(bytes))
            }
            "nprofile" => {
                let tlv = decode_tlv(&data)?;
                let pubkey = tlv_get_32(&tlv, 0).ok_or_else(|| NipError::Validation("nprofile: missing pubkey TLV".into()))?;
                let relays = tlv_get_all_strings(&tlv, 1);
                Ok(Self::Nprofile { pubkey, relays })
            }
            "nevent" => {
                let tlv = decode_tlv(&data)?;
                let event_id = tlv_get_32(&tlv, 0).ok_or_else(|| NipError::Validation("nevent: missing event_id TLV".into()))?;
                let relays = tlv_get_all_strings(&tlv, 1);
                let author = tlv_get_32(&tlv, 2);
                let kind = tlv_get_u32(&tlv, 3);
                Ok(Self::Nevent { event_id, relays, author, kind })
            }
            "naddr" => {
                let tlv = decode_tlv(&data)?;
                let identifier = tlv_get_string(&tlv, 0).unwrap_or_default();
                let relays = tlv_get_all_strings(&tlv, 1);
                let pubkey = tlv_get_32(&tlv, 2).ok_or_else(|| NipError::Validation("naddr: missing pubkey TLV".into()))?;
                let kind = tlv_get_u32(&tlv, 3).ok_or_else(|| NipError::Validation("naddr: missing kind TLV".into()))?;
                Ok(Self::Naddr { identifier, pubkey, kind, relays })
            }
            other => Err(NipError::Validation(format!("Unknown bech32 prefix: {}", other))),
        }
    }

    /// Convert to `nostr:` URI string.
    pub fn to_uri(&self) -> String {
        format!("nostr:{}", self.encode())
    }

    /// Get the hex representation (for bare keys/ids).
    pub fn to_hex(&self) -> Option<String> {
        match self {
            Self::Npub(pk) => Some(hex::encode(pk)),
            Self::Nsec(sk) => Some(hex::encode(sk)),
            Self::Note(id) => Some(hex::encode(id)),
            _ => None,
        }
    }

    /// Get the hex pubkey (for naddr / nprofile / nevent).
    pub fn pubkey_hex(&self) -> Option<String> {
        match self {
            Self::Npub(pk) => Some(hex::encode(pk)),
            Self::Nprofile { pubkey, .. } => Some(hex::encode(pubkey)),
            Self::Naddr { pubkey, .. } => Some(hex::encode(pubkey)),
            Self::Nevent { author, .. } => author.map(hex::encode),
            _ => None,
        }
    }
}

impl std::fmt::Display for Nip19Entity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.encode())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TLV encoding/decoding
// ═══════════════════════════════════════════════════════════════════════════

/// A single TLV entry.
#[derive(Debug, Clone)]
struct TlvEntry {
    ty: u8,
    value: Vec<u8>,
}

/// Encode TLV data for a profile (nprofile).
fn encode_tlv_profile(pubkey: &[u8; 32], relays: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    // TLV type 0: pubkey (32 bytes)
    out.push(0);
    out.push(32);
    out.extend_from_slice(pubkey);
    // TLV type 1: relays
    for relay in relays {
        let relay_bytes = relay.as_bytes();
        let len = relay_bytes.len();
        if len > 255 {
            continue; // skip oversized relays
        }
        out.push(1);
        out.push(len as u8);
        out.extend_from_slice(relay_bytes);
    }
    out
}

/// Encode TLV data for an event (nevent).
fn encode_tlv_event(event_id: &[u8; 32], relays: &[String], author: &Option<[u8; 32]>, kind: &Option<u32>) -> Vec<u8> {
    let mut out = Vec::new();
    // TLV type 0: event_id (32 bytes)
    out.push(0);
    out.push(32);
    out.extend_from_slice(event_id);
    // TLV type 1: relays
    for relay in relays {
        let relay_bytes = relay.as_bytes();
        let len = relay_bytes.len();
        if len > 255 {
            continue;
        }
        out.push(1);
        out.push(len as u8);
        out.extend_from_slice(relay_bytes);
    }
    // TLV type 2: author (optional, 32 bytes)
    if let Some(a) = author {
        out.push(2);
        out.push(32);
        out.extend_from_slice(a);
    }
    // TLV type 3: kind (optional, 4 bytes big-endian)
    if let Some(k) = kind {
        out.push(3);
        out.push(4);
        out.extend_from_slice(&k.to_be_bytes());
    }
    out
}

/// Encode TLV data for an addressable event (naddr).
fn encode_tlv_addr(identifier: &str, pubkey: &[u8; 32], kind: u32, relays: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    // TLV type 0: identifier (d-tag)
    let id_bytes = identifier.as_bytes();
    let id_len = id_bytes.len();
    if id_len > 255 {
        // Truncate to fit TLV length byte
        let truncated = &id_bytes[..255];
        out.push(0);
        out.push(255);
        out.extend_from_slice(truncated);
    } else {
        out.push(0);
        out.push(id_len as u8);
        out.extend_from_slice(id_bytes);
    }
    // TLV type 1: relays
    for relay in relays {
        let relay_bytes = relay.as_bytes();
        let len = relay_bytes.len();
        if len > 255 {
            continue;
        }
        out.push(1);
        out.push(len as u8);
        out.extend_from_slice(relay_bytes);
    }
    // TLV type 2: pubkey (32 bytes)
    out.push(2);
    out.push(32);
    out.extend_from_slice(pubkey);
    // TLV type 3: kind (4 bytes big-endian)
    out.push(3);
    out.push(4);
    out.extend_from_slice(&kind.to_be_bytes());
    out
}

/// Decode raw TLV bytes into a list of TLV entries.
fn decode_tlv(data: &[u8]) -> NipResult<Vec<TlvEntry>> {
    let mut entries = Vec::new();
    let mut i = 0;
    while i + 2 <= data.len() {
        let ty = data[i];
        let len = data[i + 1] as usize;
        i += 2;
        if i + len > data.len() {
            return Err(NipError::Validation("TLV: data truncated".into()));
        }
        entries.push(TlvEntry {
            ty,
            value: data[i..i + len].to_vec(),
        });
        i += len;
    }
    Ok(entries)
}

/// Get the first TLV entry of a given type as a 32-byte array.
fn tlv_get_32(tlvs: &[TlvEntry], ty: u8) -> Option<[u8; 32]> {
    tlvs.iter()
        .find(|e| e.ty == ty && e.value.len() == 32)
        .map(|e| {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&e.value);
            arr
        })
}

/// Get the first TLV entry of a given type as a string.
fn tlv_get_string(tlvs: &[TlvEntry], ty: u8) -> Option<String> {
    tlvs.iter()
        .find(|e| e.ty == ty && !e.value.is_empty())
        .map(|e| String::from_utf8_lossy(&e.value).to_string())
}

/// Get all TLV entries of a given type as strings.
fn tlv_get_all_strings(tlvs: &[TlvEntry], ty: u8) -> Vec<String> {
    tlvs.iter()
        .filter(|e| e.ty == ty)
        .map(|e| String::from_utf8_lossy(&e.value).to_string())
        .collect()
}

/// Get the first TLV entry of a given type as a u32 (4 bytes big-endian).
fn tlv_get_u32(tlvs: &[TlvEntry], ty: u8) -> Option<u32> {
    tlvs.iter()
        .find(|e| e.ty == ty && e.value.len() == 4)
        .map(|e| u32::from_be_bytes(e.value[..4].try_into().unwrap()))
}

// ═══════════════════════════════════════════════════════════════════════════
// Convenience constructors
// ═══════════════════════════════════════════════════════════════════════════

/// Create an npub from hex-encoded public key.
pub fn npub_from_hex(hex_key: &str) -> NipResult<Nip19Entity> {
    let mut bytes = [0u8; 32];
    hex::decode_to_slice(hex_key, &mut bytes)?;
    Ok(Nip19Entity::Npub(bytes))
}

/// Create an nsec from hex-encoded private key.
pub fn nsec_from_hex(hex_key: &str) -> NipResult<Nip19Entity> {
    let mut bytes = [0u8; 32];
    hex::decode_to_slice(hex_key, &mut bytes)?;
    Ok(Nip19Entity::Nsec(bytes))
}

/// Create a note from hex-encoded event id.
pub fn note_from_hex(hex_id: &str) -> NipResult<Nip19Entity> {
    let mut bytes = [0u8; 32];
    hex::decode_to_slice(hex_id, &mut bytes)?;
    Ok(Nip19Entity::Note(bytes))
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    /// Test vector from NIP-19 spec:
    /// pubkey 3bf0c63f... → npub180cvv07...
    const TEST_NPUB_HEX: &str = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
    // const TEST_NPUB_BECH32: &str = "npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6";

    const TEST_NSEC_HEX: &str = "67dea2ed018072d675f5415ecfaed7d2597555e202d85b3d65ea4e58d2d92ffa";
    // const TEST_NSEC_BECH32: &str = "nsec1vl029mgpspedva04g90vltkh6fvh240zqtv9k0t9af8935ke9laqsnlfe5";

    #[test]
    fn test_bech32_roundtrip() {
        let data = [0xabu8; 32];
        let encoded = encode_bech32("npub", &data);
        let (hrp, decoded) = decode_bech32(&encoded).unwrap();
        assert_eq!(hrp, "npub");
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_bech32_invalid_checksum() {
        let result = decode_bech32("npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq");
        assert!(result.is_err());
    }

    #[test]
    fn test_npub_roundtrip() {
        let entity = npub_from_hex(TEST_NPUB_HEX).unwrap();
        let encoded = entity.encode();
        // Verify roundtrip (encode → decode)
        let decoded = Nip19Entity::decode(&encoded).unwrap();
        assert_eq!(decoded, entity);
        assert_eq!(decoded.to_hex().unwrap(), TEST_NPUB_HEX);
        // Verify prefix
        assert!(encoded.starts_with("npub1"));
    }

    #[test]
    fn test_nsec_roundtrip() {
        let entity = nsec_from_hex(TEST_NSEC_HEX).unwrap();
        let encoded = entity.encode();
        // Verify roundtrip
        let decoded = Nip19Entity::decode(&encoded).unwrap();
        assert_eq!(decoded, entity);
    }

    #[test]
    fn test_note_roundtrip() {
        let data = [0x42u8; 32];
        let entity = Nip19Entity::Note(data);
        let encoded = entity.encode();
        assert!(encoded.starts_with("note1"));

        let decoded = Nip19Entity::decode(&encoded).unwrap();
        assert_eq!(decoded, entity);
    }

    #[test]
    fn test_nprofile_roundtrip() {
        let mut pubkey = [0u8; 32];
        hex::decode_to_slice(TEST_NPUB_HEX, &mut pubkey).unwrap();
        let entity = Nip19Entity::Nprofile {
            pubkey,
            relays: vec!["wss://r.x.com".into(), "wss://djbas.sadkb.com".into()],
        };
        let encoded = entity.encode();
        assert!(encoded.starts_with("nprofile1"));

        let decoded = Nip19Entity::decode(&encoded).unwrap();
        assert_eq!(decoded, entity);
    }

    #[test]
    fn test_nevent_roundtrip() {
        let event_id = [0x42u8; 32];
        let entity = Nip19Entity::Nevent {
            event_id,
            relays: vec!["wss://example.com".into()],
            author: Some([0xabu8; 32]),
            kind: Some(1),
        };
        let encoded = entity.encode();
        assert!(encoded.starts_with("nevent1"));

        let decoded = Nip19Entity::decode(&encoded).unwrap();
        assert_eq!(decoded, entity);
    }

    #[test]
    fn test_naddr_roundtrip() {
        let pubkey = [0xabu8; 32];
        let entity = Nip19Entity::Naddr {
            identifier: "test-event".into(),
            pubkey,
            kind: 30023,
            relays: vec![],
        };
        let encoded = entity.encode();
        assert!(encoded.starts_with("naddr1"));

        let decoded = Nip19Entity::decode(&encoded).unwrap();
        assert_eq!(decoded, entity);
    }

    #[test]
    fn test_to_uri() {
        let entity = npub_from_hex(TEST_NPUB_HEX).unwrap();
        let uri = entity.to_uri();
        assert!(uri.starts_with("nostr:npub1"));
    }

    #[test]
    fn test_pubkey_hex() {
        let entity = npub_from_hex(TEST_NPUB_HEX).unwrap();
        assert_eq!(entity.pubkey_hex().unwrap(), TEST_NPUB_HEX);

        let entity = Nip19Entity::Nsec([0xabu8; 32]);
        assert!(entity.pubkey_hex().is_none());
    }

    #[test]
    fn test_invalid_hrp() {
        let result = Nip19Entity::decode("nope1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq");
        assert!(result.is_err());
    }

    #[test]
    fn test_bech32_uppercase_rejected() {
        let result = decode_bech32("NPUB1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ");
        assert!(result.is_err());
    }
}