//! Circom JWT public-signal codec (v1) — policy extraction for host Path A.
//!
//! Mirrors the o-line gateway soundness pattern (`grant_is_sound` / richness):
//! - **Full** public vector → sound policy claim (nullifier + claim + principal hints)
//! - **Sparse** vector (missing nullifier / claim limbs) → **fail closed**
//!
//! ## Circom layout (stock jwt-auth / JwtVerifier.t.sol — 31 publics)
//!
//! | Index | Signal |
//! |-------|--------|
//! | 0 | `kid` |
//! | 1..2 | `iss[]` (2 field limbs) |
//! | 3 | `publicKeyHash` |
//! | 4 | `jwtNullifier` |
//! | 5 | `timestamp` |
//! | 6..25 | `maskedCommand[]` (20 limbs) |
//! | 26 | `accountSalt` |
//! | 27..29 | `azp[]` (3 limbs) |
//! | 30 | `isCodeExist` |
//!
//! **Locked product mapping (DECISIONS D2):**
//! - policy `nullifier` ← BE32(`jwtNullifier`)
//! - policy `claim_commitment` ← BE32(`accountSalt`)  // codec v1 demo binding
//!
//! Host Path A still verifies the **full** Fr vector when a real Groth16 proof is used.
//! This module only builds the **policy view** for nullifier spend + RegisterClaim.
//!
//! Inclusion root / msg_bind are **not** circuit-bound in stock jwt-auth (D3).

use cosmwasm_std::Binary;
use sha2::{Digest, Sha256};

use crate::error::ContractError;
use crate::instances::{CLAIM_COMMITMENT_LEN, NULLIFIER_LEN, build_public_inputs};

/// Codec profile id (attrs / schema negotiation).
pub const CIRCOM_JWT_CODEC_V1: &str = "circom-jwt-v1";

/// Event/schema attr for bridge alignment with o-line `schema=v1`.
pub const CLAIM_EVENT_SCHEMA_V1: &str = "v1";

/// Minimum limbs needed for policy extraction (nullifier @4 + accountSalt @26).
pub const CIRCOM_JWT_MIN_POLICY_PUBLICS: usize = 27;

/// Production jwt-auth (GCS `demo-18-12-2024`) public count — IC.len-1 = 40.
pub const CIRCOM_JWT_N_PUBLIC: usize = 40;

/// Older Solidity test verifier (`uint[31]`) — still decodable for policy if salt@26.
pub const CIRCOM_JWT_N_PUBLIC_LEGACY31: usize = 31;

pub const IDX_KID: usize = 0;
pub const IDX_ISS_START: usize = 1;
pub const IDX_ISS_END: usize = 2; // inclusive
pub const IDX_PUBLIC_KEY_HASH: usize = 3;
pub const IDX_JWT_NULLIFIER: usize = 4;
pub const IDX_TIMESTAMP: usize = 5;
pub const IDX_MASKED_CMD_START: usize = 6;
pub const IDX_MASKED_CMD_END: usize = 25;
pub const IDX_ACCOUNT_SALT: usize = 26;
pub const IDX_AZP_START: usize = 27;
pub const IDX_AZP_END: usize = 29;
/// Legacy 31-public layout: isCodeExist at end.
pub const IDX_IS_CODE_EXIST_LEGACY31: usize = 30;
/// Production 40-public layout: isCodeExist after domain limbs.
pub const IDX_IS_CODE_EXIST: usize = 39;

/// Minimum richness for a **policy claim** derived from circom publics.
/// Mirrors `MIN_SOUND_GRANT_RICHNESS` intent (fail closed on sparse).
///
/// Score: nullifier=4, claim=4, timestamp=1, pubkey_hash=1, is_code=1 → min 8.
pub const MIN_SOUND_CLAIM_RICHNESS: u32 = 8;

/// One public signal as a 32-byte BE Fr limb (or zero-padded short form).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrLimb([u8; 32]);

