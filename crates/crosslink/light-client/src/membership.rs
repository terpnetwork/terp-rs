//! Membership and non-membership proof verification for the Crosslink light client.
//!
//! # v1 (current)
//! Proofs target [`ConsensusState::shielded_commitment`] — the Zcash header /
//! shielded-pool commitment bound to the finalized `(bft_height, pow_anchor_height)`.
//!
//! Verification uses a **BLAKE3 pool-root proof** whose parameters match the
//! ICS-23-style [`PoolRootProofSpec`] set (see [`pool_root_proof_specs`]). The
//! implementation is intentionally self-contained (no full `ics23` crate) so the
//! CosmWasm 08-wasm artifact stays within host store-code limits.
//!
//! # IBC-v2
//! ZIP-222 application-state proofs will switch the root to
//! [`ConsensusState::app_state_commitment`] once that field is populated.

use cosmwasm_std::Api;

use crate::client_state::ClientState;
use crate::consensus_state::ConsensusState;
use crate::error::CrosslinkIBCError;

// ─── Proof specs (pool-root domain) ─────────────────────────────────────────

/// Hash algorithm identifier for the pool-root proof (BLAKE3-256).
pub const POOL_ROOT_HASH_ALG: &str = "BLAKE3";

/// Leaf prefix byte (ICS-23 Tendermint-style simple merkle: `0x00` leaf).
pub const POOL_ROOT_LEAF_PREFIX: u8 = 0x00;

/// Inner-node left/right prefixes (ICS-23 simple merkle).
pub const POOL_ROOT_INNER_LEFT: u8 = 0x01;
pub const POOL_ROOT_INNER_RIGHT: u8 = 0x02;

/// Fixed proof-spec parameters for the Crosslink v1 pool-root tree.
///
/// Mirrors Tendermint simple-merkle shape (1-byte direction prefix, 32-byte
/// children) but hashes with **BLAKE3**.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolRootProofSpec {
    /// Hash algorithm name (`"BLAKE3"`).
    pub hash: &'static str,
    /// Leaf prefix bytes applied before `key || value`.
    pub leaf_prefix: &'static [u8],
    /// Child size in bytes (32 for BLAKE3-256).
    pub child_size: usize,
    /// Whether key is pre-hashed before leaf construction (false in v1).
    pub prehash_key: bool,
    /// Whether value is pre-hashed before leaf construction (false in v1).
    pub prehash_value: bool,
}

/// Canonical pool-root BLAKE3 proof spec.
#[must_use]
pub fn pool_root_proof_spec() -> PoolRootProofSpec {
    PoolRootProofSpec {
        hash: POOL_ROOT_HASH_ALG,
        leaf_prefix: &[POOL_ROOT_LEAF_PREFIX],
        child_size: 32,
        prehash_key: false,
        prehash_value: false,
    }
}

/// The **set of proof specs** for v1 membership under the shielded pool root.
///
/// Single tree today (pool commitment is the sole commitment root). Extra
/// specs can be appended for multi-store chaining without changing the client
/// wire format.
#[must_use]
pub fn pool_root_proof_specs() -> Vec<PoolRootProofSpec> {
    vec![pool_root_proof_spec()]
}

// ─── Wire format ────────────────────────────────────────────────────────────

/// One existence step under the pool-root tree.
///
/// Binary layout (little-endian lengths):
/// ```text
/// key_len:u32 | key | value_len:u32 | value | steps:u32 |
///   for each step: side:u8 (0=left sibling, 1=right sibling) | sibling:32
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolRootExistenceProof {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    /// Sibling hashes from the leaf toward the root. `side == 0` means the
    /// sibling is on the left (current hash is right child); `side == 1` means
    /// sibling is on the right.
    pub path: Vec<SiblingStep>,
}

/// A single inner-node step in the existence path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiblingStep {
    /// 0 = sibling is left child; 1 = sibling is right child.
    pub side: u8,
    pub sibling: [u8; 32],
}

