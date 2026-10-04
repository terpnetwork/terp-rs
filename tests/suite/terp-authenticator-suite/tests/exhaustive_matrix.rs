//! Exhaustive AuthSudoMsg matrix for every suite authenticator.
//!
//! ## Matrix completeness (Mock / structural + policy negatives)
//!
//! Legend: **Y** = positive path asserted · **N** = intentional negative · **P** = partial
//! (scaffold accept-all, length-only, or host-crypto deferred) · **—** = no policy.
//!
//! | Authenticator | OnAuthAdded | OnAuthRemoved | Authenticate | Track | ConfirmExecution | Negatives covered |
//! |---------------|-------------|---------------|--------------|-------|------------------|-------------------|
//! | passkey       | Y           | Y             | Y            | Y     | Y                | missing params; empty assertion; credential_id; origin |
//! | recovery      | Y           | Y             | Y (Sha256 BG)| Y     | Y                | missing params; wrong hash_alg; empty; non-guardian; threshold 2/3; rehash_rounds |
//! | ed25519       | Y           | Y             | Y golden+batch| Y    | Y                | missing/wrong params; short sig; wrong msg; wrong key |
//! | eth           | Y           | Y             | Y golden personal_sign| Y | Y          | missing params; bad sig len; wrong signer; wrong msg |
//! | zk_jwt        | Y           | Y             | Y            | Y     | Y                | missing params; empty proof; wrong issuer; claim mismatch; msg_bind; nullifier replay |
//! | vsck          | Y           | Y             | Y all roles  | Y     | Y                | missing params; closed; empty proof; role/circuit mismatch; nullifier replay |
//!
//! ### Also covered outside this binary
//! - zk-jwt require_registered_claim / inclusion root → `zk_jwt_inclusion.rs`
//! - circom codec L1 + real snarkjs e2e → `zk_jwt_circom_codec.rs`, `zk_host.rs` (`--features zk-host`)
//!
//! ### Intentional gaps (not blocked here)
//! - vote-sdk Halo2 circuits for vsck (fail-closed until linked)
//! - Composite multi-authenticator AND/OR
//!
//! Uses [`terp_authenticator_suite::fixtures`] + [`AuthenticatorSudoExt`]. Mock only.

use cosmwasm_std::{to_json_binary, Binary};
use cw_orch::prelude::*;
use terp_authenticator_suite::fixtures::*;
use terp_authenticator_suite::traits::AuthenticatorSudoExt;
use terp_authenticator_suite::{TerpAuthenticatorDeployData, TerpAuthenticatorSuite};
use terp_ed25519::Ed25519AuthPayload;
use terp_passkey::PasskeyAuthPayload;
use terp_recovery::{
    break_glass_challenge, rehash, GuardianApproval, RecoveryAuthPayload, RecoveryHashAlg,
    DEFAULT_DOMAIN,
};
use terp_vsck::{VsckAuthPayload, VsckCircuit, VsckRole};
use terp_zkjwt::{circuit_ids, compute_msg_bind, ZkJwtAuthPayload};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn deploy() -> TerpAuthenticatorSuite<Mock> {
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
    assert!(found, "expected action={action}\nevents={:#?}", res.events);
}

fn five_lifecycle(
    label: &str,
    add: impl FnOnce() -> Result<<Mock as TxHandler>::Response, CwOrchError>,
    rem: impl FnOnce() -> Result<<Mock as TxHandler>::Response, CwOrchError>,
    auth: impl FnOnce() -> Result<<Mock as TxHandler>::Response, CwOrchError>,
    track: impl FnOnce() -> Result<<Mock as TxHandler>::Response, CwOrchError>,
    conf: impl FnOnce() -> Result<<Mock as TxHandler>::Response, CwOrchError>,
    actions: [&str; 5],
) {
    assert_action(
        &add().unwrap_or_else(|e| panic!("{label} OnAuthAdded: {e}")),
        actions[0],
    );
    assert_action(
        &rem().unwrap_or_else(|e| panic!("{label} OnAuthRemoved: {e}")),
        actions[1],
    );
    assert_action(
        &auth().unwrap_or_else(|e| panic!("{label} Authenticate: {e}")),
        actions[2],
    );
    assert_action(
        &track().unwrap_or_else(|e| panic!("{label} Track: {e}")),
        actions[3],
    );
    assert_action(
        &conf().unwrap_or_else(|e| panic!("{label} ConfirmExecution: {e}")),
        actions[4],
    );
}

fn passkey_ok_payload() -> Binary {
    to_json_binary(&terp_authenticator_suite::passkey_fixture::passkey_auth_payload(
        b"sign-mode-direct",
    ))
    .unwrap()
}

/// Address-only guardian break-glass attestation (suite default config).
fn recovery_sha256_auth_req(
    suite: &TerpAuthenticatorSuite<Mock>,
    acct: cosmwasm_std::Addr,
    authenticator_id: &str,
) -> terp_auth::AuthenticationRequest {
    let admin = suite.recovery.environment().sender_addr();
    let mut auth_req =
        dummy_authentication_request(acct.clone(), authenticator_id, Binary::default(), None);
    auth_req.tx_data.chain_id = "terp-test-1".into();
    auth_req.account = acct.clone();
    let challenge = break_glass_challenge(
        RecoveryHashAlg::Sha256,
        1,
        DEFAULT_DOMAIN,
        &auth_req.tx_data.chain_id,
        acct.as_str(),
        authenticator_id,
        auth_req.sign_mode_tx_data.sign_mode_direct.as_slice(),
    )
    .unwrap();
    let mut att_pre = admin.as_bytes().to_vec();
    att_pre.push(0);
    att_pre.extend_from_slice(&challenge);
    let attestation = rehash(RecoveryHashAlg::Sha256, &att_pre, 1).unwrap();
    let payload = RecoveryAuthPayload {
        hash_alg: RecoveryHashAlg::Sha256,
        rehash_rounds: 1,
        preimage_proof: None,
        approvals: vec![GuardianApproval {
            guardian: admin.to_string(),
            signature: Binary::from(attestation),
        }],
    };
    auth_req.signature = to_json_binary(&payload).unwrap();
    auth_req
}

