//! Headstash note `addr` helpers for `PUT /notes/{hs_id}/{addr}`.
//!
//! Charset (hash-market `validate_id`): `[a-zA-Z0-9_.-]{1,200}`.
//!
//! Conventions:
//! - **claim** — bech32 or `0x`+40 hex (caller-supplied; validated only)
//! - **bridge cm** — `cm.` + lowercase hex of 32-byte commitment
//! - **bridge pk** — `pk.` + lowercase hex of diversifier/pk_d bytes

use super::NotePersistError;

/// Max length accepted by hash-market note path segments.
pub use crate::domain::NOTE_ID_MAX_LEN;

/// Validate `hs_id` or `addr` path segment: `[a-zA-Z0-9_.-]{1,200}`.
pub fn validate_id(id: &str) -> Result<(), NotePersistError> {
    if id.is_empty() || id.len() > NOTE_ID_MAX_LEN {
        return Err(NotePersistError::InvalidId);
    }
    if !id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
    {
        return Err(NotePersistError::InvalidId);
    }
    Ok(())
}

/// Claim-path addr: validate and return owned string (no prefix rewrite).
///
/// Expected forms: bech32 (e.g. `terp1…`) or EVM-style `0x`+40 hex.
pub fn note_addr_claim(addr: &str) -> Result<String, NotePersistError> {
    validate_id(addr)?;
    Ok(addr.to_string())
}

/// Bridge/note commitment addr: `cm.` + lowercase hex(`cm`).
pub fn note_addr_cm(cm: &[u8; 32]) -> String {
    format!("cm.{}", hex::encode(cm))
}

/// Diversifier / pk_d addr: `pk.` + lowercase hex(`pk_d`).
///
/// `pk_d` may be any non-empty byte slice; result is validated against charset length.
pub fn note_addr_pk(pk_d: &[u8]) -> Result<String, NotePersistError> {
    if pk_d.is_empty() {
        return Err(NotePersistError::InvalidId);
    }
    let s = format!("pk.{}", hex::encode(pk_d));
    validate_id(&s)?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_id_accepts_common_forms() {
        validate_id("season-1").unwrap();
        validate_id("terp1abcdefghijklmnopqrstuvwxyz012345").unwrap();
        validate_id("0x0123456789abcdef0123456789abcdef01234567").unwrap();
        validate_id(&note_addr_cm(&[0xAB; 32])).unwrap();
        assert_eq!(
            note_addr_cm(&[0x0A; 32]),
            format!("cm.{}", "0a".repeat(32))
        );
        assert_eq!(
            note_addr_pk(&[0xDE, 0xAD]).unwrap(),
            "pk.dead"
        );
    }

    #[test]
    fn validate_id_rejects_bad() {
        assert!(validate_id("").is_err());
        assert!(validate_id(&"a".repeat(201)).is_err());
        assert!(validate_id("has space").is_err());
        assert!(validate_id("slash/nope").is_err());
        assert!(note_addr_pk(&[]).is_err());
    }

    #[test]
    fn claim_addr_passthrough() {
        let a = note_addr_claim("terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqsr2pd5").unwrap();
        assert_eq!(a, "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqsr2pd5");
    }
}
