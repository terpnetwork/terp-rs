//! Unified privacy-products suite, offline (`MockBech32`).
//!
//! `cargo test -p terp-orch --features privacy --test privacy_products`
//! Live (ict-rs Terp + zakura + anvil): see `~/grok-bot/privacy-products/e2e/SUITE.md`.
#![cfg(feature = "privacy")]

use cw_orch::prelude::*;
use terp_orch::suites::privacy::{mock_ctx, ChainSlot, Outcome, PrivacySuite};

#[test]
fn every_product_slot_runs_on_one_chain() {
    let chain = MockBech32::new("terp");
    let mut suite = PrivacySuite::<MockBech32>::standard();
    assert_eq!(
        suite.required_chains(),
        vec![ChainSlot::Terp, ChainSlot::Zakura, ChainSlot::Anvil]
    );
    let mut ctx = mock_ctx(chain);
    let reports = suite.run(&mut ctx);
    let mut passed = 0;
    for r in &reports {
        match r {
            Ok(r) => {
                println!(
                    "[{:?}] {} → {:?} {:?}",
                    r.workstream, r.id, r.outcome, r.notes
                );
                if r.outcome == Outcome::Passed {
                    passed += 1;
                }
            }
            Err(e) => panic!("product failed: {e:#}"),
        }
    }
    assert_eq!(
        passed, 4,
        "lc-registry, auctions-frost, dao-zk-voting, seams-l0 pass; bridge-tacit is pending"
    );
}
