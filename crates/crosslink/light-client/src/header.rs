//! Header types for the Crosslink light client update messages.
//!
//! A [`CrosslinkHeader`] bundles a BFT block with its signed fat pointer
//! so the light client can verify TFL finality.

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use cosmwasm_std::Api;

use crate::types::{
    BftBlock, Blake3Hash, FatPointerToBftBlock2, PowHeader, SerializationError, ZcashDeserialize,
    ZcashSerialize,
};

/// The client update message payload.
///
/// Contains everything needed to validate a new TFL finality root:
/// - The BFT block (carries the PoW anchor headers)
/// - The fat pointer (signed vote bundle proving TFL consensus)
/// - A trusted height to anchor verification from
#[derive(Clone, Debug, PartialEq)]
pub struct CrosslinkHeader {
    /// The BFT height we trust as the anchor point.
    pub trusted_bft_height: u32,
    /// The new BFT block being proposed for finality.
    pub bft_block: BftBlock,
    /// The fat pointer with aggregated signatures proving TFL finality.
    pub fat_pointer: FatPointerToBftBlock2,
}

impl CrosslinkHeader {
    /// Hash of the BFT block in this header.
    #[must_use]
    pub fn block_hash(&self, api: &dyn Api) -> Blake3Hash {
        self.bft_block.blake3_hash(api)
    }

    /// The PoW anchor block (finalization candidate) from the BFT block.
    #[must_use]
    pub fn finalization_candidate(&self) -> &PowHeader {
        self.bft_block.finalization_candidate()
    }

    /// Verify the fat pointer's block hash matches the BFT block hash.
    #[must_use]
    pub fn validate_block_commitment(&self, api: &dyn Api) -> bool {
        let block_hash = self.block_hash(api);
        let pointed_hash = self.fat_pointer.points_at_block_hash();
        block_hash == pointed_hash
    }
}

impl ZcashSerialize for CrosslinkHeader {
    fn zcash_serialize<W: std::io::Write>(&self, mut writer: W) -> Result<(), std::io::Error> {
        writer.write_u32::<LittleEndian>(self.trusted_bft_height)?;
        self.bft_block.zcash_serialize(&mut writer)?;
        self.fat_pointer.zcash_serialize(writer)?;
        Ok(())
    }
}

impl ZcashDeserialize for CrosslinkHeader {
    fn zcash_deserialize<R: std::io::Read>(mut reader: R) -> Result<Self, SerializationError> {
        let trusted_bft_height = reader.read_u32::<LittleEndian>()?;
        let bft_block = BftBlock::zcash_deserialize(&mut reader)?;
        let fat_pointer = FatPointerToBftBlock2::zcash_deserialize(&mut reader)?;
        Ok(CrosslinkHeader {
            trusted_bft_height,
            bft_block,
            fat_pointer,
        })
    }
}
