//! Frozen content-id encoding for the dual-index plane (calendar, Nostr, BUD).
//!
//! # Invariants (Phase A — do not change without a versioned migration)
//!
//! 1. **Primary key** is always BUD **raw-byte SHA-256** as lowercase hex (64 chars).
//! 2. **Secondary** is optional IPFS CID (CIDv1 preferred from Kubo).
//! 3. Wire / chain `MetadataExt.cid` for off-chain bodies SHOULD use the
//!    **canonical forms** below so resolvers need no guesswork.
//!
//! ## Canonical `cid` strings (prefer order)
//!
//! | Form | Example | Resolve |
//! |------|---------|---------|
//! | **Bare sha256 hex** (preferred for chain storage) | `a1b2…64hex` | `GET /content/{sha256}` |
//! | Prefixed BUD | `bud:a1b2…` or `sha256:a1b2…` | strip prefix → content plane |
//! | Prefixed IPFS | `ipfs:bafy…` or `ipfs://bafy…` | Kubo/gateway; dual-index may map to sha256 |
//!
//! Do **not** store bare CIDs without `ipfs:` prefix in new code — bare 64-hex
//! is reserved for BUD sha256. (CIDv0 `Qm…` is 46 chars; CIDv1 `bafy…` is not hex.)
//!
//! Calendar off-chain: prefer bare sha256 in `MetadataExt.cid` after POST /blobs.

use serde::{Deserialize, Serialize};

/// Kind of identifier after parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CidKind {
    /// BUD raw-byte SHA-256 (primary)
    BudSha256,
    /// IPFS content id (secondary transport)
    Ipfs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedCid {
    pub kind: CidKind,
    /// Lowercase hex sha256 **or** IPFS CID string (as provided after prefix strip)
    pub value: String,
}

/// Normalize and classify a `cid` string from chain / Nostr / API.
pub fn parse_cid(raw: &str) -> Result<ParsedCid, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("empty cid".into());
    }

    // Prefixed forms
    let lower = s.to_ascii_lowercase();
    if let Some(rest) = lower
        .strip_prefix("bud:")
        .or_else(|| lower.strip_prefix("sha256:"))
    {
        return Ok(ParsedCid {
            kind: CidKind::BudSha256,
            value: normalize_sha256_hex(rest)?,
        });
    }
    if let Some(rest) = lower
        .strip_prefix("ipfs://")
        .or_else(|| lower.strip_prefix("ipfs:"))
    {
        let v = rest.trim().trim_start_matches('/');
        if v.is_empty() {
            return Err("empty ipfs cid".into());
        }
        return Ok(ParsedCid {
            kind: CidKind::Ipfs,
            value: v.to_string(),
        });
    }

    // Bare: 64 hex → BUD sha256; else treat as IPFS CID (Qm… / bafy…)
    if is_sha256_hex(s) {
        return Ok(ParsedCid {
            kind: CidKind::BudSha256,
            value: s.to_ascii_lowercase(),
        });
    }
    if looks_like_ipfs_cid(s) {
        return Ok(ParsedCid {
            kind: CidKind::Ipfs,
            value: s.to_string(),
        });
    }
    Err(format!(
        "unrecognized cid (want 64-hex sha256, bud:/sha256:/ipfs: prefix, or IPFS CID): {s}"
    ))
}

/// Encode for chain storage (prefer bare sha256).
pub fn encode_for_chain(kind: CidKind, value: &str) -> Result<String, String> {
    match kind {
        CidKind::BudSha256 => normalize_sha256_hex(value),
        CidKind::Ipfs => {
            if value.is_empty() {
                return Err("empty ipfs".into());
            }
            // Prefer explicit prefix when storing IPFS on chain so resolvers don't mis-read
            if value.starts_with("ipfs:") || value.starts_with("ipfs://") {
                Ok(value.to_string())
            } else {
                Ok(format!("ipfs:{value}"))
            }
        }
    }
}

/// Resolve path on hash-market content plane (relative URL path).
pub fn content_path(parsed: &ParsedCid) -> String {
    match parsed.kind {
        CidKind::BudSha256 => format!("/content/{}", parsed.value),
        CidKind::Ipfs => format!("/content/by-ipfs/{}", parsed.value),
    }
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn normalize_sha256_hex(s: &str) -> Result<String, String> {
    let s = s.trim().trim_start_matches("0x");
    if !is_sha256_hex(s) {
        return Err(format!("invalid sha256 hex (need 64 hex chars): {s}"));
    }
    Ok(s.to_ascii_lowercase())
}

fn looks_like_ipfs_cid(s: &str) -> bool {
    // CIDv0
    if s.starts_with("Qm") && s.len() >= 46 {
        return true;
    }
    // CIDv1 base32
    if s.starts_with("bafy") || s.starts_with("bafk") || s.starts_with("bafz") {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_sha256_preferred() {
        let h = "a".repeat(64);
        let p = parse_cid(&h).unwrap();
        assert_eq!(p.kind, CidKind::BudSha256);
        assert_eq!(encode_for_chain(CidKind::BudSha256, &h).unwrap(), h);
        assert_eq!(content_path(&p), format!("/content/{h}"));
    }

    #[test]
    fn prefixes() {
        let h = "ab".repeat(32);
        assert_eq!(parse_cid(&format!("bud:{h}")).unwrap().value, h);
        assert_eq!(
            parse_cid(&format!("sha256:{h}")).unwrap().kind,
            CidKind::BudSha256
        );
        let p = parse_cid("ipfs:bafybeigdyrzt").unwrap();
        assert_eq!(p.kind, CidKind::Ipfs);
        assert!(encode_for_chain(CidKind::Ipfs, "bafybeigdyrzt")
            .unwrap()
            .starts_with("ipfs:"));
    }

    #[test]
    fn reject_garbage() {
        assert!(parse_cid("not-a-cid").is_err());
        assert!(parse_cid("").is_err());
    }
}