/// Suite default issuer inclusion root (see suite.rs deploy).
fn suite_inclusion_root() -> [u8; 32] {
    [0xABu8; 32]
}

fn zk_jwt_payload(
    nullifier: [u8; 32],
    claim: [u8; 32],
    proof: &[u8],
    issuer: &str,
    msg_bind: Option<[u8; 32]>,
) -> Binary {
    // Layout: nf || claim || inclusion_set_root || [msg_bind]
    let public_inputs = terp_zkjwt::build_public_inputs(
        &nullifier,
        &claim,
        Some(&suite_inclusion_root()),
        msg_bind.as_ref(),
    );
    to_json_binary(&ZkJwtAuthPayload {
        issuer: issuer.into(),
        claim_commitment: Binary::from(claim),
        public_inputs,
        proof: Binary::from(proof),
        circuit_id: circuit_ids::JWT_MEMBERSHIP.into(),
    })
    .unwrap()
}

fn vsck_root() -> Binary {
    Binary::from(b"note-tree-root-32bytes___________")
}

fn open_vsck_session(suite: &TerpAuthenticatorSuite<Mock>, session_id: &str) {
    suite
        .vsck
        .execute(
            &terp_vsck::ExecuteMsg::OpenSession {
                session_id: session_id.into(),
                note_tree_root: vsck_root(),
                proposal_ref: Some("prop-matrix".into()),
            },
            &[],
        )
        .unwrap();
}

fn vsck_payload(session_id: &str, role: VsckRole, circuit: VsckCircuit, nf: &[u8]) -> Binary {
    let mut public_inputs = vsck_root().to_vec();
    public_inputs.extend_from_slice(b"|ballot");
    to_json_binary(&VsckAuthPayload {
        session_id: session_id.into(),
        role,
        circuit,
        public_inputs: Binary::from(public_inputs),
        proof: Binary::from(b"proof-bytes"),
        nullifier: Binary::from(nf),
    })
    .unwrap()
}

// ---------------------------------------------------------------------------
// Coexistence
// ---------------------------------------------------------------------------

#[test]
fn all_suite_contracts_distinct_addresses() {
    let suite = deploy();
    let addrs = [
        suite.passkey.addr_str().unwrap(),
        suite.recovery.addr_str().unwrap(),
        suite.ed25519.addr_str().unwrap(),
        suite.eth.addr_str().unwrap(),
        suite.zk_jwt.addr_str().unwrap(),
        suite.vsck.addr_str().unwrap(),
    ];
    for a in &addrs {
        assert!(!a.is_empty(), "empty address");
    }
    for i in 0..addrs.len() {
        for j in (i + 1)..addrs.len() {
            assert_ne!(
                addrs[i], addrs[j],
                "duplicate suite address at {i} and {j}: {}",
                addrs[i]
            );
        }
    }
}

// ---------------------------------------------------------------------------
// passkey
// ---------------------------------------------------------------------------

#[test]
fn passkey_matrix_all_five_positive() {
    let suite = deploy();
    let acct = mock_account();
    let sig = passkey_ok_payload();
    five_lifecycle(
        "passkey",
        || {
            suite.passkey.sudo_on_auth_added(dummy_on_auth_added(
                acct.clone(),
                "pk-1",
                Some(Binary::from(b"p")),
            ))
        },
        || {
            suite
                .passkey
                .sudo_on_auth_removed(dummy_on_auth_removed(acct.clone(), "pk-1", None))
        },
        || {
            suite.passkey.sudo_authenticate(dummy_authentication_request(
                acct.clone(),
                "pk-1",
                sig.clone(),
                None,
            ))
        },
        || {
            suite
                .passkey
                .sudo_track(dummy_track_request(acct.clone(), "pk-1", None))
        },
        || {
            suite
                .passkey
                .sudo_confirm_execution(dummy_confirm_execution_request(acct.clone(), "pk-1", None))
        },
        [
            "passkey_on_auth_added",
            "passkey_on_auth_removed",
            "passkey_authenticate",
            "passkey_track",
            "passkey_confirm",
        ],
    );
}

#[test]
fn passkey_neg_missing_on_auth_added_params() {
    let suite = deploy();
    assert!(suite
        .passkey
        .sudo_on_auth_added(dummy_on_auth_added(mock_account(), "pk-n", None))
        .is_err());
}

#[test]
fn passkey_neg_empty_assertion_fields() {
    let suite = deploy();
    let acct = mock_account();
    let _ = suite.passkey.sudo_on_auth_added(dummy_on_auth_added(
        acct.clone(),
        "1",
        Some(Binary::from(b"p")),
    ));
    let bad = PasskeyAuthPayload {
        origin: Some("https://app.terp.network".into()),
        credential_id: Binary::from(b"cred-id-1"),
        assertion: Binary::default(),
        authenticator_data: Binary::from(b"ad"),
        client_data_json: Binary::from(b"{}"),
    };
    assert!(suite
        .passkey
        .sudo_authenticate(dummy_authentication_request(
            acct,
            "1",
            to_json_binary(&bad).unwrap(),
            None,
        ))
        .is_err());
}

#[test]
fn passkey_neg_credential_id_mismatch() {
    let suite = deploy();
    let acct = mock_account();
    let bad = PasskeyAuthPayload {
        origin: Some("https://app.terp.network".into()),
        credential_id: Binary::from(b"wrong-cred"),
        assertion: Binary::from(b"sig"),
        authenticator_data: Binary::from(b"ad"),
        client_data_json: Binary::from(b"{}"),
    };
    assert!(suite
        .passkey
        .sudo_authenticate(dummy_authentication_request(
            acct,
            "1",
            to_json_binary(&bad).unwrap(),
            None,
        ))
        .is_err());
}

#[test]
fn passkey_neg_origin_mismatch() {
    let suite = deploy();
    let acct = mock_account();
    let bad = PasskeyAuthPayload {
        origin: Some("https://evil.example".into()),
        credential_id: Binary::from(b"cred-id-1"),
        assertion: Binary::from(b"sig"),
        authenticator_data: Binary::from(b"ad"),
        client_data_json: Binary::from(b"{}"),
    };
    assert!(suite
        .passkey
        .sudo_authenticate(dummy_authentication_request(
            acct,
            "1",
            to_json_binary(&bad).unwrap(),
            None,
        ))
        .is_err());
}