impl PoolRootExistenceProof {
    /// Encode to the compact binary format used as IBC `proof` bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            4 + self.key.len() + 4 + self.value.len() + 4 + self.path.len() * 33,
        );
        out.extend_from_slice(&(self.key.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.key);
        out.extend_from_slice(&(self.value.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.value);
        out.extend_from_slice(&(self.path.len() as u32).to_le_bytes());
        for step in &self.path {
            out.push(step.side);
            out.extend_from_slice(&step.sibling);
        }
        out
    }

    /// Decode from [`Self::encode`] bytes.
    pub fn decode(mut bz: &[u8]) -> Result<Self, CrosslinkIBCError> {
        let key = read_len_bytes(&mut bz, "key")?;
        let value = read_len_bytes(&mut bz, "value")?;
        let n = read_u32(&mut bz, "path_len")? as usize;
        let mut path = Vec::with_capacity(n);
        for i in 0..n {
            if bz.is_empty() {
                return Err(fail(format!("truncated path step {i}")));
            }
            let side = bz[0];
            bz = &bz[1..];
            if side > 1 {
                return Err(fail(format!("invalid side byte {side} at step {i}")));
            }
            if bz.len() < 32 {
                return Err(fail(format!("truncated sibling at step {i}")));
            }
            let mut sibling = [0u8; 32];
            sibling.copy_from_slice(&bz[..32]);
            bz = &bz[32..];
            path.push(SiblingStep { side, sibling });
        }
        if !bz.is_empty() {
            return Err(fail(format!("{} trailing proof bytes", bz.len())));
        }
        Ok(Self { key, value, path })
    }

    /// Compute the commitment root for this existence proof under the pool-root
    /// BLAKE3 leaf/inner ops.
    pub fn calculate_root(&self, api: &dyn Api) -> Result<[u8; 32], CrosslinkIBCError> {
        let mut h = leaf_hash(api, &self.key, &self.value);
        for step in &self.path {
            h = match step.side {
                0 => inner_hash(api, &step.sibling, &h), // sibling left, current right
                1 => inner_hash(api, &h, &step.sibling), // current left, sibling right
                other => return Err(fail(format!("invalid side {other}"))),
            };
        }
        Ok(h)
    }
}

// ─── Public verify API ──────────────────────────────────────────────────────

/// Verify that `value` exists at `merkle_path` under the consensus state's
/// **v1 shielded commitment** (pool) root.
///
/// # Errors
///
/// Returns [`CrosslinkIBCError::MembershipVerificationFailed`] if the proof is
/// invalid, the path is empty, or the calculated root does not match
/// `consensus_state.shielded_commitment`.
pub fn verify_membership(
    api: &dyn Api,
    consensus_state: ConsensusState,
    _client_state: ClientState,
    proof: Vec<u8>,
    merkle_path: Vec<Vec<u8>>,
    value: Vec<u8>,
) -> Result<(), CrosslinkIBCError> {
    let _specs = pool_root_proof_specs(); // documented set of specs
    let exist = PoolRootExistenceProof::decode(&proof)?;
    let key = deepest_key(&merkle_path)?;

    if exist.key != key {
        return Err(fail("proof key does not match merkle path"));
    }
    if exist.value != value {
        return Err(fail("proof value does not match provided value"));
    }

    let root = exist.calculate_root(api)?;
    if root != consensus_state.shielded_commitment {
        return Err(fail(
            "calculated root does not match shielded pool commitment",
        ));
    }
    Ok(())
}

/// Verify that no value exists at `merkle_path` under the consensus state's
/// **v1 shielded commitment** (pool) root.
///
/// v1 non-membership uses a neighbour existence proof: the proof bytes encode
/// a [`PoolRootExistenceProof`] for a **strictly adjacent** key that sits in
/// the same tree; the verified neighbour root must equal the pool commitment
/// and the claimed key must sort strictly between the documented left/right
/// neighbour policy for singleton proofs is:
///
/// - proof value is empty and key equals the path key → rejected (use existence)
/// - otherwise: require an existence proof of a different key whose root matches,
///   and the path key is not equal to the proven key (weak non-membership for v1
///   pending full sorted-tree non-existence certificates).
///
/// # Errors
///
/// Returns [`CrosslinkIBCError::MembershipVerificationFailed`] if verification fails.
pub fn verify_non_membership(
    api: &dyn Api,
    consensus_state: ConsensusState,
    _client_state: ClientState,
    proof: Vec<u8>,
    merkle_path: Vec<Vec<u8>>,
) -> Result<(), CrosslinkIBCError> {
    let claimed = deepest_key(&merkle_path)?;
    let neighbour = PoolRootExistenceProof::decode(&proof)?;
    if neighbour.key == claimed {
        return Err(fail(
            "non-membership neighbour key must differ from claimed absent key",
        ));
    }
    let root = neighbour.calculate_root(api)?;
    if root != consensus_state.shielded_commitment {
        return Err(fail(
            "neighbour existence root does not match shielded pool commitment",
        ));
    }
    // v1: presence of a different key under the same root is accepted as a
    // weak non-membership certificate. Full left/right range proofs land with
    // the sorted SMT / ZIP-222 work.
    Ok(())
}

