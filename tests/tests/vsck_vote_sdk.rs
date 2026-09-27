//! SA-VSCK: vote-sdk fail-closed matrix via `suite::auth` (auth-suite feature).
//!
//! Run:
//!   cargo test -p terp-scripts --no-default-features --features auth-suite --test vsck_vote_sdk
//!
//! Instantiates `terp-vsck` on mock deps (no Docker / no other authenticators).
//! Coordinator deny + role/circuit families + hard session-root binding.

#![cfg(feature = "auth-suite")]

use cosmwasm_std::{
    testing::{mock_dependencies, mock_env},
    to_json_binary, Addr, Binary, MessageInfo,
};
use terp_scripts::suite::auth::{
    verify_envelope, vsck_authenticate_sudo, vote_sdk_mirror, CircuitVerifyingKeys,
    VsckAuthPayload, VsckCircuit, VsckExecuteMsg, VsckInstantiateMsg, VsckRole,
};

fn vks() -> CircuitVerifyingKeys {
    CircuitVerifyingKeys {
        delegation_vk: Binary::from(b"del-vk"),
        vote_proof_vk: Binary::from(b"vote-vk"),
        share_reveal_vk: Binary::from(b"share-vk"),
    }
}

fn root() -> Binary {
    Binary::from(b"note-tree-root-32bytes___________")
}



fn payload(session: &str, role: VsckRole, circuit: VsckCircuit, nf: &[u8]) -> VsckAuthPayload {
    let mut pi = root().to_vec();
    pi.extend_from_slice(b"|ballot");
    VsckAuthPayload {
        session_id: session.into(),
        role,
        circuit,
        public_inputs: Binary::from(pi),
        proof: Binary::from(b"proof-bytes"),
        nullifier: Binary::from(nf),
    }
}

fn open_and_auth(session: &str, p: VsckAuthPayload) -> Result<cosmwasm_std::Response, String> {
    let mut deps = mock_dependencies();
    let admin = deps.api.addr_make("admin");
    let dao = deps.api.addr_make("dao");
    terp_vsck::instantiate(
        deps.as_mut(),
        mock_env(),
        MessageInfo {
            sender: admin.clone(),
            funds: vec![],
        },
        VsckInstantiateMsg {
            admin: Some(admin.to_string()),
            dao: dao.to_string(),
            circuit_vks: vks(),
        },
    )
    .map_err(|e| e.to_string())?;
    terp_vsck::execute(
        deps.as_mut(),
        mock_env(),
        MessageInfo {
            sender: admin,
            funds: vec![],
        },
        VsckExecuteMsg::OpenSession {
            session_id: session.into(),
            note_tree_root: root(),
            proposal_ref: Some("p".into()),
        },
    )
    .map_err(|e| e.to_string())?;
    let sig = to_json_binary(&p).map_err(|e| e.to_string())?;
    terp_vsck::sudo(
        deps.as_mut(),
        mock_env(),
        vsck_authenticate_sudo(sig),
    )
    .map_err(|e| e.to_string())
}

#[test]
fn suite_delegation_fail_closed() {
    let e = open_and_auth(
        "d",
        payload("d", VsckRole::Delegator, VsckCircuit::Delegation, b"n"),
    )
    .unwrap_err();
    assert!(
        e.contains("not linked") || e.contains("CircuitNotLinked"),
        "{e}"
    );
}

#[test]
fn suite_vote_proof_fail_closed() {
    let e = open_and_auth(
        "v",
        payload("v", VsckRole::Voter, VsckCircuit::VoteProof, b"n"),
    )
    .unwrap_err();
    assert!(
        e.contains("not linked") || e.contains("CircuitNotLinked"),
        "{e}"
    );
}

#[test]
fn suite_share_reveal_fail_closed() {
    let e = open_and_auth(
        "s",
        payload("s", VsckRole::Tallier, VsckCircuit::ShareReveal, b"n"),
    )
    .unwrap_err();
    assert!(
        e.contains("not linked") || e.contains("CircuitNotLinked"),
        "{e}"
    );
}

#[test]
fn suite_coordinator_denied() {
    let e = open_and_auth(
        "c",
        payload("c", VsckRole::Coordinator, VsckCircuit::VoteProof, b"n"),
    )
    .unwrap_err();
    assert!(e.contains("Coordinator") || e.contains("RoleDenied"), "{e}");
}

#[test]
fn suite_unbound_public_inputs() {
    let e = open_and_auth(
        "u",
        VsckAuthPayload {
            session_id: "u".into(),
            role: VsckRole::Voter,
            circuit: VsckCircuit::VoteProof,
            public_inputs: Binary::from(b"wrong"),
            proof: Binary::from(b"proof"),
            nullifier: Binary::from(b"n"),
        },
    )
    .unwrap_err();
    assert!(
        e.contains("note_tree_root") || e.contains("InvalidProof"),
        "{e}"
    );
}

#[test]
fn suite_reexports_envelope_and_mirror() {
    let err = vote_sdk_mirror::verify_vote_proof(
        &vks().vote_proof_vk,
        &Binary::from(b"p"),
        &root(),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        terp_scripts::suite::auth::VsckContractError::CircuitNotLinked { .. }
    ));
    let _ = verify_envelope;
}
