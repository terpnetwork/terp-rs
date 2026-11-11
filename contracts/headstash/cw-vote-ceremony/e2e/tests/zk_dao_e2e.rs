//! zk proposal module e2e on `MockBech32`: a cw4 DAO (dao-core, dao-voting-cw4,
//! proposal-single, pre-propose-single) adds the module through its own governance, then
//! runs a private vote, an EA tally and a treasury payout. The flow lives in
//! `terp_orch::suites::privacy::zk_voting` (`dao-zk-voting` in the unified suite).
//!
//! `cargo test -p cw-vote-ceremony-e2e --test zk_dao_e2e`
use cw_orch::prelude::*;
use terp_orch::suites::privacy::{Outcome, ProductModule, ZkVotingModule, mock_ctx};

#[test]
fn existing_dao_adds_zk_module_and_runs_a_private_vote() -> anyhow::Result<()> {
    let mut ctx = mock_ctx(MockBech32::new("terp"));
    let mut m = ZkVotingModule::<MockBech32>::default();
    m.deploy(&mut ctx)?;
    let r = m.exercise(&mut ctx)?;
    assert_eq!(r.outcome, Outcome::Passed, "{:?}", r.notes);
    Ok(())
}
