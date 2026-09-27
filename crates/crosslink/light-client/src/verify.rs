//! Header and misbehaviour verification for the Crosslink light client.

use cosmwasm_std::Api;

use crate::client_state::ClientState;
use crate::consensus_state::ConsensusState;
use crate::error::CrosslinkIBCError;
use crate::header::CrosslinkHeader;

/// Verify a [`CrosslinkHeader`] against the current client state.
///
/// # Errors
///
/// Returns [`CrosslinkIBCError`] if any verification step fails.
pub fn verify_header(
    api: &dyn Api,
    client_state: &ClientState,
    consensus_state: &ConsensusState,
    header: &CrosslinkHeader,
) -> Result<(), CrosslinkIBCError> {
    // 1. Client must not be frozen
    if client_state.is_frozen {
        return Err(CrosslinkIBCError::HeaderVerificationFailed(
            "client is frozen".into(),
        ));
    }

    // 2. Validate the trusted height matches our latest state
    if header.trusted_bft_height != client_state.latest_bft_height {
        return Err(CrosslinkIBCError::HeaderVerificationFailed(format!(
            "trusted height {} != latest {}",
            header.trusted_bft_height, client_state.latest_bft_height
        )));
    }

    // 3. Validate the BFT block structure (confirmation depth)
    let params = &client_state.crosslink_params;
    let bft_block = &header.bft_block;
    if bft_block.headers.len() as u64 != params.bc_confirmation_depth_sigma {
        return Err(CrosslinkIBCError::HeaderVerificationFailed(format!(
            "confirmation depth {} != expected sigma {}",
            bft_block.headers.len(),
            params.bc_confirmation_depth_sigma
        )));
    }

    // 4. Validate the fat pointer block hash matches the BFT block hash
    if !header.validate_block_commitment() {
        return Err(CrosslinkIBCError::HeaderVerificationFailed(
            "fat pointer block hash does not match BFT block hash".into(),
        ));
    }

    // 5. Batch-verify ed25519 signatures in the fat pointer.
    if !header.fat_pointer.validate_signatures(api)? {
        return Err(CrosslinkIBCError::SignatureVerificationFailed);
    }
    // On wasm32, signature verification is a no-op — the contract trusts
    // that the relayer has already verified the fat pointer signatures
    // before constructing the update message.
    #[cfg(target_arch = "wasm32")]
    let _ = &header.fat_pointer;

    // 6. The finalized PoW anchor must be strictly ahead
    let new_pow_height = bft_block.finalization_candidate_height;
    if new_pow_height <= consensus_state.pow_anchor_height {
        return Err(CrosslinkIBCError::HeaderVerificationFailed(format!(
            "new pow height {} <= current {}",
            new_pow_height, consensus_state.pow_anchor_height
        )));
    }

    Ok(())
}

/// Check whether a header indicates misbehaviour (conflicting finality).
///
/// # Errors
///
/// Returns [`CrosslinkIBCError`] if the header cannot be checked.
pub fn check_for_misbehaviour(
    client_state: &ClientState,
    consensus_state: &ConsensusState,
    header: &CrosslinkHeader,
) -> Result<bool, CrosslinkIBCError> {
    if client_state.is_frozen {
        return Ok(true);
    }
    if header.bft_block.headers.is_empty() {
        return Err(CrosslinkIBCError::HeaderVerificationFailed(
            "misbehaviour check requires a PoW finalization candidate".into(),
        ));
    }
    if header.bft_block.height != client_state.latest_bft_height
        && header.bft_block.height != consensus_state.bft_height
    {
        return Ok(false);
    }
    let candidate = header.finalization_candidate();
    let conflicts = header.block_hash() != client_state.latest_bft_block_hash
        || candidate.hash != client_state.latest_finalized_pow_hash
        || candidate.hash != consensus_state.pow_anchor_hash;
    Ok(conflicts)
}

#[cfg(test)]
mod tests {
    use cosmwasm_std::testing::mock_dependencies;

    use super::*;
    use crate::types::{
        BftBlock, Blake3Hash, FatPointerToBftBlock2, PROTOTYPE_PARAMETERS, PowHeader,
    };