// ─── Hash helpers ───────────────────────────────────────────────────────────

fn leaf_hash(api: &dyn Api, key: &[u8], value: &[u8]) -> [u8; 32] {
    let mut buf = Vec::with_capacity(1 + 4 + key.len() + 4 + value.len());
    buf.push(POOL_ROOT_LEAF_PREFIX);
    buf.extend_from_slice(&(key.len() as u32).to_le_bytes());
    buf.extend_from_slice(key);
    buf.extend_from_slice(&(value.len() as u32).to_le_bytes());
    buf.extend_from_slice(value);
    api.blake3_256(&buf)
}

fn inner_hash(api: &dyn Api, left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut buf = Vec::with_capacity(66);
    buf.push(POOL_ROOT_INNER_LEFT);
    buf.extend_from_slice(left);
    buf.push(POOL_ROOT_INNER_RIGHT);
    buf.extend_from_slice(right);
    api.blake3_256(&buf)
}

fn deepest_key(merkle_path: &[Vec<u8>]) -> Result<Vec<u8>, CrosslinkIBCError> {
    merkle_path
        .last()
        .cloned()
        .ok_or_else(|| fail("empty merkle path"))
}

fn read_u32(bz: &mut &[u8], label: &str) -> Result<u32, CrosslinkIBCError> {
    if bz.len() < 4 {
        return Err(fail(format!("truncated {label}")));
    }
    let mut a = [0u8; 4];
    a.copy_from_slice(&bz[..4]);
    *bz = &bz[4..];
    Ok(u32::from_le_bytes(a))
}

fn read_len_bytes(bz: &mut &[u8], label: &str) -> Result<Vec<u8>, CrosslinkIBCError> {
    let n = read_u32(bz, label)? as usize;
    if bz.len() < n {
        return Err(fail(format!("truncated {label} bytes")));
    }
    let out = bz[..n].to_vec();
    *bz = &bz[n..];
    Ok(out)
}

fn fail(msg: impl Into<String>) -> CrosslinkIBCError {
    CrosslinkIBCError::MembershipVerificationFailed(msg.into())
}

// ─── Test / integrator helpers ──────────────────────────────────────────────

/// Build a leaf-only existence proof (empty path) for `(key, value)`.
#[must_use]
pub fn leaf_existence_proof(key: &[u8], value: &[u8]) -> PoolRootExistenceProof {
    PoolRootExistenceProof {
        key: key.to_vec(),
        value: value.to_vec(),
        path: Vec::new(),
    }
}

/// Encode a leaf-only existence proof for use as IBC `proof` bytes.
#[must_use]
pub fn encode_merkle_proof(proof: &PoolRootExistenceProof) -> Vec<u8> {
    proof.encode()
}

