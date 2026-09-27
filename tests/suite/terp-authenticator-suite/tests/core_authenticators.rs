//! Trait-workflow coverage for core priority authenticators.

use cosmwasm_std::{to_json_binary, Binary};
use cw_orch::prelude::*;
use terp_authenticator_suite::fixtures::*;
use terp_authenticator_suite::traits::AuthenticatorSudoExt;
use terp_authenticator_suite::{TerpAuthenticatorDeployData, TerpAuthenticatorSuite};
use terp_passkey::PasskeyAuthPayload;
use terp_irl::IrlWitnessPayload;

fn deploy() -> TerpAuthenticatorSuite<Mock> {
    let mock = Mock::new("admin");
    let admin = mock.sender_addr();
    // recovery guardian2 must be a valid bech32-like addr for addr_validate
    // Mock accepts most strings via Addr::unchecked at validate depending on api
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
    .expect("deploy")
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
    assert_action(&add().unwrap_or_else(|e| panic!("{label} add: {e}")), actions[0]);
    assert_action(&rem().unwrap_or_else(|e| panic!("{label} rem: {e}")), actions[1]);
    assert_action(&auth().unwrap_or_else(|e| panic!("{label} auth: {e}")), actions[2]);
    assert_action(&track().unwrap_or_else(|e| panic!("{label} track: {e}")), actions[3]);
    assert_action(&conf().unwrap_or_else(|e| panic!("{label} conf: {e}")), actions[4]);
}

