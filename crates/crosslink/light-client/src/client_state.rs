//! Client state for the Crosslink light client.
//!
//! Stores the latest finalized TFL state, the finalizer roster,
//! and the crosslink protocol parameters.
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::Blake3Hash;
use crate::types::{
    SerializationError, ZcashCrosslinkParameters, ZcashDeserialize, ZcashSerialize,
};

/// A finalizer (validator) in the TFL roster.
#[derive(Clone, Debug, PartialEq)]
pub struct FinalizerEntry {
    /// Ed25519 public key (32 bytes).
    pub public_key: [u8; 32],
    /// Voting power.
    pub voting_power: u64,
}

impl FinalizerEntry {
    pub fn new(pk: [u8; 32], vp: u64) -> Self {
        Self {
            public_key: pk,
            voting_power: vp,
        }
    }
}

/// Crosslink light client state stored on the Cosmos chain.
#[derive(Clone, Debug)]
pub struct ClientState {
    /// Crosslink protocol parameters (sigma, L).
    /// note: we implement .into() & .from() for the official `ZcashCrosslinkParameters` struct
    pub crosslink_params: ZcashCrosslinkParameters,
    /// Hash of the latest finalized BFT block.
    pub latest_bft_block_hash: Blake3Hash,
    /// Height of the latest finalized BFT block.
    pub latest_bft_height: u32,
    /// Height of the latest finalized PoW anchor.
    pub latest_finalized_pow_height: u32,
    /// Hash of the latest finalized PoW anchor block.
    pub latest_finalized_pow_hash: [u8; 32],
    /// Shielded / header commitment of the latest finalized PoW block
    /// (`PowHeader.commitment_bytes`). Bound to `latest_bft_height` +
    /// `latest_finalized_pow_height`.
    pub latest_shielded_commitment: [u8; 32],
    /// Current TFL finalizer roster.
    pub finalizer_roster: Vec<FinalizerEntry>,
    /// Whether the client has been frozen due to misbehaviour.
    pub is_frozen: bool,
}

impl ClientState {
    /// Create a new [`ClientState`] with initial values.
    ///
    /// Genesis typically passes zero `initial_pow_hash` / shielded commitment
    /// until the first finalized header is applied.
    #[must_use]
    pub fn new(
        crosslink_params: ZcashCrosslinkParameters,
        initial_bft_hash: Blake3Hash,
        initial_bft_height: u32,
        initial_pow_height: u32,
        initial_pow_hash: [u8; 32],
        finalizer_roster: Vec<FinalizerEntry>,
    ) -> Self {
        Self {
            crosslink_params,
            latest_bft_block_hash: initial_bft_hash,
            latest_bft_height: initial_bft_height,
            latest_finalized_pow_height: initial_pow_height,
            latest_finalized_pow_hash: initial_pow_hash,
            latest_shielded_commitment: [0u8; 32],
            finalizer_roster,
            is_frozen: false,
        }
    }

    /// Validate the finalizer roster is non-empty and has positive voting power.
    #[must_use]
    pub fn validate_roster(&self) -> bool {
        if self.finalizer_roster.is_empty() {
            return false;
        }
        let total_power: u64 = self.finalizer_roster.iter().map(|f| f.voting_power).sum();
        total_power > 0
    }
}

impl ZcashSerialize for ClientState {
    fn zcash_serialize<W: std::io::Write>(&self, mut writer: W) -> Result<(), std::io::Error> {
        writer.write_all(
            &self
                .crosslink_params
                .bc_confirmation_depth_sigma
                .to_le_bytes(),
        )?;
        writer.write_all(&self.crosslink_params.finalization_gap_bound.to_le_bytes())?;
        self.latest_bft_block_hash.zcash_serialize(&mut writer)?;
        writer.write_all(&self.latest_bft_height.to_le_bytes())?;
        writer.write_all(&self.latest_finalized_pow_height.to_le_bytes())?;
        writer.write_all(&self.latest_finalized_pow_hash)?;
        writer.write_all(&self.latest_shielded_commitment)?;
        writer.write_u16::<LittleEndian>(self.finalizer_roster.len() as u16)?;
        for r in &self.finalizer_roster {
            writer.write_all(&r.public_key)?;
            writer.write_all(&r.voting_power.to_le_bytes())?;
        }
        writer.write_u8(self.is_frozen as u8)?;
        Ok(())
    }
}

impl ZcashDeserialize for ClientState {
    fn zcash_deserialize<R: std::io::Read>(mut reader: R) -> Result<Self, SerializationError> {
        let mut one = [0u8; 8];
        let mut two = [0u8; 8];
        let mut three = [0u8; 4];
        let mut four = [0u8; 4];
        reader.read_exact(&mut one)?;
        reader.read_exact(&mut two)?;
        let latest_bft_block_hash = Blake3Hash::zcash_deserialize(&mut reader)?;
        reader.read_exact(&mut three)?;
        let latest_bft_height = u32::from_le_bytes(three);
        reader.read_exact(&mut four)?;
        let latest_finalized_pow_height = u32::from_le_bytes(four);
        let mut latest_finalized_pow_hash = [0u8; 32];
        reader.read_exact(&mut latest_finalized_pow_hash)?;
        let mut latest_shielded_commitment = [0u8; 32];
        reader.read_exact(&mut latest_shielded_commitment)?;

        let len = reader.read_u16::<LittleEndian>()?;
        let mut finalizer_roster: Vec<FinalizerEntry> = Vec::with_capacity(len.into());
        let mut pubkey = [0u8; 32];
        let mut five = [0u8; 8];
        for _ in 0..len {
            reader.read_exact(&mut pubkey)?;
            reader.read_exact(&mut five)?;
            finalizer_roster.push(FinalizerEntry::new(pubkey, u64::from_le_bytes(five)));
        }
        let mut is_frozen = [0u8; 1];
        reader.read_exact(&mut is_frozen)?;
        Ok(Self {
            crosslink_params: ZcashCrosslinkParameters {
                bc_confirmation_depth_sigma: u64::from_le_bytes(one),
                finalization_gap_bound: u64::from_le_bytes(two),
            },
            latest_bft_block_hash,
            latest_bft_height,
            latest_finalized_pow_height,
            latest_finalized_pow_hash,
            latest_shielded_commitment,
            finalizer_roster,
            is_frozen: is_frozen[0] != 0,
        })
    }
}
