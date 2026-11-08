//! This contract on `terp_orch::TerpArtifactStack`, and the cw-orch-kms
//! grant `app_setup` uses before any execute address exists.
//!
//! `cargo test -p svg-auction-e2e`

use cw_orch::prelude::*;
use cw_orch_kms::{ApprovalSession, instantiate2_bech32, wasm_checksum};
use terp_orch::TerpArtifactStack;

#[test]
fn on_the_terp_network_suite() {
    let suite = TerpArtifactStack::new(MockBech32::new("terp"));
    let _field = &suite.products.svg_auction;
}

#[test]
fn kms_grant_stores_and_instantiates_without_an_execute_address() {
    let wasm = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../../artifacts/svg_auction.wasm"
    ))
    .expect("artifacts/svg_auction.wasm");
    let predicted = instantiate2_bech32(
        "terp",
        &wasm_checksum(&wasm).unwrap(),
        &[7u8; 20],
        b"app-setup/svg-auction",
    )
    .unwrap();
    assert!(predicted.starts_with("terp1"));
    let open = ApprovalSession::open("terp1cold")
        .allow_store(&wasm)
        .expect("checksum")
        .allow_instantiate2();
    assert_eq!(open.stores.len(), 1);
    assert!(open.executes.is_empty());
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
}
