//! zk-jwt harness: issuer inclusion_set_root in public instances (Headscale-style).
//!
//! Pattern under test:
//! - Issuer registers a 32-byte inclusion set root (membership epoch)
//! - Authenticate public_inputs carry that root at offset 64
//! - Mismatch / missing root → reject; match + structural proof → accept
//! - Nullifier replay still enforced

use cosmwasm_std::{to_json_binary, Binary};
use cw_orch::prelude::*;
use terp_authenticator_suite::fixtures::*;
use terp_authenticator_suite::traits::AuthenticatorSudoExt;
use terp_authenticator_suite::{TerpAuthenticatorDeployData, TerpAuthenticatorSuite};
use terp_zkjwt::{build_public_inputs, circuit_ids, ZkJwtAuthPayload};

fn deploy_suite() -> TerpAuthenticatorSuite<Mock> {
    deploy_suite_with(false)
}

fn deploy_suite_with(require_registered_claim: bool) -> TerpAuthenticatorSuite<Mock> {
    let mock = Mock::new("admin");
    let admin = mock.sender_addr();
    TerpAuthenticatorSuite::deploy_on(
        mock,
        TerpAuthenticatorDeployData {
            admin: admin.clone(),
            dao: Some(admin),
            recovery_hash_alg: None,
            require_registered_claim,
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

/// Suite default issuer root is [0xAB; 32]
fn suite_root() -> [u8; 32] {
    [0xABu8; 32]
}

fn payload(
    claim: [u8; 32],
    nf: [u8; 32],
    root: Option<[u8; 32]>,
    proof: &[u8],
) -> ZkJwtAuthPayload {
    ZkJwtAuthPayload {
        issuer: "https://accounts.example.com".into(),
        claim_commitment: Binary::from(claim),
        public_inputs: build_public_inputs(&nf, &claim, root.as_ref(), None),
        proof: Binary::from(proof),
        circuit_id: circuit_ids::JWT_MEMBERSHIP.into(),
    }
}

#[test]
fn zk_jwt_inclusion_root_match_authenticates() {
    let suite = deploy_suite();
    let acct = mock_account();
    let claim = [1u8; 32];
    let nf = [2u8; 32];
    let p = payload(claim, nf, Some(suite_root()), b"proof-ok");

    assert_action(
        &suite
            .zk_jwt
            .sudo_on_auth_added(dummy_on_auth_added(
                acct.clone(),
                "jwt-1",
                Some(Binary::from(b"p")),
            ))
            .unwrap(),
        "zkjwt_on_auth_added",
    );

    assert_action(
        &suite
            .zk_jwt
            .sudo_authenticate(dummy_authentication_request(
                acct,
                "jwt-1",
                to_json_binary(&p).unwrap(),
                None,
            ))
            .unwrap(),
        "zkjwt_authenticate",
    );
}

#[test]
fn zk_jwt_inclusion_root_mismatch_rejected() {
    let suite = deploy_suite();
    let acct = mock_account();
    let claim = [1u8; 32];
    let nf = [3u8; 32];
    let wrong_root = [0xCDu8; 32];
    let p = payload(claim, nf, Some(wrong_root), b"proof");

    let _ = suite.zk_jwt.sudo_on_auth_added(dummy_on_auth_added(
        acct.clone(),
        "jwt-1",
        Some(Binary::from(b"p")),
    ));

    assert!(suite
        .zk_jwt
        .sudo_authenticate(dummy_authentication_request(
            acct,
            "jwt-1",
            to_json_binary(&p).unwrap(),
            None,
        ))
        .is_err());
}

#[test]
fn zk_jwt_missing_inclusion_root_when_issuer_requires_it() {
    let suite = deploy_suite();
    let acct = mock_account();
    let claim = [1u8; 32];
    // only nullifier||claim — no root slot
    let mut pi = vec![4u8; 32];
    pi.extend_from_slice(&claim);
    let p = ZkJwtAuthPayload {
        issuer: "https://accounts.example.com".into(),
        claim_commitment: Binary::from(claim),
        public_inputs: Binary::from(pi),
        proof: Binary::from(b"proof"),
        circuit_id: circuit_ids::JWT_MEMBERSHIP.into(),
    };

    let _ = suite.zk_jwt.sudo_on_auth_added(dummy_on_auth_added(
        acct.clone(),
        "jwt-1",
        Some(Binary::from(b"p")),
    ));

    assert!(suite
        .zk_jwt
        .sudo_authenticate(dummy_authentication_request(
            acct,
            "jwt-1",
            to_json_binary(&p).unwrap(),
            None,
        ))
        .is_err());
}

#[test]
fn zk_jwt_rotate_inclusion_root_then_old_root_fails() {
    let suite = deploy_suite();
    let acct = mock_account();
    let claim = [1u8; 32];
    let nf = [5u8; 32];
    let old = suite_root();
    let new_root = [0xEFu8; 32];

    let _ = suite.zk_jwt.sudo_on_auth_added(dummy_on_auth_added(
        acct.clone(),
        "jwt-1",
        Some(Binary::from(b"p")),
    ));

    // rotate root
    suite
        .zk_jwt
        .execute(
            &terp_zkjwt::ExecuteMsg::RotateInclusionRoot {
                issuer: "https://accounts.example.com".into(),
                inclusion_set_root: Binary::from(new_root),
            },
            &[],
        )
        .unwrap();

    // old root fails
    let p_old = payload(claim, nf, Some(old), b"proof");
    assert!(suite
        .zk_jwt
        .sudo_authenticate(dummy_authentication_request(
            acct.clone(),
            "jwt-1",
            to_json_binary(&p_old).unwrap(),
            None,
        ))
        .is_err());

    // new root ok
    let p_new = payload(claim, [6u8; 32], Some(new_root), b"proof");
    assert_action(
        &suite
            .zk_jwt
            .sudo_authenticate(dummy_authentication_request(
                acct,
                "jwt-1",
                to_json_binary(&p_new).unwrap(),
                None,
            ))
            .unwrap(),
        "zkjwt_authenticate",
    );
}

#[test]
fn zk_jwt_register_claim_binds_account() {
    let suite = deploy_suite();
    let admin = suite.zk_jwt.environment().sender_addr();
    // re-instantiate path: use execute RegisterClaim as admin sender
    let claim = [0x11u8; 32];
    suite
        .zk_jwt
        .execute(
            &terp_zkjwt::ExecuteMsg::RegisterClaim {
                claim_commitment: Binary::from(claim),
                issuer: "https://accounts.example.com".into(),
            },
            &[],
        )
        .unwrap();

    // query owner
    let owner: Option<String> = suite
        .zk_jwt
        .query(&terp_zkjwt::QueryMsg::ClaimOwner {
            claim_commitment: Binary::from(claim),
        })
        .unwrap();
    assert_eq!(owner.as_deref(), Some(admin.as_str()));
}

/// Headscale core path: require_registered_claim=true.
/// RegisterClaim → Authenticate as owner ok; wrong account / unregistered claim fail.
#[test]
fn zk_jwt_require_registered_claim_e2e() {
    let suite = deploy_suite_with(true);
    let owner = suite.zk_jwt.environment().sender_addr();
    let other = mock_account();

    let cfg: terp_zkjwt::ConfigResponse = suite
        .zk_jwt
        .query(&terp_zkjwt::QueryMsg::Config {})
        .unwrap();
    assert!(cfg.require_registered_claim);

    let claim = [0xAAu8; 32];
    suite
        .zk_jwt
        .execute(
            &terp_zkjwt::ExecuteMsg::RegisterClaim {
                claim_commitment: Binary::from(claim),
                issuer: "https://accounts.example.com".into(),
            },
            &[],
        )
        .unwrap();

    let _ = suite.zk_jwt.sudo_on_auth_added(dummy_on_auth_added(
        owner.clone(),
        "jwt-claim-1",
        Some(Binary::from(b"p")),
    ));

    // matching claim + registered owner account → ok
    let p_ok = payload(claim, [0x01u8; 32], Some(suite_root()), b"proof-claim-ok");
    assert_action(
        &suite
            .zk_jwt
            .sudo_authenticate(dummy_authentication_request(
                owner.clone(),
                "jwt-claim-1",
                to_json_binary(&p_ok).unwrap(),
                None,
            ))
            .unwrap(),
        "zkjwt_authenticate",
    );

    // same claim, different account → registered to different account
    let p_wrong_acct = payload(claim, [0x02u8; 32], Some(suite_root()), b"proof-wrong-acct");
    let err_wrong = suite
        .zk_jwt
        .sudo_authenticate(dummy_authentication_request(
            other,
            "jwt-claim-1",
            to_json_binary(&p_wrong_acct).unwrap(),
            None,
        ))
        .unwrap_err();
    let msg_wrong = format!("{err_wrong}");
    assert!(
        msg_wrong.contains("different account") || msg_wrong.contains("registered"),
        "expected claim/account mismatch, got: {msg_wrong}"
    );

    // unregistered claim → UnregisteredClaim
    let unregistered = [0xBBu8; 32];
    let p_unreg = payload(unregistered, [0x03u8; 32], Some(suite_root()), b"proof-unreg");
    let err_unreg = suite
        .zk_jwt
        .sudo_authenticate(dummy_authentication_request(
            owner,
            "jwt-claim-1",
            to_json_binary(&p_unreg).unwrap(),
            None,
        ))
        .unwrap_err();
    let msg_unreg = format!("{err_unreg}");
    assert!(
        msg_unreg.contains("not registered")
            || msg_unreg.contains("UnregisteredClaim")
            || msg_unreg.contains("unregistered"),
        "expected UnregisteredClaim, got: {msg_unreg}"
    );
}
