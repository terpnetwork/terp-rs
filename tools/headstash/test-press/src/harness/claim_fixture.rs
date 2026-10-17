//! DEMO-PATH A2 claim fixture — load/build without Halo2 prove.
//!
//! Schema: `crates/headstash/docs/circuit/DEMO-PATH.md` § Claim fixture schema (A2).
//!
//! # L1 mock ZK posture
//! CI / L1 multi-test may accept policy + **mock** proof bytes against this layout.
//! Real H1 composite MockProver / prove is a **separate track** (`just demo-h1` /
//! nightly). Never label mock-verify green as Tier-0 soundness.

use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Public instance wire size (6 × 32-byte field elements).
pub const CLAIM_INSTANCE_BYTES_LEN: usize = 200;

#[derive(Debug, Error)]
pub enum ClaimFixtureError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("schema: {0}")]
    Schema(String),
}

/// DEMO-PATH A2 claim fixture (JSON-stable).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimFixture {
    /// Distro hash domain tag (e.g. `"poseidon-v1"`).
    pub distro_hash_domain: String,
    /// Poseidon depth-32 root (`0x` hex, 32-byte LE pallas base).
    pub root: String,
    pub root_id: u32,
    pub leaf_index: u64,
    /// Auth path depth (product: 32).
    pub depth: u32,
    /// Sibling path hex strings (length = depth when full path present).
    #[serde(default)]
    pub path: Vec<String>,
    pub partial_note: ClaimFixturePartialNote,
    pub instance: ClaimFixtureInstance,
    /// Must be 200 when present.
    #[serde(default = "default_instance_len")]
    pub instance_bytes_len: usize,
    #[serde(default)]
    pub circuit: ClaimFixtureCircuit,
    /// Optional pre-encoded 200-byte instance wire (`0x` hex). Policy tests may omit.
    #[serde(default)]
    pub instance_bytes_hex: Option<String>,
    /// Mock-ZK proof bytes hex (L1 CI). Empty / absent = policy-only fixture.
    #[serde(default)]
    pub mock_proof_hex: Option<String>,
}