impl FrLimb {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_vec()
    }

    /// Parse decimal or 0x-hex string (snarkjs public.json style).
    pub fn from_decimal_str(s: &str) -> Result<Self, ContractError> {
        let t = s.trim();
        if t.is_empty() {
            return Ok(FrLimb([0u8; 32]));
        }
        if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
            return Self::from_hex(hex);
        }
        // decimal big-endian
        let n = parse_decimal_to_be32(t)?;
        // reject if ≥ BN254 r is done at host verify; here only packing
        Ok(FrLimb(n))
    }

    pub fn from_hex(hex: &str) -> Result<Self, ContractError> {
        let h = hex.trim();
        if h.len() > 64 || h.len() % 2 != 0 {
            return Err(ContractError::InvalidProof {
                reason: format!("Fr hex length invalid: {}", h.len()),
            });
        }
        let mut out = [0u8; 32];
        let raw = decode_hex(h)?;
        out[32 - raw.len()..].copy_from_slice(&raw);
        Ok(FrLimb(out))
    }

    pub fn from_be_bytes(b: [u8; 32]) -> Self {
        FrLimb(b)
    }

    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|&x| x == 0)
    }
}

/// Policy view extracted from circom publics (codec v1).
#[derive(Debug, Clone)]
pub struct CircomJwtPolicyClaim {
    pub codec: &'static str,
    pub schema: &'static str,
    pub nullifier: [u8; 32],
    pub claim_commitment: [u8; 32],
    pub public_key_hash: [u8; 32],
    pub timestamp: [u8; 32],
    pub is_code_exist: bool,
    /// Opaque principal hint for bridge (hex of accountSalt) — never a Zcash addr.
    pub principal_hint_hex: String,
    /// Hex nullifier for o-line AccessGrant / bridge.
    pub nullifier_hex: String,
}

impl CircomJwtPolicyClaim {
    /// Richness score (gateway-style).
    pub fn richness(&self) -> u32 {
        let mut s = 0u32;
        if self.nullifier != [0u8; 32] {
            s += 4;
        }
        if self.claim_commitment != [0u8; 32] {
            s += 4;
        }
        if self.public_key_hash != [0u8; 32] {
            s += 1;
        }
        if self.timestamp != [0u8; 32] {
            s += 1;
        }
        if self.is_code_exist {
            s += 1;
        }
        s
    }

    /// Build terp-zkjwt policy `public_inputs` prefix: nullifier || claim [|| no root for D3].
    pub fn to_policy_public_inputs(&self) -> Binary {
        build_public_inputs(&self.nullifier, &self.claim_commitment, None, None)
    }
}

/// Fail closed: sparse / incomplete claims rejected (like `grant_is_sound`).
pub fn claim_is_sound(c: &CircomJwtPolicyClaim) -> Result<(), String> {
    if c.nullifier == [0u8; 32] {
        return Err(
            "claim missing nullifier (sparse circom vector? need jwtNullifier at index 4)".into(),
        );
    }
    if c.claim_commitment == [0u8; 32] {
        return Err(
            "claim missing claim_commitment (sparse? need accountSalt at index 26 as codec v1)"
                .into(),
        );
    }
    if c.nullifier_hex.trim().is_empty() {
        return Err("claim missing nullifier_hex".into());
    }
    let r = c.richness();
    if r < MIN_SOUND_CLAIM_RICHNESS {
        return Err(format!(
            "claim richness {r} < MIN_SOUND_CLAIM_RICHNESS ({MIN_SOUND_CLAIM_RICHNESS})"
        ));
    }
    Ok(())
}

/// Decode n public Fr limbs (BE) into policy claim.
///
/// Accepts production **40**-public vectors or legacy **31**; fails closed below
/// [`CIRCOM_JWT_MIN_POLICY_PUBLICS`] (needs accountSalt at index 26).
pub fn decode_circom_publics_be(limbs: &[[u8; 32]]) -> Result<CircomJwtPolicyClaim, ContractError> {
    if limbs.len() < CIRCOM_JWT_MIN_POLICY_PUBLICS {
        return Err(ContractError::InvalidProof {
            reason: format!(
                "circom publics len {} < {CIRCOM_JWT_MIN_POLICY_PUBLICS} (sparse vector — fail closed; need jwtNullifier@4 + accountSalt@26)",
                limbs.len()
            ),
        });
    }
    let nullifier = limbs[IDX_JWT_NULLIFIER];
    let claim_commitment = limbs[IDX_ACCOUNT_SALT];
    let public_key_hash = limbs[IDX_PUBLIC_KEY_HASH];
    let timestamp = limbs[IDX_TIMESTAMP];
    let is_code_idx = if limbs.len() >= CIRCOM_JWT_N_PUBLIC {
        IDX_IS_CODE_EXIST
    } else if limbs.len() >= CIRCOM_JWT_N_PUBLIC_LEGACY31 {
        IDX_IS_CODE_EXIST_LEGACY31
    } else {
        // 27..30: no isCode limb — treat as false
        usize::MAX
    };
    let is_code_exist = if is_code_idx < limbs.len() {
        !FrLimb(limbs[is_code_idx]).is_zero()
    } else {
        false
    };

    Ok(CircomJwtPolicyClaim {
        codec: CIRCOM_JWT_CODEC_V1,
        schema: CLAIM_EVENT_SCHEMA_V1,
        nullifier,
        claim_commitment,
        public_key_hash,
        timestamp,
        is_code_exist,
        principal_hint_hex: hex_encode(&claim_commitment),
        nullifier_hex: hex_encode(&nullifier),
    })
}