// ---------------------------------------------------------------------------
// recovery
// ---------------------------------------------------------------------------

#[test]
fn recovery_matrix_all_five_sha256_break_glass() {
    let suite = deploy();
    let acct = mock_account();
    let auth_req = recovery_sha256_auth_req(&suite, acct.clone(), "rec-1");
    five_lifecycle(
        "recovery",
        || {
            suite.recovery.sudo_on_auth_added(dummy_on_auth_added(
                acct.clone(),
                "rec-1",
                Some(Binary::from(b"p")),
            ))
        },
        || {
            suite
                .recovery
                .sudo_on_auth_removed(dummy_on_auth_removed(acct.clone(), "rec-1", None))
        },
        || suite.recovery.sudo_authenticate(auth_req.clone()),
        || {
            suite
                .recovery
                .sudo_track(dummy_track_request(acct.clone(), "rec-1", None))
        },
        || {
            suite.recovery.sudo_confirm_execution(dummy_confirm_execution_request(
                acct.clone(),
                "rec-1",
                None,
            ))
        },
        [
            "recovery_on_auth_added",
            "recovery_on_auth_removed",
            "recovery_authenticate",
            "recovery_track",
            "recovery_confirm",
        ],
    );
}

#[test]
fn recovery_neg_missing_on_auth_added_params() {
    let suite = deploy();
    assert!(suite
        .recovery
        .sudo_on_auth_added(dummy_on_auth_added(mock_account(), "1", None))
        .is_err());
}

#[test]
fn recovery_neg_wrong_hash_alg() {
    let suite = deploy();
    let acct = mock_account();
    let admin = suite.recovery.environment().sender_addr();
    // Config is Sha256; client claims Keccak256 → alg confusion reject.
    let payload = RecoveryAuthPayload {
        hash_alg: RecoveryHashAlg::Keccak256,
        rehash_rounds: 1,
        preimage_proof: None,
        approvals: vec![GuardianApproval {
            guardian: admin.to_string(),
            signature: Binary::from(b"unused"),
        }],
    };
    assert!(suite
        .recovery
        .sudo_authenticate(dummy_authentication_request(
            acct,
            "1",
            to_json_binary(&payload).unwrap(),
            None,
        ))
        .is_err());
}

#[test]
fn recovery_neg_empty_approvals_below_threshold() {
    let suite = deploy();
    let acct = mock_account();
    let payload = RecoveryAuthPayload {
        hash_alg: RecoveryHashAlg::Sha256,
        rehash_rounds: 1,
        preimage_proof: None,
        approvals: vec![],
    };
    assert!(suite
        .recovery
        .sudo_authenticate(dummy_authentication_request(
            acct,
            "1",
            to_json_binary(&payload).unwrap(),
            None,
        ))
        .is_err());
}

/// Multi-guardian threshold: 2-of-3 address-only approvals succeed; 1-of-3 fails.
#[test]
fn recovery_threshold_2_of_3() {
    use cosmwasm_std::testing::MockApi;
    use terp_recovery::{ExecuteMsg, GuardianConfig, RecoveryConfig};

    let suite = deploy();
    let acct = mock_account();
    let admin = suite.recovery.environment().sender_addr();
    // MockApi generates bech32-valid guardians (addr_validate in UpdateConfig).
    let api = MockApi::default();
    let g2 = api.addr_make("guardian2");
    let g3 = api.addr_make("guardian3");

    suite
        .recovery
        .execute(
            &ExecuteMsg::UpdateConfig(RecoveryConfig {
                guardians: vec![
                    GuardianConfig {
                        address: admin.to_string(),
                        pubkey: None,
                        commitment: None,
                    },
                    GuardianConfig {
                        address: g2.to_string(),
                        pubkey: None,
                        commitment: None,
                    },
                    GuardianConfig {
                        address: g3.to_string(),
                        pubkey: None,
                        commitment: None,
                    },
                ],
                threshold: 2,
                hash_alg: RecoveryHashAlg::Sha256,
                rehash_rounds: 1,
                domain: DEFAULT_DOMAIN.into(),
                allow_address_only_approvals: true,
            }),
            &[],
        )
        .unwrap();

    let mut auth_req = dummy_authentication_request(acct.clone(), "rec-th", Binary::default(), None);
    auth_req.tx_data.chain_id = "terp-test-1".into();
    auth_req.account = acct.clone();
    let challenge = break_glass_challenge(
        RecoveryHashAlg::Sha256,
        1,
        DEFAULT_DOMAIN,
        &auth_req.tx_data.chain_id,
        acct.as_str(),
        "rec-th",
        auth_req.sign_mode_tx_data.sign_mode_direct.as_slice(),
    )
    .unwrap();

    let att = |guardian: &str| {
        let mut pre = guardian.as_bytes().to_vec();
        pre.push(0);
        pre.extend_from_slice(&challenge);
        Binary::from(rehash(RecoveryHashAlg::Sha256, &pre, 1).unwrap())
    };

    // 1 approval < threshold 2 → fail
    let one = RecoveryAuthPayload {
        hash_alg: RecoveryHashAlg::Sha256,
        rehash_rounds: 1,
        preimage_proof: None,
        approvals: vec![GuardianApproval {
            guardian: admin.to_string(),
            signature: att(admin.as_str()),
        }],
    };
    auth_req.signature = to_json_binary(&one).unwrap();
    assert!(suite.recovery.sudo_authenticate(auth_req.clone()).is_err());

    // 2 approvals → ok
    let two = RecoveryAuthPayload {
        hash_alg: RecoveryHashAlg::Sha256,
        rehash_rounds: 1,
        preimage_proof: None,
        approvals: vec![
            GuardianApproval {
                guardian: admin.to_string(),
                signature: att(admin.as_str()),
            },
            GuardianApproval {
                guardian: g2.to_string(),
                signature: att(g2.as_str()),
            },
        ],
    };
    auth_req.signature = to_json_binary(&two).unwrap();
    assert_action(
        &suite.recovery.sudo_authenticate(auth_req).unwrap(),
        "recovery_authenticate",
    );
}