fn default_instance_len() -> usize {
    CLAIM_INSTANCE_BYTES_LEN
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimFixturePartialNote {
    pub esk_hex: String,
    pub token: String,
    pub value: u64,
    pub fdi: u64,
    pub recipient: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimFixtureInstance {
    pub anchor: String,
    pub nd: String,
    pub v: u64,
    pub recp: String,
    pub nf: String,
    pub cmx: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimFixtureCircuit {
    pub k: u32,
    pub public_inputs: u32,
}

impl Default for ClaimFixtureCircuit {
    fn default() -> Self {
        Self {
            k: 18,
            public_inputs: 6,
        }
    }
}

impl ClaimFixture {
    /// Structural policy checks (no prove): root/anchor present, instance size contract.
    pub fn validate_policy(&self) -> Result<(), ClaimFixtureError> {
        if self.distro_hash_domain.is_empty() {
            return Err(ClaimFixtureError::Schema(
                "distro_hash_domain empty".into(),
            ));
        }
        if self.instance_bytes_len != CLAIM_INSTANCE_BYTES_LEN {
            return Err(ClaimFixtureError::Schema(format!(
                "instance_bytes_len must be {CLAIM_INSTANCE_BYTES_LEN}, got {}",
                self.instance_bytes_len
            )));
        }
        if self.circuit.public_inputs != 6 {
            return Err(ClaimFixtureError::Schema(
                "circuit.public_inputs must be 6".into(),
            ));
        }
        if self.depth == 0 {
            return Err(ClaimFixtureError::Schema("depth must be > 0".into()));
        }
        if !self.path.is_empty() && self.path.len() as u32 != self.depth {
            return Err(ClaimFixtureError::Schema(format!(
                "path len {} != depth {}",
                self.path.len(),
                self.depth
            )));
        }
        if self.partial_note.value != self.instance.v {
            return Err(ClaimFixtureError::Schema(
                "partial_note.value != instance.v".into(),
            ));
        }
        // Anchor should match published root for claim policy.
        if normalize_hex(&self.root) != normalize_hex(&self.instance.anchor) {
            return Err(ClaimFixtureError::Schema(
                "root != instance.anchor (claim policy bind)".into(),
            ));
        }
        Ok(())
    }

    /// Parse optional mock proof bytes (empty vec if absent).
    pub fn mock_proof_bytes(&self) -> Result<Vec<u8>, ClaimFixtureError> {
        match &self.mock_proof_hex {
            None => Ok(Vec::new()),
            Some(h) => decode_hex(h).map_err(ClaimFixtureError::Schema),
        }
    }
}

/// Load DEMO-PATH A2 JSON from disk and validate policy layout.
pub fn load_claim_fixture(path: &Path) -> Result<ClaimFixture, ClaimFixtureError> {
    let raw = std::fs::read_to_string(path)?;
    let fix: ClaimFixture = serde_json::from_str(&raw)?;
    fix.validate_policy()?;
    Ok(fix)
}

/// Build a **suite-backed** Product A claim fixture (Poseidon root + instance layout).
///
/// Requires `interface` + host suite (random tree). Policy-valid; use real
/// `Proof::create` / MockProver for soundness of the proof field.
#[cfg(feature = "interface")]
pub fn build_suite_backed_claim_fixture(
    num_leaves: usize,
    selected_index: usize,
) -> Result<ClaimFixture, ClaimFixtureError> {
    use cw_orch::prelude::Mock;
    use ff::PrimeField;
    use zk_headstash::suite::suite::HeadstashProofBuilder;
    use zk_headstash::suite::HeadstashCircuitSuite;

    let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
    let (_circuit, instance, anchor, partial) = suite
        .suite_backed_claim_pair(num_leaves, selected_index)
        .map_err(|e| ClaimFixtureError::Schema(format!("suite_backed_claim_pair: {e}")))?;

    // Wire layout: anchor(32) | nd(32) | v(8) | nf(32) | recp(32) | cmx(32) | e(32) = 200
    let inst_bytes = instance.to_bytes();
    if inst_bytes.len() != CLAIM_INSTANCE_BYTES_LEN {
        return Err(ClaimFixtureError::Schema(format!(
            "instance to_bytes len {}",
            inst_bytes.len()
        )));
    }
    let root_hex = format!("0x{}", hex::encode(anchor.to_bytes()));
    let path_hex: Vec<String> = (0..32).map(|i| format!("0x{:064x}", i)).collect();

    Ok(ClaimFixture {
        distro_hash_domain: "poseidon-v1".into(),
        root: root_hex.clone(),
        root_id: 0,
        leaf_index: selected_index as u64,
        depth: 32,
        path: path_hex,
        partial_note: ClaimFixturePartialNote {
            esk_hex: format!("0x{}", hex::encode(partial.esk_bytes)),
            token: partial.token,
            value: partial.value,
            fdi: partial.fdi,
            recipient: format!("0x{}", hex::encode(partial.recipient)),
        },
        instance: ClaimFixtureInstance {
            anchor: root_hex,
            nd: format!("0x{}", hex::encode(&inst_bytes[32..64])),
            v: u64::from_le_bytes(inst_bytes[64..72].try_into().unwrap()),
            nf: format!("0x{}", hex::encode(&inst_bytes[72..104])),
            recp: format!("0x{}", hex::encode(&inst_bytes[104..136])),
            cmx: format!("0x{}", hex::encode(&inst_bytes[136..168])),
        },
        instance_bytes_len: CLAIM_INSTANCE_BYTES_LEN,
        circuit: ClaimFixtureCircuit {
            k: 18,
            public_inputs: 6,
        },
        instance_bytes_hex: Some(format!("0x{}", hex::encode(inst_bytes))),
        mock_proof_hex: Some("0x01".into()), // placeholder; lab claim_mock_verify
    })
}

/// Build a **synthetic** policy fixture for tests (not cryptographically proven).
///
/// Poseidon root + instance layout only — sufficient for nullifier/root policy
/// unit tests without suite/MockProver. `leaf_index` is embedded in labels.
pub fn build_policy_claim_fixture(leaf_index: u64, value: u64) -> ClaimFixture {
    let root = format!("0x{:064x}", leaf_index.wrapping_mul(0x11) + 0xA1);
    // Pad to 66 chars (0x + 64 hex) already via format.
    let nf = format!("0x{:064x}", leaf_index.wrapping_mul(0x22) + 0xB2);
    let cmx = format!("0x{:064x}", leaf_index.wrapping_mul(0x33) + 0xC3);
    let recp = format!("0x{:064x}", leaf_index.wrapping_mul(0x44) + 0xD4);
    let nd = format!("0x{:064x}", leaf_index.wrapping_mul(0x55) + 0xE5);
    let path = (0..32)
        .map(|i| format!("0x{:064x}", (leaf_index << 8) | i))
        .collect();

    ClaimFixture {
        distro_hash_domain: "poseidon-v1".into(),
        root: root.clone(),
        root_id: 0,
        leaf_index,
        depth: 32,
        path,
        partial_note: ClaimFixturePartialNote {
            esk_hex: format!("0x{:064x}", leaf_index.wrapping_mul(0x66) + 0xF6),
            token: "uterp".into(),
            value,
            fdi: leaf_index,
            recipient: recp.clone(),
        },
        instance: ClaimFixtureInstance {
            anchor: root,
            nd,
            v: value,
            recp,
            nf,
            cmx,
        },
        instance_bytes_len: CLAIM_INSTANCE_BYTES_LEN,
        circuit: ClaimFixtureCircuit::default(),
        instance_bytes_hex: None,
        mock_proof_hex: Some("0xdeadbeef".into()), // mock-only; not a real proof
    }
}

fn normalize_hex(s: &str) -> String {
    let t = s.trim();
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    t.to_ascii_lowercase()
}

fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
    let t = normalize_hex(s);
    if t.len() % 2 != 0 {
        return Err(format!("odd hex length: {}", t.len()));
    }
    (0..t.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&t[i..i + 2], 16)
                .map_err(|e| format!("hex parse at {i}: {e}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_policy_fixture_validates() {
        let f = build_policy_claim_fixture(3, 1_000_000);
        f.validate_policy().expect("synthetic ok");
        assert_eq!(f.instance_bytes_len, 200);
        assert_eq!(f.depth, 32);
        assert_eq!(f.path.len(), 32);
        assert_eq!(f.mock_proof_bytes().unwrap(), vec![0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn root_anchor_mismatch_rejects() {
        let mut f = build_policy_claim_fixture(1, 100);
        f.instance.anchor = "0x00".into();
        assert!(f.validate_policy().is_err());
    }

    #[test]
    fn json_roundtrip() {
        let f = build_policy_claim_fixture(2, 42);
        let s = serde_json::to_string_pretty(&f).unwrap();
        let back: ClaimFixture = serde_json::from_str(&s).unwrap();
        back.validate_policy().unwrap();
        assert_eq!(back.leaf_index, 2);
        assert_eq!(back.instance.v, 42);
    }

    #[cfg(feature = "interface")]
    #[test]
    fn suite_backed_fixture_validates_product_a() {
        let f = build_suite_backed_claim_fixture(4, 1).expect("suite fixture");
        f.validate_policy().expect("policy");
        assert_eq!(f.distro_hash_domain, "poseidon-v1");
        assert_eq!(f.depth, 32);
        assert_eq!(f.circuit.k, 18);
        assert_eq!(f.instance_bytes_len, 200);
        assert_eq!(f.leaf_index, 1);
    }
}
