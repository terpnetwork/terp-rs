//! Private-bridge lab on Mock via [`terp_orch::TerpBridgeSuite`].
//!
//! Shell scripts under `tests/e2e/private-bridge/` remain host/container
//! notify-plane helpers. Contract + circuit Path A lives here.

#![cfg(feature = "bridge")]

use cw_orch::prelude::*;
use terp_orch::{write_demo_circuit_artifacts, TerpBridgeDeployData, TerpBridgeSuite};

#[test]
fn private_dex_lab_mock_verify_deploys() {
    write_demo_circuit_artifacts().expect("circuit artifacts");
    let mock = Mock::new("admin");
    let admin = mock.sender_addr();
    let suite = TerpBridgeSuite::deploy_on(
        mock,
        TerpBridgeDeployData {
            admin,
            mock_verify: true,
            zkid: None,
            allowed_root: Some(cosmwasm_std::Binary::from(vec![7u8; 32])),
        },
    )
    .expect("deploy private dex");
    suite.create_lab_pool().expect("create lab pool");
    let cfg: cw_private_dex::msg::ConfigResponse = suite
        .dex
        .query(&cw_private_dex::msg::QueryMsg::Config {})
        .expect("config");
    assert!(cfg.mock_verify);
    assert!(cfg.zkid.is_none());
}
