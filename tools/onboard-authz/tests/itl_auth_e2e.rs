//! ITL Speakeasy product suite — same crate as combo / ict_workflow.
//!
//! Local (always):
//!   cargo test --manifest-path tools/onboard-authz/Cargo.toml --test itl_auth_e2e
//!
//! Live LCD query (optional):
//!   ICT_LCD=https://testnet-api.terp.network ICT_CHAIN=120u-1 \
//!     cargo test --manifest-path tools/onboard-authz/Cargo.toml --test itl_auth_e2e -- --ignored --nocapture
//!
//! Live add/rotate needs a signer + feegrant (HashMerchant onboard). Set
//! ITL_AUTH_ADDR + ITL_AUTH_BROADCAST=1. Until then the in-process rotate is SoT.

use terp_onboard_authz::itl_auth::{
    add_device, lcd_authenticators_path, local_anyof_rotate, msg_add_any_of,
    msg_remove_authenticator, parse_authenticator_ids, query_json_looks_like_account, rotate,
    zip321_ok, AuthAccount, ComposeOp, Device,
};
use terp_onboard_authz::{
    authenticators_query_ok, msg_add_authenticator, ADD_AUTHENTICATOR_TYPE_URL,
    AUTHENTICATOR_ANY_OF, AUTHENTICATOR_SIGNATURE, REMOVE_AUTHENTICATOR_TYPE_URL,
};

#[test]
fn itl_local_add_query_anyof_rotate() {
    let acc = local_anyof_rotate();
    assert_eq!(acc.policy, ComposeOp::AnyOf);
    assert_eq!(acc.devices.len(), 2, "after rotate: two devices (2 and 3)");
    let ids: Vec<u64> = acc.devices.iter().filter_map(|d| d.id).collect();
    assert_eq!(ids, vec![2, 3]);

    let add = msg_add_authenticator(
        &acc.sender,
        AUTHENTICATOR_SIGNATURE,
        &acc.devices[0].data_b64,
    );
    assert_eq!(add["@type"], ADD_AUTHENTICATOR_TYPE_URL);
    let wrap = msg_add_any_of(&acc.sender, &acc.devices);
    assert_eq!(wrap["authenticator_type"], AUTHENTICATOR_ANY_OF);
    let rm = msg_remove_authenticator(&acc.sender, 2);
    assert_eq!(rm["@type"], REMOVE_AUTHENTICATOR_TYPE_URL);

    let q = format!(
        r#"{{"authenticators":[{},{}]}}"#,
        r#"{"id":"2","type":"SignatureVerification"}"#,
        r#"{"id":"3","type":"SignatureVerification"}"#,
    );
    assert!(authenticators_query_ok(&q));
    assert_eq!(parse_authenticator_ids(&q), vec![2, 3]);
}

#[test]
fn itl_zip321_and_compose_match_dregg_rules() {
    assert!(zip321_ok("zcash:u1lab?amount=0.068&memo=YWJj"));
    assert!(!zip321_ok("zcash://u1lab"));
    let d = Device {
        label: "phone".into(),
        id: Some(1),
        authenticator_type: AUTHENTICATOR_SIGNATURE.into(),
        data_b64: "cGhvbmU".into(),
    };
    let m = msg_add_any_of("terp1guest", &[d]);
    assert_eq!(m["sender"], "terp1guest");
}

#[test]
fn itl_rotate_is_add_then_remove() {
    let mut acc = AuthAccount {
        sender: "terp1guest".into(),
        policy: ComposeOp::AnyOf,
        devices: vec![],
    };
    add_device(&mut acc, "laptop", "bA");
    let old = acc.devices[0].id.unwrap();
    let new_id = rotate(&mut acc, old, "phone", "cA").unwrap();
    assert_ne!(new_id, old);
    assert_eq!(acc.devices.len(), 1);
    assert_eq!(acc.devices[0].label, "phone");
}

/// Live: GET authenticators for ITL_AUTH_ADDR (or a dummy) on ICT_LCD.
/// Empty list is ok (account has not registered yet). Transport must work.
#[test]
#[ignore = "needs ICT_LCD"]
fn live_lcd_query_authenticators() {
    let lcd = std::env::var("ICT_LCD").expect("ICT_LCD");
    let addr = std::env::var("ITL_AUTH_ADDR").unwrap_or_else(|_| {
        "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqg6p2x0".into()
    });
    let mut last = String::new();
    for path in lcd_authenticators_path(&addr) {
        let url = format!("{}{path}", lcd.trim_end_matches('/'));
        last = curl_get(&url);
        eprintln!("GET {url} → {} bytes", last.len());
        if query_json_looks_like_account(&last) || last.contains("code") {
            break;
        }
    }
    assert!(
        last.contains("authenticator")
            || last.contains("not found")
            || last.contains("code")
            || last.contains("error"),
        "unexpected LCD body: {last}"
    );
}

fn curl_get(url: &str) -> String {
    let out = std::process::Command::new("curl")
        .args(["-sS", "--max-time", "8", url])
        .output()
        .expect("curl");
    String::from_utf8_lossy(&out.stdout).into_owned()
}