/// Concat BE limbs → host instance bytes (full vector for Path A).
pub fn encode_host_instances(limbs: &[[u8; 32]]) -> Binary {
    let mut v = Vec::with_capacity(limbs.len() * 32);
    for l in limbs {
        v.extend_from_slice(l);
    }
    Binary::from(v)
}

/// Parse snarkjs-style public.json array of decimal strings → BE limbs.
pub fn parse_public_json_array(signals: &[String]) -> Result<Vec<[u8; 32]>, ContractError> {
    signals
        .iter()
        .map(|s| FrLimb::from_decimal_str(s).map(|f| f.0))
        .collect()
}

/// True when `public_inputs` looks like a full circom Fr vector (not policy prefix).
///
/// Policy layout is at most 128 bytes; circom needs ≥27×32 for salt@26.
pub fn is_circom_host_instances(bytes: &[u8]) -> bool {
    bytes.len() >= CIRCOM_JWT_MIN_POLICY_PUBLICS * 32 && bytes.len() % 32 == 0
}

/// Decode from concatenated BE bytes (length multiple of 32).
pub fn decode_host_instances_bytes(bytes: &[u8]) -> Result<CircomJwtPolicyClaim, ContractError> {
    if bytes.len() % 32 != 0 {
        return Err(ContractError::InvalidProof {
            reason: format!("instance bytes len {} not multiple of 32", bytes.len()),
        });
    }
    let limbs: Vec<[u8; 32]> = bytes
        .chunks_exact(32)
        .map(|c| {
            let mut a = [0u8; 32];
            a.copy_from_slice(c);
            a
        })
        .collect();
    decode_circom_publics_be(&limbs)
}

/// Domain-separated claim commitment from issuer + subject + salt (future circuit).
/// Codec v1 demo uses accountSalt directly; this helper is for RegisterClaim UX docs.
pub fn demo_claim_from_account_salt(account_salt: &[u8; 32]) -> [u8; 32] {
    *account_salt
}

/// Optional: SHA256(issuer || subject || salt) for off-circuit RegisterClaim when not using salt limb.
pub fn sha256_claim_commitment(issuer: &str, subject: &str, salt: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"terp-zkjwt/claim/v1");
    h.update(issuer.as_bytes());
    h.update([0u8]);
    h.update(subject.as_bytes());
    h.update([0u8]);
    h.update(salt);
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

// ── helpers ────────────────────────────────────────────────────────────────

fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn decode_hex(h: &str) -> Result<Vec<u8>, ContractError> {
    if h.len() % 2 != 0 {
        return Err(ContractError::InvalidProof {
            reason: "odd hex length".into(),
        });
    }
    (0..h.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&h[i..i + 2], 16).map_err(|_| ContractError::InvalidProof {
                reason: "bad hex digit".into(),
            })
        })
        .collect()
}

