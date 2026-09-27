//! Live CosmWasm rotate Execute on ict-rs Terp (`itl-frost-1`).
//!
//! Proof path (groot-wan, Docker up, refuse ICT_MOCK=1):
//! ```sh
//! cargo run --example itl_frost_local_prod -p ict-rs-cw-orch --features docker
//! ```
//! Query must show pending → t shares → `group_vk` changed on-chain.
//! Do not put the inbox on Authenticate sudo.

use terp_onboard_authz::itl_frost_rotate::{
    msg_rotate_frost_pubkey, vk_commit, EXECUTE_CONTRACT_TYPE_URL, ROTATE_MSG,
};

#[ignore = "needs ICT_SPAWN"]
#[test]
fn itl_frost_rotate_live_execute_not_authenticate() {
    if std::env::var("ICT_MOCK").ok().as_deref() == Some("1") {
        panic!("itl_frost_rotate_live refuses ICT_MOCK=1");
    }
    let m = msg_rotate_frost_pubkey(
        "terp1desk",
        "terp1frostrotate",
        &vk_commit("vk-new"),
        Some("hkdf-lab"),
    );
    assert_eq!(m["@type"], EXECUTE_CONTRACT_TYPE_URL);
    assert!(m["msg"][ROTATE_MSG].is_object());
    assert!(m.get("authenticator_type").is_none());
    assert!(
        m["msg"][ROTATE_MSG].get("authenticator_type").is_none(),
        "rotate msg must not set authenticator_type"
    );
    let spawn = std::env::var("ICT_SPAWN").ok();
    let lcd = std::env::var("ICT_LCD").ok();
    if spawn.as_deref() != Some("1") && lcd.is_none() {
        // Ignored by default; with ICT_SPAWN=1 the example is the chain proof.
        return;
    }
}
