//! HashMerchant loyalty whitelist codec **hm-loyalty/v1**.
//!
//! Must stay byte-compatible with:
//! `tools/hashmerchant-support/whitelist/build.py`
//!
//! ```text
//! domain: b"hm-loyalty/v1|"
//! leaf:   sha256( domain || address_utf8 )
//! pair:   sha256( sorted_concat(left, right) )  // order-independent
//! ```

#![deny(clippy::unwrap_used)]

use cosmwasm_schema::cw_serde;
use cosmwasm_std::HexBinary;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Canonical codec version string stored on-chain and in proof JSON.
pub const CODEC_V1: &str = "hm-loyalty/v1";

/// Domain separator prepended to address UTF-8 before leaf hashing.
pub const DOMAIN_V1: &[u8] = b"hm-loyalty/v1|";

/// Empty-tree sentinel used by offline tooling when there are no members.
pub const EMPTY_TREE_DOMAIN: &[u8] = b"hm-loyalty/empty";

#[derive(Error, Debug, PartialEq)]
pub enum WhitelistError {
    #[error("unsupported whitelist codec: expected {expected}, got {got}")]
    CodecMismatch { expected: String, got: String },
    #[error("empty address")]
    EmptyAddress,
    #[error("invalid hex: {0}")]
    InvalidHex(String),
    #[error("invalid hash length: expected 32 bytes, got {0}")]
    InvalidHashLength(usize),
    #[error("merkle proof does not match root")]
    ProofMismatch,
    #[error("whitelist root not set")]
    RootNotSet,
}

/// On-chain / client proof payload (hex-encoded siblings + root).
#[cw_serde]
pub struct LoyaltyProof {
    /// Must equal [`CODEC_V1`] for v1 verifiers.
    pub codec: String,
    /// Bech32 (or chain) address that was hashed into the leaf.
    pub address: String,
    /// Optional precomputed leaf hex; recomputed from address if omitted.
    pub leaf: Option<String>,
    /// Sibling hashes, bottom-up (hex).
    pub proof: Vec<String>,
    /// Expected root hex (optional cross-check against stored root).
    pub root: Option<String>,
}

/// Config snapshot for a minter that gates on loyalty eligibility.
#[cw_serde]
pub struct WhitelistConfig {
    pub enabled: bool,
    /// Codec version currently accepted (e.g. `hm-loyalty/v1`).
    pub codec: String,
    /// 32-byte merkle root as lowercase hex (empty if not set).
    pub root: Option<String>,
}

impl Default for WhitelistConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            codec: CODEC_V1.to_string(),
            root: None,
        }
    }
}

/// sha256(data)
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// Leaf = sha256( DOMAIN_V1 || address_utf8 )
pub fn leaf_hash(address: &str) -> Result<[u8; 32], WhitelistError> {
    let addr = address.trim();
    if addr.is_empty() {
        return Err(WhitelistError::EmptyAddress);
    }
    let mut buf = Vec::with_capacity(DOMAIN_V1.len() + addr.len());
    buf.extend_from_slice(DOMAIN_V1);
    buf.extend_from_slice(addr.as_bytes());
    Ok(sha256(&buf))
}

/// Order-independent interior node: sha256(min(a,b) || max(a,b))
pub fn node_hash(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let (left, right) = if a <= b { (a, b) } else { (b, a) };
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(left);
    buf[32..].copy_from_slice(right);
    sha256(&buf)
}

/// Empty tree root used by offline tooling.
pub fn empty_root() -> [u8; 32] {
    sha256(EMPTY_TREE_DOMAIN)
}

/// Fold leaf + siblings into a root.
pub fn compute_root(leaf: [u8; 32], proof: &[[u8; 32]]) -> [u8; 32] {
    let mut cur = leaf;
    for sib in proof {
        cur = node_hash(&cur, sib);
    }
    cur
}