fn parse_decimal_to_be32(s: &str) -> Result<[u8; 32], ContractError> {
    // Simple base-10 → big-endian 32 bytes (for fixture decimals that fit u128 or big).
    // For full 254-bit fields use hex in fixtures; decimal path supports common snarkjs shorts.
    if s.chars().all(|c| c == '0') {
        return Ok([0u8; 32]);
    }
    // Use iterative multiply-add for arbitrary length decimal
    let mut acc = [0u8; 32];
    for c in s.chars() {
        let d = c.to_digit(10).ok_or_else(|| ContractError::InvalidProof {
            reason: format!("non-decimal char in public signal: {c}"),
        })? as u8;
        // acc = acc * 10 + d
        let mut carry = 0u16;
        for i in (0..32).rev() {
            let v = acc[i] as u16 * 10 + carry;
            acc[i] = (v & 0xff) as u8;
            carry = v >> 8;
        }
        if carry != 0 {
            return Err(ContractError::InvalidProof {
                reason: "decimal Fr overflow 32 bytes".into(),
            });
        }
        // add d
        carry = d as u16;
        for i in (0..32).rev() {
            let v = acc[i] as u16 + carry;
            acc[i] = (v & 0xff) as u8;
            carry = v >> 8;
            if carry == 0 {
                break;
            }
        }
        if carry != 0 {
            return Err(ContractError::InvalidProof {
                reason: "decimal Fr overflow on add".into(),
            });
        }
    }
    let _ = (NULLIFIER_LEN, CLAIM_COMMITMENT_LEN); // keep linked
    Ok(acc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{FULL_CIRCOM_PUBLICS_JSON, SPARSE_CIRCOM_PUBLICS_JSON};

    fn limbs_from_fixture(json: &str) -> Vec<[u8; 32]> {
        let signals: Vec<String> = serde_json::from_str(json).unwrap();
        parse_public_json_array(&signals).unwrap()
    }

    #[test]
    fn full_fixture_claim_is_sound() {
        let limbs = limbs_from_fixture(FULL_CIRCOM_PUBLICS_JSON);
        assert!(
            limbs.len() >= CIRCOM_JWT_MIN_POLICY_PUBLICS,
            "expected full production vector, got {}",
            limbs.len()
        );
        // Production demo-18-12-2024 jwt-auth = 40 publics
        assert_eq!(limbs.len(), CIRCOM_JWT_N_PUBLIC);
        let claim = decode_circom_publics_be(&limbs).unwrap();
        claim_is_sound(&claim).expect("full fixture must be sound");
        assert_eq!(claim.codec, CIRCOM_JWT_CODEC_V1);
        assert_eq!(claim.schema, CLAIM_EVENT_SCHEMA_V1);
        assert!(!claim.nullifier_hex.is_empty());
        assert_eq!(claim.nullifier_hex.len(), 64);
        // nullifier at index 4, salt at 26
        assert_eq!(&claim.nullifier, &limbs[IDX_JWT_NULLIFIER]);
        assert_eq!(&claim.claim_commitment, &limbs[IDX_ACCOUNT_SALT]);
        // Real snarkjs fixture: non-zero nullifier (not synthetic aa/bb pattern)
        assert!(!claim.nullifier.iter().all(|&b| b == 0));
        let pi = claim.to_policy_public_inputs();
        assert_eq!(pi.len(), 64);
    }

    #[test]
    fn l2_public_matches_full_fixture() {
        use crate::fixtures::L2_PUBLIC_JSON;
        let full: Vec<String> = serde_json::from_str(FULL_CIRCOM_PUBLICS_JSON).unwrap();
        let l2: Vec<String> = serde_json::from_str(L2_PUBLIC_JSON).unwrap();
        assert_eq!(
            full, l2,
            "full_circom_publics.json must match l2/public.json"
        );
    }

    #[test]
    fn sparse_fixture_fails_closed() {
        let signals: Vec<String> = serde_json::from_str(SPARSE_CIRCOM_PUBLICS_JSON).unwrap();
        // sparse has only nullifier-like short list
        let err = parse_public_json_array(&signals)
            .and_then(|l| decode_circom_publics_be(&l))
            .expect_err("sparse must not decode as full claim");
        let msg = err.to_string();
        assert!(
            msg.contains("sparse")
                || msg.contains("fail closed")
                || msg.contains("< 27")
                || msg.contains("MIN_POLICY"),
            "got: {msg}"
        );
    }

    #[test]
    fn zero_nullifier_unsound_even_if_len_ok() {
        let mut limbs = [[0u8; 32]; CIRCOM_JWT_N_PUBLIC];
        limbs[IDX_ACCOUNT_SALT] = [0xABu8; 32];
        // nullifier still zero
        let claim = decode_circom_publics_be(&limbs).unwrap();
        assert!(claim_is_sound(&claim).is_err());
    }

    #[test]
    fn host_instances_roundtrip_policy() {
        let limbs = limbs_from_fixture(FULL_CIRCOM_PUBLICS_JSON);
        let bytes = encode_host_instances(&limbs);
        let claim = decode_host_instances_bytes(bytes.as_slice()).unwrap();
        claim_is_sound(&claim).unwrap();
    }
}