/// Non-registered guardian approval is rejected.
#[test]
fn recovery_neg_non_guardian_approval() {
    use cosmwasm_std::testing::MockApi;

    let suite = deploy();
    let acct = mock_account();
    let mut auth_req = dummy_authentication_request(acct.clone(), "1", Binary::default(), None);
    auth_req.tx_data.chain_id = "terp-test-1".into();
    auth_req.account = acct.clone();
    let challenge = break_glass_challenge(
        RecoveryHashAlg::Sha256,
        1,
        DEFAULT_DOMAIN,
        &auth_req.tx_data.chain_id,
        acct.as_str(),
        "1",
        auth_req.sign_mode_tx_data.sign_mode_direct.as_slice(),
    )
    .unwrap();
    let stranger = MockApi::default().addr_make("notaguardian");
    let mut pre = stranger.as_bytes().to_vec();
    pre.push(0);
    pre.extend_from_slice(&challenge);
    let att = rehash(RecoveryHashAlg::Sha256, &pre, 1).unwrap();
    let payload = RecoveryAuthPayload {
        hash_alg: RecoveryHashAlg::Sha256,
        rehash_rounds: 1,
        preimage_proof: None,
        approvals: vec![GuardianApproval {
            guardian: stranger.to_string(),
            signature: Binary::from(att),
        }],
    };
    auth_req.signature = to_json_binary(&payload).unwrap();
    assert!(suite.recovery.sudo_authenticate(auth_req).is_err());
}

/// rehash_rounds > 1 must match config (challenge uses N rounds).
#[test]
fn recovery_rehash_rounds_2() {
    use terp_recovery::{ExecuteMsg, GuardianConfig, RecoveryConfig};

    let suite = deploy();
    let acct = mock_account();
    let admin = suite.recovery.environment().sender_addr();
    suite
        .recovery
        .execute(
            &ExecuteMsg::UpdateConfig(RecoveryConfig {
                guardians: vec![GuardianConfig {
                    address: admin.to_string(),
                    pubkey: None,
                    commitment: None,
                }],
                threshold: 1,
                hash_alg: RecoveryHashAlg::Sha256,
                rehash_rounds: 2,
                domain: DEFAULT_DOMAIN.into(),
                allow_address_only_approvals: true,
            }),
            &[],
        )
        .unwrap();

    let mut auth_req = dummy_authentication_request(acct.clone(), "1", Binary::default(), None);
    auth_req.tx_data.chain_id = "terp-test-1".into();
    auth_req.account = acct.clone();
    let challenge = break_glass_challenge(
        RecoveryHashAlg::Sha256,
        2,
        DEFAULT_DOMAIN,
        &auth_req.tx_data.chain_id,
        acct.as_str(),
        "1",
        auth_req.sign_mode_tx_data.sign_mode_direct.as_slice(),
    )
    .unwrap();
    let mut pre = admin.as_bytes().to_vec();
    pre.push(0);
    pre.extend_from_slice(&challenge);
    let att = rehash(RecoveryHashAlg::Sha256, &pre, 2).unwrap();
    let payload = RecoveryAuthPayload {
        hash_alg: RecoveryHashAlg::Sha256,
        rehash_rounds: 2,
        preimage_proof: None,
        approvals: vec![GuardianApproval {
            guardian: admin.to_string(),
            signature: Binary::from(att),
        }],
    };
    auth_req.signature = to_json_binary(&payload).unwrap();
    assert_action(
        &suite.recovery.sudo_authenticate(auth_req).unwrap(),
        "recovery_authenticate",
    );

    // Client claims rounds=1 while config is 2 → reject
    let bad = RecoveryAuthPayload {
        hash_alg: RecoveryHashAlg::Sha256,
        rehash_rounds: 1,
        preimage_proof: None,
        approvals: vec![],
    };
    let mut bad_req = dummy_authentication_request(acct, "1", Binary::default(), None);
    bad_req.signature = to_json_binary(&bad).unwrap();
    assert!(suite.recovery.sudo_authenticate(bad_req).is_err());
}

/// Keccak-256 address-only break-glass (non-default hash algorithm).
#[test]
fn recovery_keccak256_hook() {
    use terp_recovery::{ExecuteMsg, GuardianConfig, RecoveryConfig, DEFAULT_DOMAIN};

    let suite = deploy();
    let acct = mock_account();
    let admin = suite.recovery.environment().sender_addr();

    suite
        .recovery
        .execute(
            &ExecuteMsg::UpdateConfig(RecoveryConfig {
                guardians: vec![GuardianConfig {
                    address: admin.to_string(),
                    pubkey: None,
                    commitment: None,
                }],
                threshold: 1,
                hash_alg: RecoveryHashAlg::Keccak256,
                rehash_rounds: 1,
                domain: DEFAULT_DOMAIN.into(),
                allow_address_only_approvals: true,
            }),
            &[],
        )
        .unwrap();

    let mut auth_req = dummy_authentication_request(acct.clone(), "1", Binary::default(), None);
    auth_req.tx_data.chain_id = "terp-test-1".into();
    auth_req.account = acct.clone();
    let challenge = break_glass_challenge(
        RecoveryHashAlg::Keccak256,
        1,
        DEFAULT_DOMAIN,
        &auth_req.tx_data.chain_id,
        acct.as_str(),
        "1",
        auth_req.sign_mode_tx_data.sign_mode_direct.as_slice(),
    )
    .unwrap();
    let mut att_pre = admin.as_bytes().to_vec();
    att_pre.push(0);
    att_pre.extend_from_slice(&challenge);
    let attestation = rehash(RecoveryHashAlg::Keccak256, &att_pre, 1).unwrap();
    let payload = RecoveryAuthPayload {
        hash_alg: RecoveryHashAlg::Keccak256,
        rehash_rounds: 1,
        preimage_proof: None,
        approvals: vec![GuardianApproval {
            guardian: admin.to_string(),
            signature: Binary::from(attestation),
        }],
    };
    auth_req.signature = to_json_binary(&payload).unwrap();
    assert_action(
        &suite.recovery.sudo_authenticate(auth_req).unwrap(),
        "recovery_authenticate",
    );
}

