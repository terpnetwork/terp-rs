//! Consensus state for the Crosslink light client.
//!
//! Stores the finalized PoW/BFT anchor and the shielded-pool commitment bound
//! to that height. ZIP-222 application-state roots are reserved for IBC-v2.

use crate::types::{SerializationError, ZcashDeserialize, ZcashSerialize};

/// Consensus state at a specific BFT height.
///
/// Height binding (v1):
/// - `bft_height` — PoS / TFL height that finalized this tip
/// - `pow_anchor_height` + `pow_anchor_hash` — the finalized Zcash PoW block
/// - `shielded_commitment` — Zcash header commitment for that PoW block
///   (era-dependent shielded / history / auth-data anchor)
///
/// IBC-v2 will populate `app_state_commitment` from ZIP-222; v1 leaves it zero.
#[derive(Clone, Debug, PartialEq)]
pub struct ConsensusState {
    /// BFT (PoS / TFL) height that produced this consensus snapshot.
    pub bft_height: u32,
    /// PoW anchor block height (the finalized Zcash block).
    pub pow_anchor_height: u32,
    /// PoW anchor block hash.
    pub pow_anchor_hash: [u8; 32],
    /// Unix timestamp (seconds) of the anchor block.
    pub timestamp: u64,
    /// Shielded-pool / header commitment from the finalized PoW header
    /// (`PowHeader.commitment_bytes`). Bound to `(bft_height, pow_anchor_height)`.
    pub shielded_commitment: [u8; 32],
    /// Reserved for ZIP-222 application state root (IBC-v2). Always zero in v1.
    pub app_state_commitment: [u8; 32],
}

impl ConsensusState {
    /// Construct a v1 consensus state (app-state root left zero).
    #[must_use]
    pub fn v1(
        bft_height: u32,
        pow_anchor_height: u32,
        pow_anchor_hash: [u8; 32],
        timestamp: u64,
        shielded_commitment: [u8; 32],
    ) -> Self {
        Self {
            bft_height,
            pow_anchor_height,
            pow_anchor_hash,
            timestamp,
            shielded_commitment,
            app_state_commitment: [0u8; 32],
        }
    }
}

impl ZcashSerialize for ConsensusState {
    fn zcash_serialize<W: std::io::Write>(&self, mut writer: W) -> Result<(), std::io::Error> {
        writer.write_all(&self.bft_height.to_le_bytes())?;
        writer.write_all(&self.pow_anchor_height.to_le_bytes())?;
        writer.write_all(&self.pow_anchor_hash)?;
        writer.write_all(&self.timestamp.to_le_bytes())?;
        writer.write_all(&self.shielded_commitment)?;
        writer.write_all(&self.app_state_commitment)?;
        Ok(())
    }
}

impl ZcashDeserialize for ConsensusState {
    fn zcash_deserialize<R: std::io::Read>(mut reader: R) -> Result<Self, SerializationError> {
        let mut bft = [0u8; 4];
        let mut pow_h = [0u8; 4];
        let mut pow_hash = [0u8; 32];
        let mut ts = [0u8; 8];
        let mut shielded = [0u8; 32];
        let mut app = [0u8; 32];
        reader.read_exact(&mut bft)?;
        reader.read_exact(&mut pow_h)?;
        reader.read_exact(&mut pow_hash)?;
        reader.read_exact(&mut ts)?;
        reader.read_exact(&mut shielded)?;
        reader.read_exact(&mut app)?;
        Ok(Self {
            bft_height: u32::from_le_bytes(bft),
            pow_anchor_height: u32::from_le_bytes(pow_h),
            pow_anchor_hash: pow_hash,
            timestamp: u64::from_le_bytes(ts),
            shielded_commitment: shielded,
            app_state_commitment: app,
        })
    }
}
