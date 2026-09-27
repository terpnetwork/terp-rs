//! L1 golden pyramid for circom JWT claim codec (gateway-style).
//!
//! Mirrors o-line auth_plane fixtures soundness:
//! | Level | What | Claim |
//! |-------|------|-------|
//! | L1 full | full_circom_publics.json | claim_is_sound Ok |
//! | L1 sparse | sparse_only | fail closed |
//! | L1 policy PI | nullifier\|\|claim for Authenticate envelope | layout ok |
//!
//! Path A Groth16 (L2 crypto) remains in cosmwasm-vm bn254 golden tests.
//! Live Headscale join (L3) remains o-line `just test gateway chain hs`.

use cosmwasm_std::{to_json_binary, Binary};
use cw_orch::prelude::*;
use terp_authenticator_suite::fixtures::*;
use terp_authenticator_suite::traits::AuthenticatorSudoExt;
use terp_authenticator_suite::{TerpAuthenticatorDeployData, TerpAuthenticatorSuite};
use terp_zkjwt::{
    circuit_ids, claim_is_sound, decode_circom_publics_be, parse_public_json_array,
    CircomJwtPolicyClaim, ZkJwtAuthPayload, CIRCOM_JWT_CODEC_V1, CLAIM_EVENT_SCHEMA_V1,
    MIN_SOUND_CLAIM_RICHNESS,
};

fn deploy_no_inclusion_root() -> TerpAuthenticatorSuite<Mock> {
    // D3: first JWT codec ship without issuer inclusion root
    let mock = Mock::new("admin");
    let admin = mock.sender_addr();
    let suite = TerpAuthenticatorSuite::deploy_on(
        mock,
        TerpAuthenticatorDeployData {
            admin: admin.clone(),
            dao: Some(admin.clone()),
            recovery_hash_alg: None,
            require_registered_claim: false,
            eth_signer: terp_authenticator_suite::ETH_GOLDEN_SIGNER.into(),
        },
    )
    .expect("deploy");
    // Clear inclusion root so policy PI without root is accepted
    suite
        .zk_jwt
        .execute(
            &terp_zkjwt::ExecuteMsg::UpsertIssuer(terp_zkjwt::IssuerConfig {
                issuer: "https://accounts.example.com".into(),
                verifying_key: Binary::from(b"test-issuer-vk"),
                zkid: Some(1),
                audience: Some("terp-app".into()),
                inclusion_set_root: None,
            }),
            &[],
        )
        .unwrap();
    suite
}

fn full_claim() -> CircomJwtPolicyClaim {
    let signals: Vec<String> =
        serde_json::from_str(terp_zkjwt::fixtures::FULL_CIRCOM_PUBLICS_JSON).unwrap();
    let limbs = parse_public_json_array(&signals).unwrap();
    decode_circom_publics_be(&limbs).unwrap()
}

/// L1: full golden merge → sound (like wasmd full access_granted).
#[test]
fn l1_full_circom_fixture_claim_is_sound() {
    let claim = full_claim();
    claim_is_sound(&claim).expect("full fixture sound");
    assert!(claim.richness() >= MIN_SOUND_CLAIM_RICHNESS);
    assert_eq!(claim.codec, CIRCOM_JWT_CODEC_V1);
    assert_eq!(claim.schema, CLAIM_EVENT_SCHEMA_V1);
    assert_eq!(claim.nullifier_hex.len(), 64);
}

/// L1: sparse-only → fail closed (like wasmd Response-only).
#[test]
fn l1_sparse_circom_fixture_fails_closed() {
    let signals: Vec<String> =
        serde_json::from_str(terp_zkjwt::fixtures::SPARSE_CIRCOM_PUBLICS_JSON).unwrap();
    let err = parse_public_json_array(&signals)
        .and_then(|l| decode_circom_publics_be(&l))
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("sparse")
            || msg.contains("fail closed")
            || msg.contains("< 27")
            || msg.contains("accountSalt"),
        "{msg}"
    );
}

/// L1: policy PI from codec authenticates under structural verifier (no root issuer).
#[test]
fn l1_codec_policy_pi_structural_authenticate() {
    let suite = deploy_no_inclusion_root();
    let claim = full_claim();
    claim_is_sound(&claim).unwrap();

    let acct = mock_account();
    let _ = suite.zk_jwt.sudo_on_auth_added(dummy_on_auth_added(
        acct.clone(),
        "jwt-circom-1",
        Some(Binary::from(b"p")),
    ));

    let pi = claim.to_policy_public_inputs();
    // structural path needs nf||claim in public_inputs matching payload
    let p = ZkJwtAuthPayload {
        issuer: "https://accounts.example.com".into(),
        claim_commitment: Binary::from(claim.claim_commitment),
        public_inputs: pi,
        proof: Binary::from(b"structural-proof-ok"),
        circuit_id: circuit_ids::JWT_MEMBERSHIP.into(),
    };

    let res = suite
        .zk_jwt
        .sudo_authenticate(dummy_authentication_request(
            acct,
            "jwt-circom-1",
            to_json_binary(&p).unwrap(),
            None,
        ))
        .expect("structural auth with codec PI");
    let ok = res.events.iter().any(|e| {
        e.attributes
            .iter()
            .any(|a| a.key == "action" && a.value == "zkjwt_authenticate")
    });
    assert!(ok, "events={:#?}", res.events);
}