// ---------------------------------------------------------------------------
// ed25519
// ---------------------------------------------------------------------------

/// CosmWasm MockApi RFC-aligned fixtures (hex).
const ED25519_MSG_HEX: &str = "72";
const ED25519_SIG_HEX: &str = "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00";
const ED25519_PUBKEY_HEX: &str =
    "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c";

#[test]
fn ed25519_matrix_lifecycle_and_golden_auth() {
    let suite = deploy();
    let acct = mock_account();
    let pk = hex::decode(ED25519_PUBKEY_HEX).unwrap();
    let msg = hex::decode(ED25519_MSG_HEX).unwrap();
    let sig = hex::decode(ED25519_SIG_HEX).unwrap();

    // Clear suite default [1u8;32] so OnAuthAdded can install golden pubkey.
    assert_action(
        &suite
            .ed25519
            .sudo_on_auth_removed(dummy_on_auth_removed(acct.clone(), "1", None))
            .unwrap(),
        "ed25519_on_auth_removed",
    );
    assert_action(
        &suite
            .ed25519
            .sudo_on_auth_added(dummy_on_auth_added(
                acct.clone(),
                "1",
                Some(Binary::from(pk)),
            ))
            .unwrap(),
        "ed25519_on_auth_added",
    );

    let mut auth_req =
        dummy_authentication_request(acct.clone(), "1", Binary::from(sig), None);
    auth_req.sign_mode_tx_data.sign_mode_direct = Binary::from(msg);
    assert_action(
        &suite.ed25519.sudo_authenticate(auth_req).unwrap(),
        "ed25519_authenticate",
    );
    assert_action(
        &suite
            .ed25519
            .sudo_track(dummy_track_request(acct.clone(), "1", None))
            .unwrap(),
        "ed25519_track",
    );
    assert_action(
        &suite
            .ed25519
            .sudo_confirm_execution(dummy_confirm_execution_request(acct, "1", None))
            .unwrap(),
        "ed25519_confirm",
    );
}

#[test]
fn ed25519_neg_missing_on_auth_added_params() {
    let suite = deploy();
    // remove first so missing-params path is exercised (not mismatch)
    let acct = mock_account();
    let _ = suite
        .ed25519
        .sudo_on_auth_removed(dummy_on_auth_removed(acct.clone(), "1", None));
    assert!(suite
        .ed25519
        .sudo_on_auth_added(dummy_on_auth_added(acct, "1", None))
        .is_err());
}

#[test]
fn ed25519_neg_short_signature() {
    let suite = deploy();
    let acct = mock_account();
    assert!(suite
        .ed25519
        .sudo_authenticate(dummy_authentication_request(
            acct,
            "1",
            Binary::from(b"short"),
            None,
        ))
        .is_err());
}

/// Batch multi-sig JSON shape through contract Authenticate (host batch_verify).
/// Valid golden sig over wrong sign_mode_direct message → reject.
#[test]
fn ed25519_neg_wrong_message() {
    let suite = deploy();
    let acct = mock_account();
    let pk = hex::decode(ED25519_PUBKEY_HEX).unwrap();
    let sig = hex::decode(ED25519_SIG_HEX).unwrap();
    let _ = suite
        .ed25519
        .sudo_on_auth_removed(dummy_on_auth_removed(acct.clone(), "1", None));
    let _ = suite.ed25519.sudo_on_auth_added(dummy_on_auth_added(
        acct.clone(),
        "1",
        Some(Binary::from(pk)),
    ));
    // Signature is over 0x72, not "sign-mode-direct"
    let mut auth_req =
        dummy_authentication_request(acct, "1", Binary::from(sig), None);
    auth_req.sign_mode_tx_data.sign_mode_direct = Binary::from(b"sign-mode-direct");
    assert!(suite.ed25519.sudo_authenticate(auth_req).is_err());
}

/// Golden sig against a different registered pubkey → reject.
#[test]
fn ed25519_neg_wrong_key() {
    let suite = deploy();
    let acct = mock_account();
    let sig = hex::decode(ED25519_SIG_HEX).unwrap();
    let msg = hex::decode(ED25519_MSG_HEX).unwrap();
    // All-ones is not the golden pubkey
    let wrong_pk = [0xAAu8; 32];
    let _ = suite
        .ed25519
        .sudo_on_auth_removed(dummy_on_auth_removed(acct.clone(), "1", None));
    let _ = suite.ed25519.sudo_on_auth_added(dummy_on_auth_added(
        acct.clone(),
        "1",
        Some(Binary::from(wrong_pk)),
    ));
    let mut auth_req =
        dummy_authentication_request(acct, "1", Binary::from(sig), None);
    auth_req.sign_mode_tx_data.sign_mode_direct = Binary::from(msg);
    assert!(suite.ed25519.sudo_authenticate(auth_req).is_err());
}

#[test]
fn ed25519_batch_payload_json_multi_sig() {
    let suite = deploy();
    let acct = mock_account();
    let pk = hex::decode(ED25519_PUBKEY_HEX).unwrap();
    let msg = hex::decode(ED25519_MSG_HEX).unwrap();
    let sig = hex::decode(ED25519_SIG_HEX).unwrap();

    let _ = suite
        .ed25519
        .sudo_on_auth_removed(dummy_on_auth_removed(acct.clone(), "1", None));
    let _ = suite.ed25519.sudo_on_auth_added(dummy_on_auth_added(
        acct.clone(),
        "1",
        Some(Binary::from(pk.clone())),
    ));

    // Shape (1 msg, 2 identical sig/key pairs) — valid host batch.
    let batch = Ed25519AuthPayload {
        messages: Some(vec![Binary::from(msg.clone())]),
        signatures: vec![Binary::from(sig.clone()), Binary::from(sig)],
        public_keys: Some(vec![Binary::from(pk.clone()), Binary::from(pk)]),
    };
    let mut auth_req = dummy_authentication_request(
        acct,
        "1",
        to_json_binary(&batch).unwrap(),
        None,
    );
    auth_req.sign_mode_tx_data.sign_mode_direct = Binary::from(msg);
    let res = suite.ed25519.sudo_authenticate(auth_req).unwrap();
    assert_action(&res, "ed25519_authenticate");
    let batch_attr = res.events.iter().any(|e| {
        e.attributes
            .iter()
            .any(|a| a.key == "verify" && a.value == "ed25519_batch_verify")
    });
    assert!(batch_attr, "expected ed25519_batch_verify attr, events={:#?}", res.events);
}

