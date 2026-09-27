//! Settlement head advance — continuity + structural verify (P0).

use crate::error::ContractError;
use crate::types::{
    lanes_canonical, pack_lanes_hex, ClientState, ConsensusState, SettlementUpdate,
};

/// Apply a SettlementUpdate to (client, consensus). Returns new consensus.
pub fn apply_settlement(
    client: &ClientState,
    head: &ConsensusState,
    update: &SettlementUpdate,
) -> Result<(ClientState, ConsensusState), ContractError> {
    if let Some(fz) = client.frozen_height {
        return Err(ContractError::Frozen(fz));
    }
    if update.num_turns == 0 {
        return Err(ContractError::ZeroTurns);
    }
    if !lanes_canonical(&update.genesis_root)
        || !lanes_canonical(&update.final_root)
        || !lanes_canonical(&update.chain_digest)
    {
        return Err(ContractError::InvalidLane);
    }
    if update.genesis_root != head.proven_lanes {
        return Err(ContractError::ContinuityBroken);
    }
    if update.proof.len() != 8
        || update.commitments.len() != 2
        || update.commitment_pok.len() != 2
    {
        return Err(ContractError::VerifyFailed(
            "proof envelope length mismatch (need 8/2/2)".into(),
        ));
    }

    verify_proof(client, update)?;

    let new_height = head
        .proven_height
        .checked_add(update.num_turns as u64)
        .ok_or_else(|| ContractError::VerifyFailed("height overflow".into()))?;

    let new_consensus = ConsensusState {
        version: 0,
        kind: crate::types::ConsensusKind::B,
        proven_lanes: update.final_root.clone(),
        proven_root_key: pack_lanes_hex(&update.final_root)
            .map_err(|_| ContractError::InvalidLane)?,
        proven_height: new_height,
    };
    let mut new_client = client.clone();
    new_client.latest_height = new_height;

    Ok((new_client, new_consensus))
}

fn verify_proof(client: &ClientState, update: &SettlementUpdate) -> Result<(), ContractError> {
    crate::types::parse_vk_hash(&client.settlement_vk_hash)
        .map_err(|_| ContractError::InvalidVkHash)?;

    #[cfg(feature = "mock-verify")]
    {
        for p in update
            .proof
            .iter()
            .chain(update.commitments.iter())
            .chain(update.commitment_pok.iter())
        {
            if p.is_empty() {
                return Err(ContractError::VerifyFailed("empty proof coordinate".into()));
            }
        }
        if update.proof[0] == "0" {
            return Err(ContractError::VerifyFailed("mock forge reject".into()));
        }
        return Ok(());
    }

    #[cfg(not(feature = "mock-verify"))]
    {
        let _ = update;
        Err(ContractError::VerifyFailed(
            "groth16-verify feature not enabled".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ClaimCeiling, TrustLevel};

    fn client_head() -> (ClientState, ConsensusState) {
        let genesis = vec![1u32, 2, 3, 4, 5, 6, 7, 8];
        let cs = ClientState {
            client_schema_version: 1,
            chain_id: "terp-local".into(),
            federation_id: None,
            trust_level: TrustLevel::SettlementWrapOnly,
            settlement_vk_hash: hex::encode([1u8; 32]),
            genesis_lanes: genesis.clone(),
            latest_height: 0,
            frozen_height: None,
            allow_gate_off_finality: false,
            claim_ceiling: ClaimCeiling::SettlementOnly,
            auth_min_proven_height: Some(0),
            auth_require_root_in_history: true,
        };
        let head = ConsensusState {
            version: 0,
            kind: crate::types::ConsensusKind::B,
            proven_lanes: genesis.clone(),
            proven_root_key: pack_lanes_hex(&genesis).unwrap(),
            proven_height: 0,
        };
        (cs, head)
    }

    fn ok_update(genesis: Vec<u32>, final_root: Vec<u32>, turns: u32) -> SettlementUpdate {
        SettlementUpdate {
            proof: (1..=8).map(|i| format!("{i}")).collect(),
            commitments: vec!["1".into(), "2".into()],
            commitment_pok: vec!["3".into(), "4".into()],
            genesis_root: genesis,
            final_root,
            num_turns: turns,
            chain_digest: vec![9, 10, 11, 12, 13, 14, 15, 16],
        }
    }

    #[test]
    fn advances_height_on_continuous_settle() {
        let (c, h) = client_head();
        let final_root = vec![10u32, 11, 12, 13, 14, 15, 16, 17];
        let (c2, h2) =
            apply_settlement(&c, &h, &ok_update(h.proven_lanes.clone(), final_root.clone(), 3))
                .unwrap();
        assert_eq!(h2.proven_height, 3);
        assert_eq!(c2.latest_height, 3);
        assert_eq!(h2.proven_lanes, final_root);
    }

    #[test]
    fn rejects_continuity_break() {
        let (c, h) = client_head();
        let err = apply_settlement(
            &c,
            &h,
            &ok_update(vec![99u32; 8], vec![1u32; 8], 1),
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::ContinuityBroken));
    }

    #[test]
    fn rejects_zero_turns() {
        let (c, h) = client_head();
        let err = apply_settlement(&c, &h, &ok_update(h.proven_lanes.clone(), vec![1u32; 8], 0))
            .unwrap_err();
        assert!(matches!(err, ContractError::ZeroTurns));
    }

    #[test]
    fn mock_forge_marker() {
        let (c, h) = client_head();
        let mut u = ok_update(h.proven_lanes.clone(), vec![1u32; 8], 1);
        u.proof[0] = "0".into();
        let err = apply_settlement(&c, &h, &u).unwrap_err();
        assert!(matches!(err, ContractError::VerifyFailed(_)));
    }
}
