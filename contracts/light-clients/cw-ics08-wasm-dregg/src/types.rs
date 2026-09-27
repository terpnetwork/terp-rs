//! Logical ClientState / ConsensusState / SettlementUpdate (B2).
//! Lanes are `Vec<u32>` length 8 for cosmwasm-schema Schemaifier support.

use cosmwasm_schema::cw_serde;
use cosmwasm_std::Binary;

/// BabyBear modulus (for structural lane checks).
pub const BABYBEAR_P: u32 = 0x7800_0001;

#[cw_serde]
pub enum TrustLevel {
    SettlementWrapOnly,
}

#[cw_serde]
pub enum ClaimCeiling {
    SettlementOnly,
}

#[cw_serde]
pub enum ConsensusKind {
    B,
}

#[cw_serde]
pub struct ClientState {
    pub client_schema_version: u32,
    pub chain_id: String,
    pub federation_id: Option<Binary>,
    pub trust_level: TrustLevel,
    /// Hex, 32 bytes, non-zero.
    pub settlement_vk_hash: String,
    /// Length 8 BabyBear lanes.
    pub genesis_lanes: Vec<u32>,
    pub latest_height: u64,
    pub frozen_height: Option<u64>,
    pub allow_gate_off_finality: bool,
    pub claim_ceiling: ClaimCeiling,
    pub auth_min_proven_height: Option<u64>,
    pub auth_require_root_in_history: bool,
}

#[cw_serde]
pub struct ConsensusState {
    pub version: u8,
    pub kind: ConsensusKind,
    pub proven_lanes: Vec<u32>,
    pub proven_root_key: String,
    pub proven_height: u64,
}

#[cw_serde]
pub struct SettlementUpdate {
    pub proof: Vec<String>,
    pub commitments: Vec<String>,
    pub commitment_pok: Vec<String>,
    pub genesis_root: Vec<u32>,
    pub final_root: Vec<u32>,
    pub num_turns: u32,
    pub chain_digest: Vec<u32>,
}

pub fn as_lanes8(v: &[u32]) -> Result<[u32; 8], ()> {
    if v.len() != 8 {
        return Err(());
    }
    let mut a = [0u32; 8];
    a.copy_from_slice(v);
    Ok(a)
}

pub fn pack_lanes_hex(lanes: &[u32]) -> Result<String, ()> {
    use sha2::{Digest, Sha256};
    let a = as_lanes8(lanes)?;
    let mut bytes = [0u8; 32];
    for (i, lane) in a.iter().enumerate() {
        bytes[i * 4..(i + 1) * 4].copy_from_slice(&lane.to_be_bytes());
    }
    Ok(hex::encode(Sha256::digest(bytes)))
}

pub fn lanes_canonical(lanes: &[u32]) -> bool {
    as_lanes8(lanes)
        .map(|a| a.iter().all(|l| *l < BABYBEAR_P))
        .unwrap_or(false)
}

pub fn parse_vk_hash(s: &str) -> Result<[u8; 32], ()> {
    let s = s.trim_start_matches("0x");
    let raw = hex::decode(s).map_err(|_| ())?;
    if raw.len() != 32 || raw.iter().all(|b| *b == 0) {
        return Err(());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&raw);
    Ok(out)
}
