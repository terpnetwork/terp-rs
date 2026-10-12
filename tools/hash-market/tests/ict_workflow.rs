//! Existing test script: msg contract + preset curation.
//! Chain hook: `ICT_ONBOARD=1 cargo test -p terp-onboard-authz --test ict_workflow -- --ignored`

use hash_market::onboard_authz::curate::{curate, load_infra, preset, CurateMode, PRESETS};
use hash_market::onboard_authz::{
    dao_authz_grants, registration_feegrant, AllowanceScope, FeegrantRequest, OnboardRoles,
    ALLOWED_MSG_TYPES,
};

#[test]
fn prepared_msgs_match_faucet_adjacent_contract() {
    let roles = OnboardRoles {
        runtime: "terp1dao".into(),
        hashmerchant: "terp1rt".into(),
        chain_id: "120u-1".into(),
    };
    let grants = dao_authz_grants(&roles);
    assert_eq!(grants["msgs"].as_array().unwrap().len(), 2);
    let fg = registration_feegrant(
        &roles,
        &FeegrantRequest {
            grantee: "terp1alice".into(),
            spend_limit: "25000".into(),
            expiration: None,
        },
    );
    assert_eq!(
        fg["allowance"]["allowed_messages"][0],
        ALLOWED_MSG_TYPES[0]
    );
    assert!(AllowanceScope::registration_only().is_sound());
}

#[test]
fn curate_dao_and_wallet_modes_use_state_presets() {
    let infra = load_infra(None);
    let roles = OnboardRoles {
        runtime: "terp1dao".into(),
        hashmerchant: "terp1rt".into(),
        chain_id: "120u-1".into(),
    };
    assert_eq!(
        preset("norick")
            .unwrap()
            .resolve("120u-1", &infra)
            .code_ids
            .get("headstash:cw_norick"),
        Some(&72)
    );
    for p in PRESETS {
        let dao = curate(
            CurateMode::DaoProposal,
            &roles,
            p,
            &infra,
            None,
            "terp1alice",
        );
        assert!(dao.dao_proposal.is_some(), "{}", p.id);
        let self_rt = curate(
            CurateMode::WalletAsRuntime,
            &roles,
            p,
            &infra,
            None,
            "terp1alice",
        );
        assert!(self_rt.dao_proposal.is_none(), "{}", p.id);
        assert!(!self_rt.cosmos_msgs["msgs"].as_array().unwrap().is_empty());
    }
}

#[test]
#[ignore = "needs local ict-rs chain + onboard-runtime"]
fn live_register_grant_dry_run() {
    let url = std::env::var("ONBOARD_URL").unwrap_or_else(|_| "http://127.0.0.1:5001".into());
    let status = curl_get(&format!("{url}/status"));
    assert!(
        status.contains("onboard") || status.contains("hash-market"),
        "{status}"
    );
    let body = curl_post(
        &format!("{url}/v1/register-grant"),
        r#"{"address":"terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqg6p2x0"}"#,
    );
    assert!(body.contains("AllowedMsgAllowance"), "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
    let allowed = v["msg"]["allowance"]["allowed_messages"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        allowed
            .iter()
            .all(|x| x.as_str() != Some("/cosmos.bank.v1beta1.MsgSend")),
        "{body}"
    );
    assert!(
        body.contains("/terp.smartaccount.v1beta1.MsgAddAuthenticator"),
        "register grant must allow the module MsgAddAuthenticator, not CosmWasm: {body}"
    );
}

/// Live: after a register grant, broadcast proto MsgAddAuthenticator
/// `{ sender, authenticator_type, data }` (SignatureVerification or CosmwasmAuthenticatorV1).
/// Not `{ execute: { add_authenticator } }`. Needs ICT_ONBOARD + funded runtime.
#[test]
#[ignore = "needs ict-rs chain + onboard-runtime + signer"]
fn live_broadcast_module_add_authenticator() {
    let url = std::env::var("ONBOARD_URL").unwrap_or_else(|_| "http://127.0.0.1:5001".into());
    let status = curl_get(&format!("{url}/status"));
    assert!(
        status.contains("/terp.smartaccount.v1beta1.MsgAddAuthenticator"),
        "{status}"
    );
}

fn curl_get(url: &str) -> String {
    let out = std::process::Command::new("curl")
        .args(["-sS", "--max-time", "4", url])
        .output()
        .expect("curl");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn curl_post(url: &str, body: &str) -> String {
    let out = std::process::Command::new("curl")
        .args([
            "-sS",
            "--max-time",
            "4",
            "-H",
            "content-type: application/json",
            "-d",
            body,
            url,
        ])
        .output()
        .expect("curl");
    String::from_utf8_lossy(&out.stdout).into_owned()
}