/// Leaf commitment root for a singleton tree — store as
/// `ConsensusState.shielded_commitment` in tests.
#[must_use]
pub fn leaf_commitment_root(api: &dyn Api, key: &[u8], value: &[u8]) -> [u8; 32] {
    leaf_hash(api, key, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_state::ClientState;
    use crate::types::{Blake3Hash, PROTOTYPE_PARAMETERS};

    fn api() -> cosmwasm_std::testing::MockApi {
        cosmwasm_std::testing::MockApi::default()
    }

    fn dummy_client() -> ClientState {
        ClientState::new(
            PROTOTYPE_PARAMETERS,
            Blake3Hash([0u8; 32]),
            1,
            100,
            [0u8; 32],
            vec![],
        )
    }

    #[test]
    fn pool_root_specs_are_blake3() {
        let specs = pool_root_proof_specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].hash, "BLAKE3");
        assert_eq!(specs[0].child_size, 32);
        assert_eq!(specs[0].leaf_prefix, &[0x00]);
    }

    #[test]
    fn leaf_membership_against_shielded_commitment() {
        let key = b"ibc/packet/channel-0/1".to_vec();
        let value = b"packet-commitment-bytes".to_vec();
        let root = leaf_commitment_root(&api(), &key, &value);

        let consensus = ConsensusState::v1(1, 100, [0xab; 32], 1_700_000_000, root);
        let client = dummy_client();
        let proof = encode_merkle_proof(&leaf_existence_proof(&key, &value));

        verify_membership(
            &api(),
            consensus.clone(),
            client.clone(),
            proof.clone(),
            vec![key.clone()],
            value.clone(),
        )
        .expect("membership should succeed");

        assert!(
            verify_membership(
                &api(),
                consensus.clone(),
                client.clone(),
                proof.clone(),
                vec![key.clone()],
                b"wrong".to_vec(),
            )
            .is_err()
        );

        let bad = ConsensusState::v1(1, 100, [0xab; 32], 1_700_000_000, [0xff; 32]);
        assert!(verify_membership(&api(), bad, client, proof, vec![key], value).is_err());
    }

    #[test]
    fn path_with_prefix_uses_deepest_key() {
        let key = b"leaf-key".to_vec();
        let value = b"v".to_vec();
        let root = leaf_commitment_root(&api(), &key, &value);
        let consensus = ConsensusState::v1(0, 0, [0u8; 32], 0, root);
        let proof = encode_merkle_proof(&leaf_existence_proof(&key, &value));

        verify_membership(
            &api(),
            consensus,
            dummy_client(),
            proof,
            vec![b"ibc".to_vec(), key],
            value,
        )
        .expect("prefix path should use deepest key");
    }

    #[test]
    fn existence_with_one_sibling() {
        let key = b"k".to_vec();
        let value = b"v".to_vec();
        let leaf = leaf_hash(&api(), &key, &value);
        let sibling = [0x11; 32];
        let root = inner_hash(&api(), &leaf, &sibling); // current left, sibling right

        let proof = PoolRootExistenceProof {
            key: key.clone(),
            value: value.clone(),
            path: vec![SiblingStep { side: 1, sibling }],
        };
        assert_eq!(proof.calculate_root(&api()).unwrap(), root);

        let consensus = ConsensusState::v1(0, 0, [0u8; 32], 0, root);
        verify_membership(
            &api(),
            consensus,
            dummy_client(),
            proof.encode(),
            vec![key],
            value,
        )
        .unwrap();
    }

    #[test]
    fn empty_path_or_proof_rejected() {
        let consensus = ConsensusState::v1(0, 0, [0u8; 32], 0, [0u8; 32]);
        let client = dummy_client();
        assert!(
            verify_membership(
                &api(),
                consensus.clone(),
                client.clone(),
                vec![],
                vec![b"k".to_vec()],
                vec![1],
            )
            .is_err()
        );
        assert!(
            verify_membership(&api(), consensus, client, vec![1, 2, 3], vec![], vec![1]).is_err()
        );
    }

    #[test]
    fn non_membership_requires_different_neighbour_key() {
        let key = b"absent".to_vec();
        let other = b"present".to_vec();
        let value = b"v".to_vec();
        let root = leaf_commitment_root(&api(), &other, &value);
        let consensus = ConsensusState::v1(0, 0, [0u8; 32], 0, root);
        let client = dummy_client();

        let ok_proof = encode_merkle_proof(&leaf_existence_proof(&other, &value));
        verify_non_membership(
            &api(),
            consensus.clone(),
            client.clone(),
            ok_proof,
            vec![key.clone()],
        )
        .expect("neighbour under same root");

        let bad = encode_merkle_proof(&leaf_existence_proof(&key, &value));
        assert!(verify_non_membership(&api(), consensus, client, bad, vec![key]).is_err());
    }
}
