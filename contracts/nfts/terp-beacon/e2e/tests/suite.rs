//! This contract on `terp_orch::TerpArtifactStack`.
//!
//! `cargo test -p terp-beacon-e2e`

use cw_orch::prelude::*;
use terp_orch::TerpArtifactStack;

#[test]
fn on_the_terp_network_suite() {
    let suite = TerpArtifactStack::new(MockBech32::new("terp"));
    let _field = &suite.products.terp_beacon;
}
