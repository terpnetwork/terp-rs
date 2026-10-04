//! Mock coverage: zk scaffolds + zk-jwt design + vsck private voting auth.

use cosmwasm_std::{Binary, to_json_binary};
use cw_orch::prelude::*;
use terp_authenticator_suite::fixtures::*;
use terp_authenticator_suite::traits::AuthenticatorSudoExt;
use terp_authenticator_suite::{TerpAuthenticatorDeployData, TerpAuthenticatorSuite};
use terp_zkjwt::{circuit_ids, ZkJwtAuthPayload};
use terp_vsck::{VsckAuthPayload, VsckCircuit, VsckRole};

fn deploy_suite() -> TerpAuthenticatorSuite<Mock> {
    let mock = Mock::new("admin");
    let admin = mock.sender_addr();
    TerpAuthenticatorSuite::deploy_on(
        mock,
        TerpAuthenticatorDeployData {
            admin: admin.clone(),
            dao: Some(admin),
            recovery_hash_alg: None,
            require_registered_claim: false,
            eth_signer: terp_authenticator_suite::ETH_GOLDEN_SIGNER.into(),
        },
    )
    .expect("deploy suite")
}

fn assert_action(res: &<Mock as TxHandler>::Response, action: &str) {
    let found = res.events.iter().any(|e| {
        e.attributes
            .iter()
            .any(|a| a.key == "action" && a.value == action)
    });
    assert!(found, "expected action={action}, events={:#?}", res.events);
}

#[test]
fn zk_jwt_authenticate_with_proof_payload() {
    let suite = deploy_suite();
    let acct = mock_account();

    // OnAuthAdded requires params
    assert!(suite
        .zk_jwt
        .sudo_on_auth_added(dummy_on_auth_added(acct.clone(), "jwt-1", None))
        .is_err());

    assert_action(
        &suite
            .zk_jwt
            .sudo_on_auth_added(dummy_on_auth_added(
                acct.clone(),
                "jwt-1",
                Some(Binary::from(b"cfg")),
            ))
            .unwrap(),
        "zkjwt_on_auth_added",
    );

    // public_inputs: nullifier(32) || claim(32) || inclusion_set_root(32)
    // Suite issuer root is [0xAB; 32]
    let claim = [1u8; 32];
    let nf = [2u8; 32];
    let root = [0xABu8; 32];
    let public_inputs = terp_zkjwt::build_public_inputs(&nf, &claim, Some(&root), None);
    let payload = ZkJwtAuthPayload {
        issuer: "https://accounts.example.com".into(),
        claim_commitment: Binary::from(claim),
        public_inputs,
        proof: Binary::from(b"proof-bytes"),
        circuit_id: circuit_ids::JWT_MEMBERSHIP.into(),
    };
    let sig = to_json_binary(&payload).unwrap();

    assert_action(
        &suite
            .zk_jwt
            .sudo_authenticate(dummy_authentication_request(
                acct.clone(),
                "jwt-1",
                sig.clone(),
                None,
            ))
            .unwrap(),
        "zkjwt_authenticate",
    );

    // nullifier replay rejected on second Authenticate
    assert!(suite
        .zk_jwt
        .sudo_authenticate(dummy_authentication_request(
            acct.clone(),
            "jwt-1",
            sig,
            None,
        ))
        .is_err());

    // empty proof rejected
    let bad = ZkJwtAuthPayload {
        proof: Binary::default(),
        public_inputs: terp_zkjwt::build_public_inputs(&[3u8; 32], &claim, Some(&root), None),
        claim_commitment: Binary::from(claim),
        issuer: "https://accounts.example.com".into(),
        circuit_id: circuit_ids::JWT_MEMBERSHIP.into(),
    };
    assert!(suite
        .zk_jwt
        .sudo_authenticate(dummy_authentication_request(
            acct,
            "jwt-1",
            to_json_binary(&bad).unwrap(),
            None,
        ))
        .is_err());
}

#[test]
fn vsck_voter_auth_after_open_session() {
    let suite = deploy_suite();
    let acct = mock_account();
    let root = Binary::from(b"note-tree-root-32bytes___________");

    suite
        .vsck
        .execute(
            &terp_vsck::ExecuteMsg::OpenSession {
                session_id: "sess-1".into(),
                note_tree_root: root.clone(),
                proposal_ref: Some("prop-1".into()),
            },
            &[],
        )
        .unwrap();

    let mut public_inputs = root.to_vec();
    public_inputs.extend_from_slice(b"|ballot");
    let payload = VsckAuthPayload {
        session_id: "sess-1".into(),
        role: VsckRole::Voter,
        circuit: VsckCircuit::VoteProof,
        public_inputs: Binary::from(public_inputs),
        proof: Binary::from(b"vote-proof"),
        nullifier: Binary::from(b"nf-1"),
    };

    assert_action(
        &suite
            .vsck
            .sudo_on_auth_added(dummy_on_auth_added(
                acct.clone(),
                "vsck-1",
                Some(Binary::from(b"p")),
            ))
            .unwrap(),
        "vsck_on_auth_added",
    );

    let auth_err = suite
        .vsck
        .sudo_authenticate(dummy_authentication_request(
            acct.clone(),
            "vsck-1",
            to_json_binary(&payload).unwrap(),
            None,
        ))
        .unwrap_err();
    assert!(
        auth_err.to_string().contains("CircuitNotLinked")
            || auth_err.to_string().contains("not linked"),
        "{auth_err}"
    );

    assert_action(
        &suite
            .vsck
            .sudo_track(dummy_track_request(
                acct,
                "vsck-1",
                Some(to_json_binary(&payload).unwrap()),
            ))
            .unwrap(),
        "vsck_track",
    );
}

#[test]
fn suite_coexistence_deploy() {
    let suite = deploy_suite();
    let addrs = [
        suite.zk_jwt.addr_str().unwrap(),
        suite.vsck.addr_str().unwrap(),
    ];
    assert_ne!(addrs[0], addrs[1]);
}
