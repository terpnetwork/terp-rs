//! This contract on `terp_orch::TerpArtifactStack`, and the cw-orch-kms
//! grants `app_setup` uses for it.
//!
//! `cargo test -p terp-bounty-escrow-e2e`

use cw_orch::prelude::*;
use cw_orch_kms::{ApprovalSession, ScopedCall, instantiate2_bech32, wasm_checksum};
use terp_orch::TerpArtifactStack;

#[test]
fn on_the_terp_network_suite() {
    let suite = TerpArtifactStack::new(MockBech32::new("terp"));
    let _field = &suite.products.bounty_escrow;
}

#[test]
fn kms_grant_stores_then_instantiates_and_configures_only_this_contract() {
    let wasm = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../../artifacts/terp_bounty_escrow.wasm"
    ))
    .expect("artifacts/terp_bounty_escrow.wasm");
    let predicted = instantiate2_bech32(
        "terp",
        &wasm_checksum(&wasm).unwrap(),
        &[7u8; 20],
        b"app-setup/bounty-escrow",
    )
    .unwrap();
    let open = ApprovalSession::open("terp1cold")
        .allow_store(&wasm)
        .expect("checksum")
        .allow_instantiate2()
        .allow_execute(ScopedCall {
            address: predicted.clone(),
            keys: vec!["update_config".into()],
            calls: 1,
            funds: vec![],
        })
        .expect("address");
    assert_eq!(open.stores.len(), 1);
    assert_eq!(open.executes.len(), 1);
    assert_eq!(open.executes[0].address, predicted);
    assert_eq!(open.executes[0].keys, vec!["update_config".to_string()]);
    assert!(
        open.type_urls
            .iter()
            .any(|url| url.ends_with("MsgStoreCode"))
    );
    assert!(
        open.type_urls
            .iter()
            .any(|url| url.ends_with("MsgInstantiateContract2"))
    );
    assert!(!open.retain);
    assert!(
        ApprovalSession::open("terp1cold")
            .allow_execute(ScopedCall {
                address: String::new(),
                keys: vec!["update_config".into()],
                calls: 1,
                funds: vec![],
            })
            .is_err()
    );
}