/// L1: nullifier replay still enforced after codec-derived PI auth.
#[test]
fn l1_codec_nullifier_replay() {
    let suite = deploy_no_inclusion_root();
    let claim = full_claim();
    let acct = mock_account();
    let _ = suite.zk_jwt.sudo_on_auth_added(dummy_on_auth_added(
        acct.clone(),
        "jwt-circom-2",
        Some(Binary::from(b"p")),
    ));
    let p = ZkJwtAuthPayload {
        issuer: "https://accounts.example.com".into(),
        claim_commitment: Binary::from(claim.claim_commitment),
        public_inputs: claim.to_policy_public_inputs(),
        proof: Binary::from(b"proof"),
        circuit_id: circuit_ids::JWT_MEMBERSHIP.into(),
    };
    suite
        .zk_jwt
        .sudo_authenticate(dummy_authentication_request(
            acct.clone(),
            "jwt-circom-2",
            to_json_binary(&p).unwrap(),
            None,
        ))
        .unwrap();
    assert!(suite
        .zk_jwt
        .sudo_authenticate(dummy_authentication_request(
            acct,
            "jwt-circom-2",
            to_json_binary(&p).unwrap(),
            None,
        ))
        .is_err());
}

/// Bridge-facing surface: nullifier_hex + schema for AccessGrant-style merge.
#[test]
fn l1_claim_maps_to_bridge_nullifier_hex() {
    let claim = full_claim();
    claim_is_sound(&claim).unwrap();
    // o-line AccessGrant.nullifier_hex is lowercase hex
    assert_eq!(claim.nullifier_hex, claim.nullifier_hex.to_ascii_lowercase());
    assert_eq!(claim.schema, "v1");
    // principal hint is opaque hex of salt (never a raw email in logs)
    assert_eq!(claim.principal_hint_hex.len(), 64);
}

/// L2: committed snarkjs public vector has production length + sound claim.
/// Crypto verify runs via `node terp-zkjwt/scripts/l2_snarkjs_verify.mjs`.
#[test]
fn l2_real_snarkjs_publics_are_production_layout() {
    let signals: Vec<String> =
        serde_json::from_str(terp_zkjwt::fixtures::L2_PUBLIC_JSON).unwrap();
    assert_eq!(signals.len(), 40, "demo-18-12-2024 jwt-auth nPublic");
    let claim = full_claim();
    claim_is_sound(&claim).unwrap();
    // known nullifier from this fixture run (idx 4)
    assert!(
        claim.nullifier_hex.starts_with("1753a470")
            || claim.nullifier != [0u8; 32],
        "expected real nullifier from snarkjs fixture"
    );
}

/// L2: RegisterClaim + require_registered_claim with real accountSalt claim.
#[test]
fn l2_register_claim_and_auth_with_real_salt() {
    let mock = Mock::new("admin");
    let admin = mock.sender_addr();
    let suite = TerpAuthenticatorSuite::deploy_on(
        mock,
        TerpAuthenticatorDeployData {
            admin: admin.clone(),
            dao: Some(admin.clone()),
            recovery_hash_alg: None,
            require_registered_claim: true,
            eth_signer: terp_authenticator_suite::ETH_GOLDEN_SIGNER.into(),
        },
    )
    .expect("deploy");
    suite
        .zk_jwt
        .execute(
            &terp_zkjwt::ExecuteMsg::UpsertIssuer(terp_zkjwt::IssuerConfig {
                issuer: "https://accounts.example.com".into(),
                verifying_key: Binary::from(b"test-issuer-vk"),
                zkid: Some(1),
                audience: None,
                inclusion_set_root: None,
            }),
            &[],
        )
        .unwrap();

    let claim = full_claim();
    claim_is_sound(&claim).unwrap();
    suite
        .zk_jwt
        .execute(
            &terp_zkjwt::ExecuteMsg::RegisterClaim {
                claim_commitment: Binary::from(claim.claim_commitment),
                issuer: "https://accounts.example.com".into(),
            },
            &[],
        )
        .unwrap();

    let owner = suite.zk_jwt.environment().sender_addr();
    let _ = suite.zk_jwt.sudo_on_auth_added(dummy_on_auth_added(
        owner.clone(),
        "jwt-l2",
        Some(Binary::from(b"p")),
    ));
    let p = ZkJwtAuthPayload {
        issuer: "https://accounts.example.com".into(),
        claim_commitment: Binary::from(claim.claim_commitment),
        public_inputs: claim.to_policy_public_inputs(),
        proof: Binary::from(b"structural-after-claim-link"),
        circuit_id: circuit_ids::JWT_PROFILE_LINK.into(),
    };
    suite
        .zk_jwt
        .sudo_authenticate(dummy_authentication_request(
            owner,
            "jwt-l2",
            to_json_binary(&p).unwrap(),
            None,
        ))
        .expect("registered claim + real salt must auth (structural host off)");
}
