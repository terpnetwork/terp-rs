//! VSCK proof verification hooks aligned with vote-sdk circuit families.
//!
//! vote-sdk (`shielded-vote-circuits`) exposes:
//! - `delegation` — prove voting power / note membership
//! - `vote_proof` — prove ballot well-formedness without revealing choice
//! - `share_reveal` — ceremony share open for tally
//!
//! Policy (role ↔ circuit, VK presence, session-root binding) always runs.
//! Cryptographic verify is **fail-closed**: without the `vote-circuits` feature
//! (and a linked vote-sdk crate) [`DefaultVsckVerifier`] returns
//! [`ContractError::CircuitNotLinked`]. Stub proofs never authenticate.

use cosmwasm_std::Deps;

use crate::error::ContractError;
use crate::msg::{CircuitVerifyingKeys, VsckAuthPayload, VsckCircuit, VsckRole};
use crate::state::Session;

pub trait VsckVerifier {
    fn verify(
        &self,
        deps: Deps,
        vks: &CircuitVerifyingKeys,
        session: &Session,
        payload: &VsckAuthPayload,
    ) -> Result<(), ContractError>;
}

pub struct DefaultVsckVerifier;

impl VsckVerifier for DefaultVsckVerifier {
    fn verify(
        &self,
        _deps: Deps,
        vks: &CircuitVerifyingKeys,
        session: &Session,
        payload: &VsckAuthPayload,
    ) -> Result<(), ContractError> {
        verify_envelope(vks, session, payload)?;
        vote_sdk_mirror::verify_circuit(vks, payload)
    }
}

/// Role/circuit matrix, VKs, and hard session-root binding (no soft skip).
pub fn verify_envelope(
    vks: &CircuitVerifyingKeys,
    session: &Session,
    payload: &VsckAuthPayload,
) -> Result<(), ContractError> {
    if !session.open {
        return Err(ContractError::InvalidProof {
            reason: "session closed".into(),
        });
    }
    if payload.proof.is_empty() || payload.public_inputs.is_empty() {
        return Err(ContractError::InvalidProof {
            reason: "empty proof or public_inputs".into(),
        });
    }
    if payload.nullifier.is_empty() {
        return Err(ContractError::InvalidProof {
            reason: "empty nullifier".into(),
        });
    }

    match (&payload.role, &payload.circuit) {
        (VsckRole::Delegator, VsckCircuit::Delegation) => {
            if vks.delegation_vk.is_empty() {
                return Err(ContractError::InvalidProof {
                    reason: "delegation_vk missing".into(),
                });
            }
        }
        (VsckRole::Voter, VsckCircuit::VoteProof) => {
            if vks.vote_proof_vk.is_empty() {
                return Err(ContractError::InvalidProof {
                    reason: "vote_proof_vk missing".into(),
                });
            }
        }
        (VsckRole::Tallier, VsckCircuit::ShareReveal) => {
            if vks.share_reveal_vk.is_empty() {
                return Err(ContractError::InvalidProof {
                    reason: "share_reveal_vk missing".into(),
                });
            }
        }
        (VsckRole::Coordinator, _) => {
            return Err(ContractError::RoleDenied {
                role: "Coordinator".into(),
            });
        }
        (role, circuit) => {
            return Err(ContractError::InvalidProof {
                reason: format!("role/circuit mismatch: {role:?} vs {circuit:?}"),
            });
        }
    }

    if session.note_tree_root.is_empty() {
        return Err(ContractError::InvalidProof {
            reason: "session note_tree_root empty".into(),
        });
    }
    // Production instance layout: note_tree_root is a prefix of public_inputs.
    if !payload
        .public_inputs
        .as_slice()
        .starts_with(session.note_tree_root.as_slice())
    {
        return Err(ContractError::InvalidProof {
            reason: "public_inputs do not bind session note_tree_root".into(),
        });
    }
    Ok(())
}

/// vote-sdk API surface. Halo2 bodies land when `shielded-vote-circuits` is a
/// workspace dep; until then every family is fail-closed.
pub mod vote_sdk_mirror {
    use super::*;

    pub fn verify_circuit(
        vks: &CircuitVerifyingKeys,
        payload: &VsckAuthPayload,
    ) -> Result<(), ContractError> {
        match payload.circuit {
            VsckCircuit::Delegation => {
                verify_delegation(&vks.delegation_vk, &payload.proof, &payload.public_inputs)
            }
            VsckCircuit::VoteProof => {
                verify_vote_proof(&vks.vote_proof_vk, &payload.proof, &payload.public_inputs)
            }
            VsckCircuit::ShareReveal => {
                verify_share_reveal(&vks.share_reveal_vk, &payload.proof, &payload.public_inputs)
            }
        }
    }