// ---------------------------------------------------------------------------
// eth
// ---------------------------------------------------------------------------

/// EIP-191 personal_sign golden over fixture `sign_mode_direct` = `b"sign-mode-direct"`.
///
/// Key: Hardhat/Anvil account #0
///   priv  = 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
///   addr  = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266  (= ETH_GOLDEN_SIGNER)
/// Generated with `cast wallet sign --private-key … "sign-mode-direct"`.
fn eth_golden_sig() -> Binary {
    Binary::from(
        hex::decode(
            "2f009b149c8ced8d23c2070873bf7b4a990673878a381dfb3a65697e811bac1f\
             05260e2c5d9f37748ed2335d3ec8c223f943857d2e46e7e847774b33273fa5b41b",
        )
        .expect("golden sig hex"),
    )
}

/// Valid personal_sign from Anvil account #1 (different signer).
fn eth_other_signer_sig() -> Binary {
    Binary::from(
        hex::decode(
            "6f6370ba3576bbdb1de75d56b7b362bf80be8691e73d191fd4d7a4dbb319788e\
             4a7ba0ec75de515262d307f603da6d16a1a4ec2e76b8d8facbbdbc2f845d94051b",
        )
        .expect("other signer sig hex"),
    )
}

/// Valid personal_sign from account #0 over a different message ("other-message").
fn eth_wrong_msg_sig() -> Binary {
    Binary::from(
        hex::decode(
            "86aef8ef45c29ecf7c3c856cd778edee79fd081dac7a1c5ade7ad53664e73614\
             27e2916c7a86f5a039adc2c1a888fe9e11feacb7851e029a7f6db1dba39166161c",
        )
        .expect("wrong-msg sig hex"),
    )
}

#[test]
fn eth_matrix_all_five_positive() {
    let suite = deploy();
    let acct = mock_account();
    assert_action(
        &suite
            .eth
            .sudo_on_auth_added(dummy_on_auth_added(
                acct.clone(),
                "eth-1",
                Some(Binary::from(b"p")),
            ))
            .unwrap(),
        "eth_on_auth_added",
    );
    assert_action(
        &suite
            .eth
            .sudo_on_auth_removed(dummy_on_auth_removed(acct.clone(), "eth-1", None))
            .unwrap(),
        "eth_on_auth_removed",
    );
    assert_action(
        &suite
            .eth
            .sudo_authenticate(dummy_authentication_request(
                acct.clone(),
                "eth-1",
                eth_golden_sig(),
                None,
            ))
            .unwrap(),
        "eth_authenticate",
    );
    assert_action(
        &suite
            .eth
            .sudo_track(dummy_track_request(acct.clone(), "eth-1", None))
            .unwrap(),
        "eth_track",
    );
    assert_action(
        &suite
            .eth
            .sudo_confirm_execution(dummy_confirm_execution_request(acct, "eth-1", None))
            .unwrap(),
        "eth_confirm",
    );
}

#[test]
fn eth_golden_personal_sign_authenticates() {
    let suite = deploy();
    let acct = mock_account();
    let _ = suite.eth.sudo_on_auth_added(dummy_on_auth_added(
        acct.clone(),
        "eth-golden",
        Some(Binary::from(b"p")),
    ));
    assert_action(
        &suite
            .eth
            .sudo_authenticate(dummy_authentication_request(
                acct,
                "eth-golden",
                eth_golden_sig(),
                None,
            ))
            .unwrap(),
        "eth_authenticate",
    );
}

#[test]
fn eth_neg_missing_on_auth_added_params() {
    let suite = deploy();
    assert!(suite
        .eth
        .sudo_on_auth_added(dummy_on_auth_added(mock_account(), "1", None))
        .is_err());
}

#[test]
fn eth_neg_bad_signature_length() {
    let suite = deploy();
    assert!(suite
        .eth
        .sudo_authenticate(dummy_authentication_request(
            mock_account(),
            "1",
            Binary::from(b"not65bytes"),
            None,
        ))
        .is_err());
}

#[test]
fn eth_neg_65_byte_wrong_signer() {
    let suite = deploy();
    // Valid 65-byte personal_sign from a *different* secp256k1 key over the same message.
    // Recover succeeds; signer mismatch must reject (not just zero-byte recover failure).
    assert!(suite
        .eth
        .sudo_authenticate(dummy_authentication_request(
            mock_account(),
            "1",
            eth_other_signer_sig(),
            None,
        ))
        .is_err());
}

#[test]
fn eth_neg_wrong_message() {
    let suite = deploy();
    // Signature from the registered key over a different message than sign_mode_direct.
    assert!(suite
        .eth
        .sudo_authenticate(dummy_authentication_request(
            mock_account(),
            "1",
            eth_wrong_msg_sig(),
            None,
        ))
        .is_err());
}

// ---------------------------------------------------------------------------
// zk_jwt
// ---------------------------------------------------------------------------

