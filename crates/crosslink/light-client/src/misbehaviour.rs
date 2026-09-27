//! Misbehaviour detection for the Crosslink light client.
//!
//! Detects equivocation: two conflicting fat pointers at the same BFT height
//! signed by the same finalizer set.

use crate::client_state::ClientState;
use crate::consensus_state::ConsensusState;
use crate::error::CrosslinkIBCError;
use crate::header::CrosslinkHeader;

/// Two headers at the same BFT height with a different BFT hash or PoW
/// finalization candidate are equivocation evidence.
#[must_use]
pub fn headers_equivocate(header_1: &CrosslinkHeader, header_2: &CrosslinkHeader) -> bool {
    if header_1.bft_block.height != header_2.bft_block.height {
        return false;
    }
    if header_1.bft_block.headers.is_empty() || header_2.bft_block.headers.is_empty() {
        return false;
    }
    header_1.block_hash() != header_2.block_hash()
        || header_1.finalization_candidate().hash != header_2.finalization_candidate().hash
}

/// Verify that two headers constitute misbehaviour (equivocation).
///
/// Fail-closed: missing headers or incomparable heights are errors, not
/// "no misbehaviour". Confirmed same-height disagreement is
/// [`CrosslinkIBCError::MisbehaviourDetected`]. Identical headers succeed
/// (evidence is not equivocation).
///
/// # Errors
///
/// Returns [`CrosslinkIBCError::MisbehaviourDetected`] if misbehaviour is confirmed,
/// or [`CrosslinkIBCError::HeaderVerificationFailed`] if the pair cannot be compared.
pub fn verify_misbehaviour(
    _client_state: &ClientState,
    _consensus_state: &ConsensusState,
    header_1: &CrosslinkHeader,
    header_2: &CrosslinkHeader,
) -> Result<(), CrosslinkIBCError> {
    if header_1.bft_block.headers.is_empty() || header_2.bft_block.headers.is_empty() {
        return Err(CrosslinkIBCError::HeaderVerificationFailed(
            "misbehaviour evidence requires non-empty PoW headers".into(),
        ));
    }
    if header_1.bft_block.height != header_2.bft_block.height {
        return Err(CrosslinkIBCError::HeaderVerificationFailed(format!(
            "misbehaviour headers must share BFT height ({} vs {})",
            header_1.bft_block.height, header_2.bft_block.height
        )));
    }
    if headers_equivocate(header_1, header_2) {
        return Err(CrosslinkIBCError::MisbehaviourDetected);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BftBlock, Blake3Hash, FatPointerToBftBlock2, PROTOTYPE_PARAMETERS, PowHeader};

    fn header(height: u32, pow_hash: u8) -> CrosslinkHeader {
        CrosslinkHeader {
            trusted_bft_height: height,
            bft_block: BftBlock {
                version: 1,
                height,
                previous_block_fat_ptr: FatPointerToBftBlock2::null(),
                finalization_candidate_height: 10,
                headers: vec![PowHeader {
                    hash: [pow_hash; 32],
                    timestamp: 1,
                    height: 10,
                    commitment_bytes: [0u8; 32],
                }],
            },
            fat_pointer: FatPointerToBftBlock2::null(),
        }
    }

    fn client() -> ClientState {
        ClientState::new(
            PROTOTYPE_PARAMETERS.clone(),
            Blake3Hash([0u8; 32]),
            1,
            10,
            [1u8; 32],
            vec![],
        )
    }

    #[test]
    fn same_height_different_pow_is_misbehaviour() {
        let err = verify_misbehaviour(
            &client(),
            &ConsensusState::v1(1, 10, [1u8; 32], 0, [0u8; 32]),
            &header(1, 0xaa),
            &header(1, 0xbb),
        )
        .unwrap_err();
        assert!(matches!(err, CrosslinkIBCError::MisbehaviourDetected));
    }

    #[test]
    fn same_headers_are_not_misbehaviour() {
        let h = header(1, 0xaa);
        verify_misbehaviour(
            &client(),
            &ConsensusState::v1(1, 10, [0xaa; 32], 0, [0u8; 32]),
            &h,
            &h,
        )
        .unwrap();
    }

    #[test]
    fn different_heights_fail_closed() {
        let err = verify_misbehaviour(
            &client(),
            &ConsensusState::v1(1, 10, [1u8; 32], 0, [0u8; 32]),
            &header(1, 0xaa),
            &header(2, 0xbb),
        )
        .unwrap_err();
        assert!(matches!(err, CrosslinkIBCError::HeaderVerificationFailed(_)));
    }
}