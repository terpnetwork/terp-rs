//! Consensus state update logic for the Crosslink light client.
//!
//! After a header is verified, this module derives the new consensus
//! state and optionally updates the client state.

use cosmwasm_std::Api;

use crate::client_state::ClientState;
use crate::consensus_state::ConsensusState;
use crate::error::CrosslinkIBCError;
use crate::header::CrosslinkHeader;

/// Update the consensus state given a verified header.
///
/// Returns `(updated_bft_height, new_consensus_state, optional_updated_client_state)`.
///
/// The finalized PoW header's `commitment_bytes` is stored as
/// [`ConsensusState::shielded_commitment`] and mirrored on
/// [`ClientState::latest_shielded_commitment`], binding the shielded/auth
/// anchor to both the BFT (PoS) height and the PoW height.
///
/// [`ConsensusState::app_state_commitment`] remains zero (ZIP-222 / IBC-v2).
///
/// # Errors
///
/// Returns [`CrosslinkIBCError`] if the update logic fails.
pub fn update_consensus_state(
    api: &dyn Api,
    _consensus_state: ConsensusState,
    client_state: ClientState,
    header: &CrosslinkHeader,
) -> Result<(u32, ConsensusState, Option<ClientState>), CrosslinkIBCError> {
    let bft_block = &header.bft_block;
    let new_bft_height = bft_block.height;
    let pow_anchor = bft_block.finalization_candidate();

    let new_consensus_state = ConsensusState::v1(
        new_bft_height,
        bft_block.finalization_candidate_height,
        pow_anchor.hash,
        pow_anchor.timestamp,
        pow_anchor.commitment_bytes,
    );

    let mut new_client_state = client_state;
    new_client_state.latest_bft_block_hash = header.block_hash(api);
    new_client_state.latest_bft_height = new_bft_height;
    new_client_state.latest_finalized_pow_height = new_consensus_state.pow_anchor_height;
    new_client_state.latest_finalized_pow_hash = new_consensus_state.pow_anchor_hash;
    new_client_state.latest_shielded_commitment = new_consensus_state.shielded_commitment;

    Ok((new_bft_height, new_consensus_state, Some(new_client_state)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        BftBlock, Blake3Hash, FatPointerToBftBlock2, PROTOTYPE_PARAMETERS, PowHeader,
        ZcashDeserialize, ZcashSerialize,
    };

    #[test]
    fn test_update_consensus_state_basic() {
        let client_state = ClientState::new(
            PROTOTYPE_PARAMETERS.clone(),
            Blake3Hash([0u8; 32]),
            0,
            0,
            [0u8; 32],
            vec![],
        );
        let consensus_state = ConsensusState::v1(0, 0, [0u8; 32], 0, [0u8; 32]);

        let commitment = [0xab; 32];
        let header = PowHeader {
            hash: [1u8; 32],
            timestamp: 1_700_000_000,
            height: 100,
            commitment_bytes: commitment,
        };

        let bft_block = BftBlock {
            version: 1,
            height: 1,
            previous_block_fat_ptr: FatPointerToBftBlock2::null(),
            finalization_candidate_height: 100,
            headers: vec![header.clone(), header.clone(), header],
        };

        let crosslink_header = CrosslinkHeader {
            trusted_bft_height: 0,
            bft_block,
            fat_pointer: FatPointerToBftBlock2::null(),
        };

        let result = update_consensus_state(
            &cosmwasm_std::testing::MockApi::default(),
            consensus_state,
            client_state,
            &crosslink_header,
        );
        assert!(result.is_ok());

        let (new_height, new_consensus, updated_client) = result.unwrap();
        assert_eq!(new_height, 1);
        assert_eq!(new_consensus.bft_height, 1);
        assert_eq!(new_consensus.pow_anchor_height, 100);
        assert_eq!(new_consensus.shielded_commitment, commitment);
        assert_eq!(new_consensus.app_state_commitment, [0u8; 32]);
        assert!(updated_client.is_some());
        let client = updated_client.unwrap();
        assert_eq!(client.latest_bft_height, 1);
        assert_eq!(client.latest_finalized_pow_height, 100);
        assert_eq!(client.latest_shielded_commitment, commitment);
    }

    #[test]
    fn test_consensus_state_roundtrip_includes_shielded() {
        let cs = ConsensusState::v1(3, 120, [0x11; 32], 1_700_000_100, [0x22; 32]);
        let bytes = cs.zcash_serialize_to_vec().unwrap();
        let decoded = ConsensusState::zcash_deserialize(&bytes[..]).unwrap();
        assert_eq!(cs, decoded);
        assert_eq!(decoded.shielded_commitment, [0x22; 32]);
        assert_eq!(decoded.app_state_commitment, [0u8; 32]);
    }
}