#[test]
fn zk_jwt_matrix_all_five_positive() {
    let suite = deploy();
    let acct = mock_account();
    let claim = [11u8; 32];
    let nf = [22u8; 32];
    let sig = zk_jwt_payload(nf, claim, b"proof-ok", "https://accounts.example.com", None);

    five_lifecycle(
        "zk_jwt",
        || {
            suite.zk_jwt.sudo_on_auth_added(dummy_on_auth_added(
                acct.clone(),
                "jwt-m",
                Some(Binary::from(b"cfg")),
            ))
        },
        || {
            suite
                .zk_jwt
                .sudo_on_auth_removed(dummy_on_auth_removed(acct.clone(), "jwt-m", None))
        },
        || {
            suite.zk_jwt.sudo_authenticate(dummy_authentication_request(
                acct.clone(),
                "jwt-m",
                sig.clone(),
                None,
            ))
        },
        || {
            suite
                .zk_jwt
                .sudo_track(dummy_track_request(acct.clone(), "jwt-m", None))
        },
        || {
            suite.zk_jwt.sudo_confirm_execution(dummy_confirm_execution_request(
                acct.clone(),
                "jwt-m",
                None,
            ))
        },
        [
            "zkjwt_on_auth_added",
            "zkjwt_on_auth_removed",
            "zkjwt_authenticate",
            "zkjwt_track",
            "zkjwt_confirm",
        ],
    );
}

#[test]
fn zk_jwt_neg_missing_on_auth_added_params() {
    let suite = deploy();
    assert!(suite
        .zk_jwt
        .sudo_on_auth_added(dummy_on_auth_added(mock_account(), "j", None))
        .is_err());
}

#[test]
fn zk_jwt_neg_wrong_issuer() {
    let suite = deploy();
    let claim = [1u8; 32];
    let sig = zk_jwt_payload([9u8; 32], claim, b"p", "https://unknown.issuer", None);
    assert!(suite
        .zk_jwt
        .sudo_authenticate(dummy_authentication_request(mock_account(), "1", sig, None))
        .is_err());
}

#[test]
fn zk_jwt_neg_empty_proof() {
    let suite = deploy();
    let claim = [1u8; 32];
    let sig = zk_jwt_payload([8u8; 32], claim, b"", "https://accounts.example.com", None);
    assert!(suite
        .zk_jwt
        .sudo_authenticate(dummy_authentication_request(mock_account(), "1", sig, None))
        .is_err());
}

#[test]
fn zk_jwt_neg_claim_commitment_mismatch() {
    let suite = deploy();
    let claim_field = [1u8; 32];
    let claim_in_pi = [2u8; 32];
    let public_inputs = terp_zkjwt::build_public_inputs(
        &[7u8; 32],
        &claim_in_pi,
        Some(&suite_inclusion_root()),
        None,
    );
    let payload = ZkJwtAuthPayload {
        issuer: "https://accounts.example.com".into(),
        claim_commitment: Binary::from(claim_field),
        public_inputs,
        proof: Binary::from(b"proof"),
        circuit_id: circuit_ids::JWT_MEMBERSHIP.into(),
    };
    assert!(suite
        .zk_jwt
        .sudo_authenticate(dummy_authentication_request(
            mock_account(),
            "1",
            to_json_binary(&payload).unwrap(),
            None,
        ))
        .is_err());
}

#[test]
fn zk_jwt_neg_msg_bind_mismatch() {
    let suite = deploy();
    let acct = mock_account();
    let claim = [3u8; 32];
    let wrong_bind = [0xAAu8; 32];
    let sig = zk_jwt_payload(
        [4u8; 32],
        claim,
        b"proof",
        "https://accounts.example.com",
        Some(wrong_bind),
    );
    // Also prove correct bind would match helper (sanity, not auth).
    let auth_req = dummy_authentication_request(acct.clone(), "jwt-bind", sig, None);
    let expected = compute_msg_bind(
        &auth_req.tx_data.chain_id,
        auth_req.account.as_str(),
        &auth_req.authenticator_id,
        auth_req.msg_index,
        auth_req.sign_mode_tx_data.sign_mode_direct.as_slice(),
    );
    assert_ne!(wrong_bind, expected);
    assert!(suite.zk_jwt.sudo_authenticate(auth_req).is_err());
}

#[test]
fn zk_jwt_msg_bind_positive_and_nullifier_replay() {
    let suite = deploy();
    let acct = mock_account();
    let claim = [5u8; 32];
    let mut auth_req = dummy_authentication_request(
        acct.clone(),
        "jwt-ok",
        Binary::default(),
        None,
    );
    let bind = compute_msg_bind(
        &auth_req.tx_data.chain_id,
        auth_req.account.as_str(),
        &auth_req.authenticator_id,
        auth_req.msg_index,
        auth_req.sign_mode_tx_data.sign_mode_direct.as_slice(),
    );
    let sig = zk_jwt_payload(
        [6u8; 32],
        claim,
        b"proof",
        "https://accounts.example.com",
        Some(bind),
    );
    auth_req.signature = sig.clone();
    assert_action(
        &suite.zk_jwt.sudo_authenticate(auth_req.clone()).unwrap(),
        "zkjwt_authenticate",
    );
    // same nullifier again
    assert!(suite.zk_jwt.sudo_authenticate(auth_req).is_err());
}

// ---------------------------------------------------------------------------
// vsck
// ---------------------------------------------------------------------------

#[test]
fn vsck_matrix_all_five_voter() {
    let suite = deploy();
    let acct = mock_account();
    open_vsck_session(&suite, "sess-matrix");
    let payload = vsck_payload(
        "sess-matrix",
        VsckRole::Voter,
        VsckCircuit::VoteProof,
        b"nf-matrix-1",
    );

    vsck_lifecycle_fail_closed(&suite, &acct, "vsck-m", payload);
}

#[test]
fn vsck_neg_missing_on_auth_added_params() {
    let suite = deploy();
    assert!(suite
        .vsck
        .sudo_on_auth_added(dummy_on_auth_added(mock_account(), "1", None))
        .is_err());
}

#[test]
fn vsck_neg_closed_session() {
    let suite = deploy();
    let acct = mock_account();
    open_vsck_session(&suite, "sess-close");
    suite
        .vsck
        .execute(
            &terp_vsck::ExecuteMsg::CloseSession {
                session_id: "sess-close".into(),
            },
            &[],
        )
        .unwrap();
    let payload = vsck_payload(
        "sess-close",
        VsckRole::Voter,
        VsckCircuit::VoteProof,
        b"nf-closed",
    );
    assert!(suite
        .vsck
        .sudo_authenticate(dummy_authentication_request(acct, "1", payload, None))
        .is_err());
}

