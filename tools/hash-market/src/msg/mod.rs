//! Shared protobuf-compatible types using `anybuf`.
//!
//! Field numbers **must** match `terp-core/proto/terp/hashmerchant/v1/types.proto`.

use anybuf::{Anybuf, Bufany};

// ---------------------------------------------------------------------------
// VoteExtensionHashData — ABCI++ vote extension payload
// ---------------------------------------------------------------------------

/// Payload validators include in their ABCI++ vote extensions.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VoteExtensionHashData {
    /// field 1: runtime identifier
    pub runtime_id: String,
    /// field 2: foreign chain UID (e.g. "ethereum-mainnet")
    pub chain_uid: String,
    /// field 3: hash algorithm (e.g. "keccak256")
    pub algo: String,
    /// field 4: state root bytes
    pub root: Vec<u8>,
    /// field 5: block height on the foreign chain
    pub foreign_height: u64,
    /// field 6: unix seconds of foreign block
    pub foreign_block_time: i64,
    /// field 7: optional ICS-23 commitment proof
    pub ics23_proof: Vec<u8>,
}

impl VoteExtensionHashData {
    pub fn encode(&self) -> Vec<u8> {
        Anybuf::new()
            .append_string(1, &self.runtime_id)
            .append_string(2, &self.chain_uid)
            .append_string(3, &self.algo)
            .append_bytes(4, &self.root)
            .append_uint64(5, self.foreign_height)
            .append_int64(6, self.foreign_block_time)
            .append_bytes(7, &self.ics23_proof)
            .into_vec()
    }

    pub fn decode(data: &[u8]) -> anyhow::Result<Self> {
        let buf = Bufany::deserialize(data)?;
        Ok(Self {
            runtime_id: buf.string(1).unwrap_or_default(),
            chain_uid: buf.string(2).unwrap_or_default(),
            algo: buf.string(3).unwrap_or_default(),
            root: buf.bytes(4).unwrap_or_default(),
            foreign_height: buf.uint64(5).unwrap_or_default(),
            foreign_block_time: buf.int64(6).unwrap_or_default(),
            ics23_proof: buf.bytes(7).unwrap_or_default(),
        })
    }
}

// ---------------------------------------------------------------------------
// HashRoot — confirmed foreign-chain state root after quorum
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HashRoot {
    /// field 1
    pub chain_uid: String,
    /// field 2
    pub algo: String,
    /// field 3
    pub height: u64,
    /// field 4
    pub root: Vec<u8>,
    /// field 5
    pub attestation_count: u32,
    /// field 6: unix seconds
    pub block_time: i64,
}

impl HashRoot {
    pub fn encode(&self) -> Vec<u8> {
        Anybuf::new()
            .append_string(1, &self.chain_uid)
            .append_string(2, &self.algo)
            .append_uint64(3, self.height)
            .append_bytes(4, &self.root)
            .append_uint32(5, self.attestation_count)
            .append_int64(6, self.block_time)
            .into_vec()
    }

    pub fn decode(data: &[u8]) -> anyhow::Result<Self> {
        let buf = Bufany::deserialize(data)?;
        Ok(Self {
            chain_uid: buf.string(1).unwrap_or_default(),
            algo: buf.string(2).unwrap_or_default(),
            height: buf.uint64(3).unwrap_or_default(),
            root: buf.bytes(4).unwrap_or_default(),
            attestation_count: buf.uint32(5).unwrap_or_default(),
            block_time: buf.int64(6).unwrap_or_default(),
        })
    }
}

// ---------------------------------------------------------------------------
// HashPairTicket — on-chain hash-pair primitive
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct HashPairTicket {
    /// field 1
    pub project_id: String,
    /// field 2
    pub origin_hash: Vec<u8>,
    /// field 3
    pub destination_hash: Vec<u8>,
    /// field 4
    pub zk_circuit_id: String,
    /// field 5
    pub destination_chain_id: u64,
    /// field 6
    pub expiry_block: u64,
    /// field 7
    pub metadata: Vec<u8>,
}

impl HashPairTicket {
    pub fn encode(&self) -> Vec<u8> {
        Anybuf::new()
            .append_string(1, &self.project_id)
            .append_bytes(2, &self.origin_hash)
            .append_bytes(3, &self.destination_hash)
            .append_string(4, &self.zk_circuit_id)
            .append_uint64(5, self.destination_chain_id)
            .append_uint64(6, self.expiry_block)
            .append_bytes(7, &self.metadata)
            .into_vec()
    }

