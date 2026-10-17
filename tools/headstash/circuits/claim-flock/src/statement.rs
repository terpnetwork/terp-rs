//! The message a holder signs for a headstash claim.
//!
//! The body is the campaign context, the allocation root, the nullifier,
//! the bech32 destination, and the decimal amount. MetaMask applies the
//! EIP-191 prefix on top. The Ligerito circuit publishes those words. It
//! does not yet contain a secp256k1 gate, so the signature check stays next
//! to the proof.

use headstash_wire::recover_pubkey_x;

pub const CANONICAL_PREFIX: &str = "headstash-claim-v1:";

/// Little-endian packing of eight `u32` words into the 32 bytes the contract
/// stores. This is the same order as `sidecar::words_to_bytes`.
pub fn pack_words(words: &[u32; 8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, word) in words.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&word.to_le_bytes());
    }
    out
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Bech32 (BIP-173) of the 20-byte destination. Each word is little-endian.
pub fn destination_bech32(hrp: &str, words: &[u32; 5]) -> Result<String, ()> {
    let mut raw = [0u8; 20];
    for (i, word) in words.iter().enumerate() {
        raw[4 * i..4 * i + 4].copy_from_slice(&word.to_le_bytes());
    }
    let hrp = bech32::Hrp::parse(hrp).map_err(|_| ())?;
    bech32::encode::<bech32::Bech32>(hrp, &raw).map_err(|_| ())
}

/// UTF-8 body the holder passes to `personal_sign`.
pub fn canonical_sign_message(
    ctx: &[u8; 32],
    root: &[u8; 32],
    nullifier: &[u8; 32],
    destination: &str,
    amount: u64,
) -> Vec<u8> {
    format!(
        "{CANONICAL_PREFIX}{}:{}:{}:{destination}:{amount}",
        hex_bytes(ctx),
        hex_bytes(root),
        hex_bytes(nullifier),
    )
    .into_bytes()
}

/// Canonical body for a public [`crate::Claim`] under `hrp`.
pub fn canonical_claim_message(claim: &crate::Claim, hrp: &str) -> Result<Vec<u8>, ()> {
    let destination = destination_bech32(hrp, &claim.destination)?;
    Ok(canonical_sign_message(
        &pack_words(&claim.ctx),
        &pack_words(&claim.root),
        &pack_words(&claim.nullifier),
        &destination,
        u64::from(claim.amount),
    ))
}

/// The signature recovers `leaf_x` for this message.
pub fn signature_opens_leaf(message: &[u8], signature: &[u8], leaf_x: &[u8; 32]) -> bool {
    recover_pubkey_x(message, signature).ok().as_ref() == Some(leaf_x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use headstash_wire::sign_personal;

    #[test]
    fn the_body_is_the_five_fields_in_order() {
        let ctx = [0xab; 32];
        let root = [0x11; 32];
        let nullifier = [0x22; 32];
        let message = canonical_sign_message(&ctx, &root, &nullifier, "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq0g3g2k", 15);
        let text = String::from_utf8(message.clone()).unwrap();
        assert!(text.starts_with("headstash-claim-v1:"));
        assert!(text.ends_with(":terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq0g3g2k:15"));
        assert_eq!(text.matches(':').count(), 5);
        assert!(!text.contains("headstash-private-claim-v1"));
    }

    #[test]
    fn signature_is_bound_to_the_canonical_body() {
        let secret = [7u8; 32];
        let ctx = [1u8; 32];
        let root = [2u8; 32];
        let nullifier = [3u8; 32];
        let message = canonical_sign_message(&ctx, &root, &nullifier, "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq0g3g2k", 15);
        let signature = sign_personal(&secret, &message).expect("sign");
        let leaf_x = recover_pubkey_x(&message, &signature).expect("recover");
        assert!(signature_opens_leaf(&message, &signature, &leaf_x));
        let mut other = message.clone();
        other[20] ^= 1;
        assert!(!signature_opens_leaf(&other, &signature, &leaf_x));
    }

    #[test]
    fn destination_bech32_uses_little_endian_words() {
        let words = [0x01020304, 0, 0, 0, 0];
        let text = destination_bech32("terp", &words).expect("bech32");
        assert!(text.starts_with("terp1"));
        let (hrp, raw) = bech32::decode(&text).expect("decode");
        assert_eq!(hrp.as_str(), "terp");
        assert_eq!(raw[..4], [0x04, 0x03, 0x02, 0x01]);
    }
}
