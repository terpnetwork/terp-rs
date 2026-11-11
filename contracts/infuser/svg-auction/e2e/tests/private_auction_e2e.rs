//! Private auction e2e on `MockBech32`. The flow lives in
//! `terp_orch::suites::privacy::auctions` and is the same one the unified ict-rs suite
//! runs (`auctions-frost`).
//!
//! `cargo test -p svg-auction-e2e --test private_auction_e2e`
use cw_orch::prelude::*;
use terp_orch::suites::privacy::{Outcome, PrivateAuctionModule, ProductModule, mock_ctx};

#[test]
fn private_auction_on_mock() -> anyhow::Result<()> {
    let mut ctx = mock_ctx(MockBech32::new("terp"));
    let mut m = PrivateAuctionModule::<MockBech32>::default();
    m.deploy(&mut ctx)?;
    let r = m.exercise(&mut ctx)?;
    assert_eq!(r.outcome, Outcome::Passed, "{:?}", r.notes);
    Ok(())
}