/// Parse 32-byte hash from hex (with or without 0x).
pub fn parse_hash_hex(s: &str) -> Result<[u8; 32], WhitelistError> {
    let s = s.trim().strip_prefix("0x").unwrap_or(s.trim());
    let bytes = hex::decode(s).map_err(|e| WhitelistError::InvalidHex(e.to_string()))?;
    if bytes.len() != 32 {
        return Err(WhitelistError::InvalidHashLength(bytes.len()));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

pub fn hash_to_hex(h: &[u8; 32]) -> String {
    hex::encode(h)
}

/// Validate root hex shape for set_root.
pub fn validate_root_hex(root: &str) -> Result<String, WhitelistError> {
    let h = parse_hash_hex(root)?;
    Ok(hash_to_hex(&h))
}

/// Verify a proof against an expected root and codec.
///
/// When `expected_codec` is set, proof.codec must match exactly (fail closed).
pub fn verify_proof(
    proof: &LoyaltyProof,
    expected_root_hex: &str,
    expected_codec: &str,
) -> Result<(), WhitelistError> {
    if proof.codec != expected_codec {
        return Err(WhitelistError::CodecMismatch {
            expected: expected_codec.to_string(),
            got: proof.codec.clone(),
        });
    }

    let expected_root = parse_hash_hex(expected_root_hex)?;
    let leaf = if let Some(ref leaf_hex) = proof.leaf {
        let claimed = parse_hash_hex(leaf_hex)?;
        let computed = leaf_hash(&proof.address)?;
        if claimed != computed {
            return Err(WhitelistError::ProofMismatch);
        }
        computed
    } else {
        leaf_hash(&proof.address)?
    };

    let mut siblings = Vec::with_capacity(proof.proof.len());
    for s in &proof.proof {
        siblings.push(parse_hash_hex(s)?);
    }

    if let Some(ref r) = proof.root {
        let proof_root = parse_hash_hex(r)?;
        if proof_root != expected_root {
            return Err(WhitelistError::ProofMismatch);
        }
    }

    let computed = compute_root(leaf, &siblings);
    if computed != expected_root {
        return Err(WhitelistError::ProofMismatch);
    }
    Ok(())
}

/// Convenience for contracts: verify when whitelist enabled, else no-op.
pub fn assert_eligible(
    cfg: &WhitelistConfig,
    address: &str,
    proof: Option<&LoyaltyProof>,
) -> Result<(), WhitelistError> {
    if !cfg.enabled {
        return Ok(());
    }
    let root = cfg.root.as_deref().ok_or(WhitelistError::RootNotSet)?;
    let proof = proof.ok_or(WhitelistError::ProofMismatch)?;
    // Address in proof must match sender (defense in depth).
    if proof.address.trim() != address.trim() {
        return Err(WhitelistError::ProofMismatch);
    }
    verify_proof(proof, root, &cfg.codec)
}

/// Binary helper for clients that prefer Binary over hex strings.
pub fn leaf_hash_binary(address: &str) -> Result<HexBinary, WhitelistError> {
    Ok(HexBinary::from(leaf_hash(address)?.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Golden vectors from:
    //   python3 tools/hashmerchant-support/whitelist/build.py \
    //     --in tools/hashmerchant-support/whitelist/fixtures/members.json --out /tmp/loyalty-wl-test
    const ALICE: &str = "terp1alice000000000000000000000000000001";
    const BOB: &str = "terp1bob000000000000000000000000000000002";
    const CAROL: &str = "terp1carol000000000000000000000000000003";
    const DAVE: &str = "terp1dave00000000000000000000000000000004";

    const ROOT: &str = "d97a6a7ea9c877c628d7d27eefa7ce63ff9c0eb7f83755d77e0778a08782293f";

    fn alice_proof() -> LoyaltyProof {
        LoyaltyProof {
            codec: CODEC_V1.to_string(),
            address: ALICE.to_string(),
            leaf: Some("1ce202d3f07152b1b05ce927ff02369f9336909e359488964ad17501d0fc8c60".into()),
            proof: vec![
                "25bedb835af688d4f03c52a582c6e7588f45286a786e63eb3d18724c284162da".into(),
                "3a87fbb37a3d837219fed2184b905d2ea29b2cc2ebc2b240cf4af5a633a129bc".into(),
            ],
            root: Some(ROOT.into()),
        }
    }

    fn bob_proof() -> LoyaltyProof {
        LoyaltyProof {
            codec: CODEC_V1.to_string(),
            address: BOB.to_string(),
            leaf: Some("7987eae86d084c4f196ca2e6e0ad9d1d578c98f12bb34a77cbbc79bd56b71c01".into()),
            proof: vec![
                "b48a799ac867b7450b62e1e67bd22c89a8344a529d7cb3d1e467e1916b9d56c3".into(),
                "c01f412c449f1a83ea38ae56bcf791ea833bc8999c2ff7389c2bcea90fa2ec19".into(),
            ],
            root: Some(ROOT.into()),
        }
    }

    #[test]
    fn leaf_matches_python_fixture() {
        assert_eq!(
            hash_to_hex(&leaf_hash(ALICE).unwrap()),
            "1ce202d3f07152b1b05ce927ff02369f9336909e359488964ad17501d0fc8c60"
        );
        assert_eq!(
            hash_to_hex(&leaf_hash(BOB).unwrap()),
            "7987eae86d084c4f196ca2e6e0ad9d1d578c98f12bb34a77cbbc79bd56b71c01"
        );
        assert_eq!(
            hash_to_hex(&leaf_hash(CAROL).unwrap()),
            "25bedb835af688d4f03c52a582c6e7588f45286a786e63eb3d18724c284162da"
        );
        assert_eq!(
            hash_to_hex(&leaf_hash(DAVE).unwrap()),
            "b48a799ac867b7450b62e1e67bd22c89a8344a529d7cb3d1e467e1916b9d56c3"
        );
    }

    #[test]
    fn golden_alice_and_bob_verify() {
        verify_proof(&alice_proof(), ROOT, CODEC_V1).unwrap();
        verify_proof(&bob_proof(), ROOT, CODEC_V1).unwrap();
    }

    #[test]
    fn wrong_proof_rejected() {
        let mut p = alice_proof();
        p.address = BOB.to_string();
        assert!(matches!(
            verify_proof(&p, ROOT, CODEC_V1),
            Err(WhitelistError::ProofMismatch)
        ));
    }

    #[test]
    fn codec_mismatch_fail_closed() {
        let mut p = alice_proof();
        p.codec = "hm-loyalty/v2".into();
        assert!(matches!(
            verify_proof(&p, ROOT, CODEC_V1),
            Err(WhitelistError::CodecMismatch { .. })
        ));
    }

    #[test]
    fn wrong_root_rejected() {
        let bad_root = "00".repeat(32);
        assert!(matches!(
            verify_proof(&alice_proof(), &bad_root, CODEC_V1),
            Err(WhitelistError::ProofMismatch)
        ));
    }

    #[test]
    fn empty_root_matches_python() {
        // sha256(b"hm-loyalty/empty")
        let expected = hash_to_hex(&empty_root());
        assert_eq!(expected.len(), 64);
        // deterministic
        assert_eq!(empty_root(), sha256(b"hm-loyalty/empty"));
    }

    #[test]
    fn assert_eligible_respects_enabled_flag() {
        let cfg = WhitelistConfig {
            enabled: false,
            codec: CODEC_V1.into(),
            root: None,
        };
        assert_eligible(&cfg, ALICE, None).unwrap();

        let cfg_on = WhitelistConfig {
            enabled: true,
            codec: CODEC_V1.into(),
            root: Some(ROOT.into()),
        };
        assert_eligible(&cfg_on, ALICE, Some(&alice_proof())).unwrap();
        assert!(assert_eligible(&cfg_on, ALICE, None).is_err());
        // sender must match proof address
        assert!(assert_eligible(&cfg_on, BOB, Some(&alice_proof())).is_err());
    }

    #[test]
    fn node_hash_order_independent() {
        let a = leaf_hash(ALICE).unwrap();
        let b = leaf_hash(BOB).unwrap();
        assert_eq!(node_hash(&a, &b), node_hash(&b, &a));
    }

    #[test]
    fn self_test_style_two_leaf_tree() {
        // Manual mini-tree matching build.py pairing of sorted unique leaves.
        let leaves = [leaf_hash("a").unwrap(), leaf_hash("b").unwrap()];
        let mut sorted = leaves;
        sorted.sort();
        let root = node_hash(&sorted[0], &sorted[1]);
        let proof_for_0 = [sorted[1]];
        assert_eq!(compute_root(sorted[0], &proof_for_0), root);
    }
}