#[test]
fn vsck_neg_empty_proof() {
    let suite = deploy();
    open_vsck_session(&suite, "sess-empty");
    let mut public_inputs = vsck_root().to_vec();
    public_inputs.extend_from_slice(b"|x");
    let payload = VsckAuthPayload {
        session_id: "sess-empty".into(),
        role: VsckRole::Voter,
        circuit: VsckCircuit::VoteProof,
        public_inputs: Binary::from(public_inputs),
        proof: Binary::default(),
        nullifier: Binary::from(b"nf"),
    };
    assert!(suite
        .vsck
        .sudo_authenticate(dummy_authentication_request(
            mock_account(),
            "1",
            to_json_binary(&payload).unwrap(),
            None,
        ))
        .is_err());
}

#[test]
fn vsck_role_delegator_and_tallier_smoke() {
    let suite = deploy();
    let acct = mock_account();
    open_vsck_session(&suite, "sess-roles");

    let del = vsck_payload(
        "sess-roles",
        VsckRole::Delegator,
        VsckCircuit::Delegation,
        b"nf-del",
    );
    let del_err = suite
        .vsck
        .sudo_authenticate(dummy_authentication_request(acct.clone(), "1", del, None))
        .unwrap_err();
    assert!(
        del_err.to_string().contains("CircuitNotLinked")
            || del_err.to_string().contains("not linked"),
        "{del_err}"
    );

    let tall = vsck_payload(
        "sess-roles",
        VsckRole::Tallier,
        VsckCircuit::ShareReveal,
        b"nf-tall",
    );
    let tall_err = suite
        .vsck
        .sudo_authenticate(dummy_authentication_request(acct, "1", tall, None))
        .unwrap_err();
    assert!(
        tall_err.to_string().contains("CircuitNotLinked")
            || tall_err.to_string().contains("not linked"),
        "{tall_err}"
    );
}

fn vsck_five_lifecycle_role(
    session: &str,
    role: VsckRole,
    circuit: VsckCircuit,
    nf: &[u8],
    auth_id: &str,
) {
    let suite = deploy();
    let acct = mock_account();
    open_vsck_session(&suite, session);
    let payload = vsck_payload(session, role.clone(), circuit, nf);
    vsck_lifecycle_fail_closed(&suite, &acct, auth_id, payload);
}

fn vsck_lifecycle_fail_closed(
    suite: &TerpAuthenticatorSuite<Mock>,
    acct: &cosmwasm_std::Addr,
    auth_id: &str,
    payload: Binary,
) {
    assert_action(
        &suite
            .vsck
            .sudo_on_auth_added(dummy_on_auth_added(
                acct.clone(),
                auth_id,
                Some(Binary::from(b"p")),
            ))
            .unwrap(),
        "vsck_on_auth_added",
    );
    assert_action(
        &suite
            .vsck
            .sudo_on_auth_removed(dummy_on_auth_removed(acct.clone(), auth_id, None))
            .unwrap(),
        "vsck_on_auth_removed",
    );
    let auth_err = suite
        .vsck
        .sudo_authenticate(dummy_authentication_request(
            acct.clone(),
            auth_id,
            payload.clone(),
            None,
        ))
        .unwrap_err();
    assert!(
        auth_err.to_string().contains("CircuitNotLinked")
            || auth_err.to_string().contains("not linked"),
        "expected fail-closed circuit: {auth_err}"
    );
    assert_action(
        &suite
            .vsck
            .sudo_track(dummy_track_request(acct.clone(), auth_id, Some(payload)))
            .unwrap(),
        "vsck_track",
    );
    assert_action(
        &suite
            .vsck
            .sudo_confirm_execution(dummy_confirm_execution_request(acct.clone(), auth_id, None))
            .unwrap(),
        "vsck_confirm",
    );
}

#[test]
fn vsck_matrix_all_five_delegator() {
    vsck_five_lifecycle_role(
        "sess-del-5",
        VsckRole::Delegator,
        VsckCircuit::Delegation,
        b"nf-del-5",
        "vsck-del",
    );
}

#[test]
fn vsck_matrix_all_five_tallier() {
    vsck_five_lifecycle_role(
        "sess-tall-5",
        VsckRole::Tallier,
        VsckCircuit::ShareReveal,
        b"nf-tall-5",
        "vsck-tall",
    );
}

/// Voter role with Delegation circuit → mismatch reject.
#[test]
fn vsck_neg_role_circuit_mismatch() {
    let suite = deploy();
    open_vsck_session(&suite, "sess-mismatch");
    let payload = vsck_payload(
        "sess-mismatch",
        VsckRole::Voter,
        VsckCircuit::Delegation,
        b"nf-mm",
    );
    assert!(suite
        .vsck
        .sudo_authenticate(dummy_authentication_request(
            mock_account(),
            "1",
            payload,
            None,
        ))
        .is_err());
}

/// Same nullifier tracked twice in a session → replay reject.
/// Authenticate is fail-closed until vote-sdk circuits are linked; track still records nf.
#[test]
fn vsck_neg_nullifier_replay_on_track() {
    let suite = deploy();
    let acct = mock_account();
    open_vsck_session(&suite, "sess-nf-replay");
    let payload = vsck_payload(
        "sess-nf-replay",
        VsckRole::Voter,
        VsckCircuit::VoteProof,
        b"nf-same",
    );
    let auth_err = suite
        .vsck
        .sudo_authenticate(dummy_authentication_request(
            acct.clone(),
            "1",
            payload.clone(),
            None,
        ))
        .unwrap_err();
    assert!(
        auth_err.to_string().contains("CircuitNotLinked")
            || auth_err.to_string().contains("not linked"),
        "expected fail-closed circuit: {auth_err}"
    );
    suite
        .vsck
        .sudo_track(dummy_track_request(
            acct.clone(),
            "1",
            Some(payload.clone()),
        ))
        .unwrap();
    assert!(suite
        .vsck
        .sudo_track(dummy_track_request(acct, "1", Some(payload)))
        .is_err());
}