    pub fn decode(data: &[u8]) -> anyhow::Result<Self> {
        let buf = Bufany::deserialize(data)?;
        Ok(Self {
            project_id: buf.string(1).unwrap_or_default(),
            origin_hash: buf.bytes(2).unwrap_or_default(),
            destination_hash: buf.bytes(3).unwrap_or_default(),
            zk_circuit_id: buf.string(4).unwrap_or_default(),
            destination_chain_id: buf.uint64(5).unwrap_or_default(),
            expiry_block: buf.uint64(6).unwrap_or_default(),
            metadata: buf.bytes(7).unwrap_or_default(),
        })
    }
}

// ---------------------------------------------------------------------------
// PallasLeaf — Pallas-curve leaf for ZK-friendly merkle trees
// ---------------------------------------------------------------------------

/// A 32-byte Pallas field element representing a leaf in a ZK-friendly tree.
#[derive(Debug, Clone, PartialEq)]
pub struct PallasLeaf(pub [u8; 32]);

/// Compute a transport merkle root from a set of Pallas leaves.
///
/// Uses a simple binary merkle tree with sorted-concat hashing (SHA-256).
/// Returns the hex-encoded root.
pub fn transport_merkle_root(leaves: &[PallasLeaf]) -> String {
    use std::collections::VecDeque;

    if leaves.is_empty() {
        return hex::encode([0u8; 32]);
    }

    use sha2::Digest;
    fn sha256(data: &[u8]) -> [u8; 32] {
        sha2::Sha256::digest(data).into()
    }

    let mut queue: VecDeque<[u8; 32]> = leaves.iter().map(|l| l.0).collect();

    while queue.len() > 1 {
        let mut next = VecDeque::new();
        while queue.len() >= 2 {
            let a = queue.pop_front().unwrap();
            let b = queue.pop_front().unwrap();
            // sorted concat
            let mut combined = Vec::with_capacity(64);
            if a <= b {
                combined.extend_from_slice(&a);
                combined.extend_from_slice(&b);
            } else {
                combined.extend_from_slice(&b);
                combined.extend_from_slice(&a);
            }
            next.push_back(sha256(&combined));
        }
        if let Some(remainder) = queue.pop_front() {
            next.push_back(remainder);
        }
        queue = next;
    }

    hex::encode(queue.pop_front().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vote_extension_roundtrip() {
        let orig = VoteExtensionHashData {
            runtime_id: "sidecar-1".into(),
            chain_uid: "ethereum-mainnet".into(),
            algo: "keccak256".into(),
            root: vec![0xde, 0xad, 0xbe, 0xef],
            foreign_height: 12345678,
            foreign_block_time: 1700000000,
            ics23_proof: vec![1, 2, 3],
        };
        let encoded = orig.encode();
        let decoded = VoteExtensionHashData::decode(&encoded).unwrap();
        assert_eq!(orig, decoded);
    }

    #[test]
    fn hash_root_roundtrip() {
        let orig = HashRoot {
            chain_uid: "cosmoshub-4".into(),
            algo: "sha256".into(),
            height: 999,
            root: vec![0xaa; 32],
            attestation_count: 42,
            block_time: 1700000000,
        };
        let encoded = orig.encode();
        let decoded = HashRoot::decode(&encoded).unwrap();
        assert_eq!(orig, decoded);
    }

    #[test]
    fn hash_pair_ticket_roundtrip() {
        let orig = HashPairTicket {
            project_id: "headstash-v1".into(),
            origin_hash: vec![0x11; 32],
            destination_hash: vec![0x22; 32],
            zk_circuit_id: "poseidon-2".into(),
            destination_chain_id: 1,
            expiry_block: 50000,
            metadata: vec![],
        };
        let encoded = orig.encode();
        let decoded = HashPairTicket::decode(&encoded).unwrap();
        assert_eq!(orig, decoded);
    }

    #[test]
    fn transport_merkle_root_single() {
        let leaf = PallasLeaf([0xab; 32]);
        let root = transport_merkle_root(std::slice::from_ref(&leaf));
        assert_eq!(root, hex::encode(leaf.0));
    }

    #[test]
    fn transport_merkle_root_empty() {
        let root = transport_merkle_root(&[]);
        assert_eq!(root, hex::encode([0u8; 32]));
    }
}