#[test]
fn passkey_trait_workflow() {
    let suite = deploy();
    let acct = mock_account();
    let payload = PasskeyAuthPayload {
        origin: Some("https://app.terp.network".into()),
        credential_id: Binary::from(b"cred-id-1"),
        assertion: Binary::from(b"sig"),
        authenticator_data: Binary::from(b"ad"),
        client_data_json: Binary::from(b"{}"),
    };
    let sig = to_json_binary(&payload).unwrap();

    assert!(suite
        .passkey
        .sudo_on_auth_added(dummy_on_auth_added(acct.clone(), "1", None))
        .is_err());

    five_lifecycle(
        "passkey",
        || {
            suite.passkey.sudo_on_auth_added(dummy_on_auth_added(
                acct.clone(),
                "1",
                Some(Binary::from(b"p")),
            ))
        },
        || suite.passkey.sudo_on_auth_removed(dummy_on_auth_removed(acct.clone(), "1", None)),
        || {
            suite.passkey.sudo_authenticate(dummy_authentication_request(
                acct.clone(),
                "1",
                sig.clone(),
                None,
            ))
        },
        || suite.passkey.sudo_track(dummy_track_request(acct.clone(), "1", None)),
        || suite.passkey.sudo_confirm_execution(dummy_confirm_execution_request(acct.clone(), "1", None)),
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
fn recovery_trait_workflow() {
    use terp_recovery::{
        break_glass_challenge, rehash, GuardianApproval, RecoveryAuthPayload, RecoveryHashAlg,
        DEFAULT_DOMAIN,
    };

    let suite = deploy();
    let acct = mock_account();
    let admin = suite.recovery.environment().sender_addr();

    // Build break-glass attestation for address-only guardian (suite default)
    let mut auth_req = dummy_authentication_request(acct.clone(), "1", Binary::default(), None);
    // align chain/account with what authenticator will hash
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
    let mut att_pre = admin.as_bytes().to_vec();
    att_pre.push(0);
    att_pre.extend_from_slice(&challenge);
    let attestation = rehash(RecoveryHashAlg::Sha256, &att_pre, 1).unwrap();
    let payload = RecoveryAuthPayload {
        hash_alg: RecoveryHashAlg::Sha256,
        rehash_rounds: 1,
        approvals: vec![GuardianApproval {
            guardian: admin.to_string(),
            signature: Binary::from(attestation),
        }],
    };
    let sig = to_json_binary(&payload).unwrap();
    auth_req.signature = sig;

    five_lifecycle(
        "recovery",
        || {
            suite.recovery.sudo_on_auth_added(dummy_on_auth_added(
                acct.clone(),
                "1",
                Some(Binary::from(b"p")),
            ))
        },
        || suite.recovery.sudo_on_auth_removed(dummy_on_auth_removed(acct.clone(), "1", None)),
        || suite.recovery.sudo_authenticate(auth_req.clone()),
        || suite.recovery.sudo_track(dummy_track_request(acct.clone(), "1", None)),
        || {
            suite
                .recovery
                .sudo_confirm_execution(dummy_confirm_execution_request(acct.clone(), "1", None))
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
fn recovery_poseidon_break_glass() {
    use terp_recovery::{
        break_glass_challenge, rehash, ExecuteMsg, GuardianApproval, GuardianConfig,
        RecoveryAuthPayload, RecoveryConfig, RecoveryHashAlg, DEFAULT_DOMAIN,
    };

    let suite = deploy();
    let acct = mock_account();
    let admin = suite.recovery.environment().sender_addr();

    // Optional Poseidon path: switch recovery hash_alg without redeploying the suite.
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
                hash_alg: RecoveryHashAlg::PoseidonPallas,
                rehash_rounds: 1,
                domain: DEFAULT_DOMAIN.into(),
                allow_address_only_approvals: true,
            }),
            &[],
        )
        .expect("update recovery to PoseidonPallas");

    let mut auth_req = dummy_authentication_request(acct.clone(), "1", Binary::default(), None);
    auth_req.tx_data.chain_id = "terp-test-1".into();
    auth_req.account = acct.clone();
    let challenge = break_glass_challenge(
        RecoveryHashAlg::PoseidonPallas,
        1,
        DEFAULT_DOMAIN,
        &auth_req.tx_data.chain_id,
        acct.as_str(),
        "1",
        auth_req.sign_mode_tx_data.sign_mode_direct.as_slice(),
    )
    .unwrap();
    assert_eq!(challenge.len(), 32);

    let mut att_pre = admin.as_bytes().to_vec();
    att_pre.push(0);
    att_pre.extend_from_slice(&challenge);
    let attestation = rehash(RecoveryHashAlg::PoseidonPallas, &att_pre, 1).unwrap();
    let payload = RecoveryAuthPayload {
        hash_alg: RecoveryHashAlg::PoseidonPallas,
        rehash_rounds: 1,
        approvals: vec![GuardianApproval {
            guardian: admin.to_string(),
            signature: Binary::from(attestation),
        }],
    };
    auth_req.signature = to_json_binary(&payload).unwrap();

    let res = suite
        .recovery
        .sudo_authenticate(auth_req)
        .expect("poseidon break-glass authenticate");
    assert_action(&res, "recovery_authenticate");
}

#[test]
fn ed25519_lifecycle_without_valid_sig() {
    let suite = deploy();
    let acct = mock_account();
    // Note: suite deploys with pubkey [1u8;32] — not a golden key.
    // add/remove/track/confirm do not need valid crypto
    assert_action(
        &suite
            .ed25519
            .sudo_on_auth_added(dummy_on_auth_added(
                acct.clone(),
                "1",
                Some(Binary::from([1u8; 32])),
            ))
            .unwrap(),
        "ed25519_on_auth_added",
    );
    assert_action(
        &suite
            .ed25519
            .sudo_on_auth_removed(dummy_on_auth_removed(acct.clone(), "1", None))
            .unwrap(),
        "ed25519_on_auth_removed",
    );
    // re-add after remove for remaining lifecycle
    let _ = suite.ed25519.sudo_on_auth_added(dummy_on_auth_added(
        acct.clone(),
        "1",
        Some(Binary::from([1u8; 32])),
    ));
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
            .sudo_confirm_execution(dummy_confirm_execution_request(acct.clone(), "1", None))
            .unwrap(),
        "ed25519_confirm",
    );
    // bad signature length → error (host never called)
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

/// Golden single-sig using cosmwasm-std RFC fixtures (host `ed25519_verify`).
#[test]
fn ed25519_host_verify_golden_single() {
    use cosmwasm_std::testing::mock_dependencies;
    use terp_ed25519::{verify_batch, verify_single};

    const MSG: &str = "72";
    const SIG: &str = "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00";
    const PK: &str = "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c";

    let msg = hex::decode(MSG).unwrap();
    let sig = hex::decode(SIG).unwrap();
    let pk = hex::decode(PK).unwrap();

    let deps = mock_dependencies();
    verify_single(deps.as_ref().api, &msg, &sig, &pk).unwrap();
    verify_batch(
        deps.as_ref().api,
        &[&msg],
        &[&sig, &sig],
        &[&pk, &pk],
    )
    .unwrap();
}

#[test]
fn eth_lifecycle_structural() {
    let suite = deploy();
    let acct = mock_account();
    // Hardhat/Anvil #0 personal_sign over fixture sign_mode_direct (see exhaustive_matrix goldens)
    let golden_sig = Binary::from(
        hex::decode(
            "2f009b149c8ced8d23c2070873bf7b4a990673878a381dfb3a65697e811bac1f\
             05260e2c5d9f37748ed2335d3ec8c223f943857d2e46e7e847774b33273fa5b41b",
        )
        .unwrap(),
    );
    assert_action(
        &suite
            .eth
            .sudo_on_auth_added(dummy_on_auth_added(
                acct.clone(),
                "1",
                Some(Binary::from(b"p")),
            ))
            .unwrap(),
        "eth_on_auth_added",
    );
    assert_action(
        &suite
            .eth
            .sudo_on_auth_removed(dummy_on_auth_removed(acct.clone(), "1", None))
            .unwrap(),
        "eth_on_auth_removed",
    );
    assert!(suite
        .eth
        .sudo_authenticate(dummy_authentication_request(
            acct.clone(),
            "1",
            Binary::from(b"not65bytes"),
            None,
        ))
        .is_err());
    assert_action(
        &suite
            .eth
            .sudo_authenticate(dummy_authentication_request(
                acct.clone(),
                "1",
                golden_sig,
                None,
            ))
            .unwrap(),
        "eth_authenticate",
    );
    assert_action(
        &suite
            .eth
            .sudo_track(dummy_track_request(acct.clone(), "1", None))
            .unwrap(),
        "eth_track",
    );
    assert_action(
        &suite
            .eth
            .sudo_confirm_execution(dummy_confirm_execution_request(acct, "1", None))
            .unwrap(),
        "eth_confirm",
    );
}

#[test]
fn irl_trait_workflow() {
    let suite = deploy();
    let acct = mock_account();
    let payload = IrlWitnessPayload {
        epoch_root: Binary::from(b"epoch-root-v1"),
        witness: Binary::from(b"w"),
        nullifier: Binary::from(b"n"),
    };
    let sig = to_json_binary(&payload).unwrap();
    five_lifecycle(
        "irl",
        || {
            suite.irl.sudo_on_auth_added(dummy_on_auth_added(
                acct.clone(),
                "1",
                Some(Binary::from(b"p")),
            ))
        },
        || suite.irl.sudo_on_auth_removed(dummy_on_auth_removed(acct.clone(), "1", None)),
        || {
            suite
                .irl
                .sudo_authenticate(dummy_authentication_request(acct.clone(), "1", sig, None))
        },
        || suite.irl.sudo_track(dummy_track_request(acct.clone(), "1", None)),
        || suite.irl.sudo_confirm_execution(dummy_confirm_execution_request(acct.clone(), "1", None)),
        [
            "irl_on_auth_added",
            "irl_on_auth_removed",
            "irl_authenticate",
            "irl_track",
            "irl_confirm",
        ],
    );
}

#[test]
fn full_suite_coexistence() {
    let suite = deploy();
    let addrs = [
        suite.passkey.addr_str().unwrap(),
        suite.recovery.addr_str().unwrap(),
        suite.ed25519.addr_str().unwrap(),
        suite.eth.addr_str().unwrap(),
        suite.irl.addr_str().unwrap(),
        suite.zk_jwt.addr_str().unwrap(),
        suite.zk_poseidon.addr_str().unwrap(),
        suite.vsck.addr_str().unwrap(),
    ];
    for a in &addrs {
        assert!(!a.is_empty());
    }
    for i in 0..addrs.len() {
        for j in (i + 1)..addrs.len() {
            assert_ne!(addrs[i], addrs[j], "duplicate address {i}/{j}");
        }
    }
}