    #[test]
    fn test_verify_header_frozen() {
        let mut client_state = ClientState::new(
            PROTOTYPE_PARAMETERS.clone(),
            Blake3Hash([0u8; 32]),
            0,
            0,
            [0u8; 32],
            vec![],
        );
        client_state.is_frozen = true;
        let consensus_state = ConsensusState::v1(0, 0, [0u8; 32], 0, [0u8; 32]);
        let header = CrosslinkHeader {
            trusted_bft_height: 0,
            bft_block: BftBlock {
                version: 1,
                height: 1,
                previous_block_fat_ptr: FatPointerToBftBlock2::null(),
                finalization_candidate_height: 0,
                headers: vec![],
            },
            fat_pointer: FatPointerToBftBlock2::null(),
        };
        let result = verify_header(
            &mock_dependencies().api,
            &client_state,
            &consensus_state,
            &header,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_header_invalid_trusted_height() {
        let client_state = ClientState::new(
            PROTOTYPE_PARAMETERS.clone(),
            Blake3Hash([0u8; 32]),
            0,
            0,
            [0u8; 32],
            vec![],
        );
        let consensus_state = ConsensusState::v1(0, 0, [0u8; 32], 0, [0u8; 32]);
        let header = CrosslinkHeader {
            trusted_bft_height: 999,
            bft_block: BftBlock {
                version: 1,
                height: 1,
                previous_block_fat_ptr: FatPointerToBftBlock2::null(),
                finalization_candidate_height: 0,
                headers: vec![],
            },
            fat_pointer: FatPointerToBftBlock2::null(),
        };
        let result = verify_header(
            &mock_dependencies().api,
            &client_state,
            &consensus_state,
            &header,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_block_commitment_mismatch() {
        let bft = BftBlock {
            version: 1,
            height: 1,
            previous_block_fat_ptr: FatPointerToBftBlock2::null(),
            finalization_candidate_height: 0,
            headers: vec![],
        };
        let header = CrosslinkHeader {
            trusted_bft_height: 0,
            bft_block: bft,
            fat_pointer: FatPointerToBftBlock2::null(),
        };
        // Null fat pointer has all-zero hash, which won't match the BFT block hash
        assert!(!header.validate_block_commitment());
    }

    #[test]
    fn test_verify_confirmation_depth_mismatch() {
        // PROTOTYPE_PARAMETERS has sigma=3, but we provide 0 headers
        let client_state = ClientState::new(
            PROTOTYPE_PARAMETERS.clone(),
            Blake3Hash([0u8; 32]),
            0,
            0,
            [0u8; 32],
            vec![],
        );
        let consensus_state = ConsensusState::v1(0, 0, [0u8; 32], 0, [0u8; 32]);
        let header = CrosslinkHeader {
            trusted_bft_height: 0,
            bft_block: BftBlock {
                version: 1,
                height: 1,
                previous_block_fat_ptr: FatPointerToBftBlock2::null(),
                finalization_candidate_height: 100,
                headers: vec![],
            },
            fat_pointer: FatPointerToBftBlock2::null(),
        };
        let result = verify_header(
            &mock_dependencies().api,
            &client_state,
            &consensus_state,
            &header,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("confirmation depth"));
    }

    #[test]
    fn check_for_misbehaviour_same_height_conflict() {
        let client_state = ClientState::new(
            PROTOTYPE_PARAMETERS.clone(),
            Blake3Hash([0u8; 32]),
            1,
            10,
            [0x11; 32],
            vec![],
        );
        let consensus_state = ConsensusState::v1(1, 10, [0x11; 32], 0, [0u8; 32]);
        let header = CrosslinkHeader {
            trusted_bft_height: 1,
            bft_block: BftBlock {
                version: 1,
                height: 1,
                previous_block_fat_ptr: FatPointerToBftBlock2::null(),
                finalization_candidate_height: 10,
                headers: vec![PowHeader {
                    hash: [0x22; 32],
                    timestamp: 1,
                    height: 10,
                    commitment_bytes: [0u8; 32],
                }],
            },
            fat_pointer: FatPointerToBftBlock2::null(),
        };
        assert!(check_for_misbehaviour(&client_state, &consensus_state, &header).unwrap());
    }

    #[test]
    fn check_for_misbehaviour_empty_headers_fail_closed() {
        let client_state = ClientState::new(
            PROTOTYPE_PARAMETERS.clone(),
            Blake3Hash([0u8; 32]),
            1,
            10,
            [0x11; 32],
            vec![],
        );
        let consensus_state = ConsensusState::v1(1, 10, [0x11; 32], 0, [0u8; 32]);
        let header = CrosslinkHeader {
            trusted_bft_height: 1,
            bft_block: BftBlock {
                version: 1,
                height: 1,
                previous_block_fat_ptr: FatPointerToBftBlock2::null(),
                finalization_candidate_height: 10,
                headers: vec![],
            },
            fat_pointer: FatPointerToBftBlock2::null(),
        };
        assert!(check_for_misbehaviour(&client_state, &consensus_state, &header).is_err());
    }
}