    pub fn verify_delegation(
        _vk: &cosmwasm_std::Binary,
        _proof: &cosmwasm_std::Binary,
        _public_inputs: &cosmwasm_std::Binary,
    ) -> Result<(), ContractError> {
        fail_closed(VsckCircuit::Delegation)
    }

    pub fn verify_vote_proof(
        _vk: &cosmwasm_std::Binary,
        _proof: &cosmwasm_std::Binary,
        _public_inputs: &cosmwasm_std::Binary,
    ) -> Result<(), ContractError> {
        fail_closed(VsckCircuit::VoteProof)
    }

    pub fn verify_share_reveal(
        _vk: &cosmwasm_std::Binary,
        _proof: &cosmwasm_std::Binary,
        _public_inputs: &cosmwasm_std::Binary,
    ) -> Result<(), ContractError> {
        fail_closed(VsckCircuit::ShareReveal)
    }

    fn fail_closed(circuit: VsckCircuit) -> Result<(), ContractError> {
        // `vote-circuits` does not link shielded-vote-circuits. A build with
        // that feature still rejects stub proofs.
        let _ = cfg!(feature = "vote-circuits");
        Err(ContractError::CircuitNotLinked {
            circuit: format!("{circuit:?}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::{testing::mock_dependencies, Binary};

    fn vks() -> CircuitVerifyingKeys {
        CircuitVerifyingKeys {
            delegation_vk: Binary::from(b"del-vk"),
            vote_proof_vk: Binary::from(b"vote-vk"),
            share_reveal_vk: Binary::from(b"share-vk"),
        }
    }

    fn session(open: bool) -> Session {
        Session {
            session_id: "s1".into(),
            note_tree_root: Binary::from(b"root-32-bytes___________________"),
            proposal_ref: None,
            open,
        }
    }

    fn payload(role: VsckRole, circuit: VsckCircuit) -> VsckAuthPayload {
        let mut pi = session(true).note_tree_root.to_vec();
        pi.extend_from_slice(b"|inst");
        VsckAuthPayload {
            session_id: "s1".into(),
            role,
            circuit,
            public_inputs: Binary::from(pi),
            proof: Binary::from(b"proof"),
            nullifier: Binary::from(b"nf"),
        }
    }

    fn check(role: VsckRole, circuit: VsckCircuit) -> Result<(), ContractError> {
        let deps = mock_dependencies();
        DefaultVsckVerifier.verify(deps.as_ref(), &vks(), &session(true), &payload(role, circuit))
    }

    #[test]
    fn delegation_fail_closed() {
        let err = check(VsckRole::Delegator, VsckCircuit::Delegation).unwrap_err();
        assert!(matches!(
            err,
            ContractError::CircuitNotLinked { circuit } if circuit == "Delegation"
        ));
    }

    #[test]
    fn vote_proof_fail_closed() {
        let err = check(VsckRole::Voter, VsckCircuit::VoteProof).unwrap_err();
        assert!(matches!(
            err,
            ContractError::CircuitNotLinked { circuit } if circuit == "VoteProof"
        ));
    }

    #[test]
    fn share_reveal_fail_closed() {
        let err = check(VsckRole::Tallier, VsckCircuit::ShareReveal).unwrap_err();
        assert!(matches!(
            err,
            ContractError::CircuitNotLinked { circuit } if circuit == "ShareReveal"
        ));
    }

    #[test]
    fn coordinator_denied_all_circuits() {
        for c in [
            VsckCircuit::Delegation,
            VsckCircuit::VoteProof,
            VsckCircuit::ShareReveal,
        ] {
            let err = check(VsckRole::Coordinator, c).unwrap_err();
            assert!(matches!(err, ContractError::RoleDenied { role } if role == "Coordinator"));
        }
    }

    #[test]
    fn role_circuit_mismatch() {
        let err = check(VsckRole::Delegator, VsckCircuit::VoteProof).unwrap_err();
        match err {
            ContractError::InvalidProof { reason } => {
                assert!(reason.contains("mismatch"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unbound_public_inputs_hard_fail() {
        let deps = mock_dependencies();
        let mut p = payload(VsckRole::Voter, VsckCircuit::VoteProof);
        p.public_inputs = Binary::from(b"not-the-root");
        let err = DefaultVsckVerifier
            .verify(deps.as_ref(), &vks(), &session(true), &p)
            .unwrap_err();
        match err {
            ContractError::InvalidProof { reason } => {
                assert!(reason.contains("note_tree_root"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn envelope_ok_before_circuit() {
        verify_envelope(
            &vks(),
            &session(true),
            &payload(VsckRole::Voter, VsckCircuit::VoteProof),
        )
        .unwrap();
    }
}
