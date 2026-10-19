//! cw-orch Mock suite for terp-bounty-escrow: deploy through `Deploy`, then drive the
//! v3.2/v3.3 flow (summary + details hash/CID, CID-carrying submit, submit and upload stake gates) with the
//! generated `ExecuteMsgFns` / `QueryMsgFns`.
use bounty_escrow_scripts::*;
use cosmwasm_std::{Uint256, coin, coins};
use cw_orch::prelude::*;
use terp_bounty_escrow::msg::ExecuteMsg;
use terp_bounty_escrow::state::BountyStatus;

const UTERP: &str = "uterp";
const DETAILS_HASH: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
const SUB_HASH: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const CID_RAW: &str = "bafkreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku";
const CID_V0: &str = "QmY7Yh4UquoXHLPFo2XbhXkhBvFoPwmQUSa92pxnxjQuPU";

struct Actors {
    owner: Addr,
    approver: Addr,
    proposer: Addr,
    hunter: Addr,
}

fn setup(
    min_submit_stake: Uint256,
) -> anyhow::Result<(MockBech32, BountyEscrowSuite<MockBech32>, Actors)> {
    let chain = MockBech32::new("terp");
    let a = Actors {
        owner: chain.addr_make("owner"),
        approver: chain.addr_make("approver-dao"),
        proposer: chain.addr_make("proposer"),
        hunter: chain.addr_make("hunter"),
    };
    chain.add_balance(&a.proposer, coins(1_000_000_000_000, UTERP))?;
    let msg = init_msg(
        &a.owner,
        &a.approver,
        vec![a.proposer.to_string()],
        min_submit_stake,
    );
    let suite = BountyEscrowSuite::deploy_on(chain.clone(), msg)?;
    Ok((chain, suite, a))
}

fn create(
    chain: &MockBech32,
    suite: &BountyEscrowSuite<MockBech32>,
    a: &Actors,
    r: &str,
    amount: u128,
) -> anyhow::Result<u64> {
    let exp = expiry(chain.block_info()?.time);
    suite.escrow.call_as(&a.proposer).create_bounty(
        r.to_string(),
        DETAILS_HASH.to_string(),
        exp,
        coin(amount, UTERP),
        "Port the escrow into terp-rs.".to_string(),
        format!("bounty {r}"),
        Some(CID_RAW.to_string()),
        None,
        &coins(amount, UTERP),
    )?;
    Ok(suite.escrow.bounty_by_ref(r.to_string())?.id)
}

#[test]
fn full_lifecycle_through_cw_orch() -> anyhow::Result<()> {
    let (chain, suite, a) = setup(Uint256::zero())?;
    let amount = 5_000_000u128;
    let id = create(&chain, &suite, &a, "orch-1", amount)?;

    let b = suite.escrow.bounty(id)?;
    assert_eq!(b.status, BountyStatus::Pending);
    assert_eq!(b.summary, "Port the escrow into terp-rs.");
    assert_eq!(b.details_cid.as_deref(), Some(CID_RAW));

    suite.escrow.call_as(&a.approver).approve(id)?;
    suite
        .escrow
        .call_as(&a.approver)
        .assign(a.hunter.to_string(), id)?;
    assert!(suite.escrow.can_submit(a.hunter.to_string())?.eligible);
    suite
        .escrow
        .call_as(&a.hunter)
        .submit(CID_V0.to_string(), id, SUB_HASH.to_string())?;
    let subs = suite.escrow.submissions(id, None, None)?;
    assert_eq!(subs.submissions.len(), 1);
    assert_eq!(subs.submissions[0].cid, CID_V0);

    suite.escrow.call_as(&a.approver).accept(id)?;
    suite.escrow.call_as(&a.approver).payout(id, None)?;
    assert_eq!(suite.escrow.bounty(id)?.status, BountyStatus::Paid);
    assert_eq!(
        chain.query_balance(&a.hunter, UTERP)?,
        Uint256::from(amount)
    );
    Ok(())
}

#[test]
fn summary_cap_and_cid_checks_hold_through_the_interface() -> anyhow::Result<()> {
    let (chain, suite, a) = setup(Uint256::zero())?;
    let exp = expiry(chain.block_info()?.time);
    let too_long = "x".repeat(281);
    let err = suite
        .escrow
        .call_as(&a.proposer)
        .create_bounty(
            "r".to_string(),
            DETAILS_HASH.to_string(),
            exp,
            coin(5_000_000, UTERP),
            too_long,
            "t".to_string(),
            None,
            None,
            &coins(5_000_000, UTERP),
        )
        .unwrap_err();
    assert!(
        format!("{err:?}").contains("summary must be 1..=280 bytes"),
        "{err:?}"
    );

    let err = suite
        .escrow
        .call_as(&a.proposer)
        .create_bounty(
            "r".to_string(),
            DETAILS_HASH.to_string(),
            exp,
            coin(5_000_000, UTERP),
            "ok".to_string(),
            "t".to_string(),
            Some("Qm0OIl".to_string()),
            None,
            &coins(5_000_000, UTERP),
        )
        .unwrap_err();
    assert!(format!("{err:?}").contains("invalid IPFS CID"), "{err:?}");

    let id = create(&chain, &suite, &a, "orch-2", 5_000_000)?;
    suite.escrow.call_as(&a.approver).approve(id)?;
    suite
        .escrow
        .call_as(&a.approver)
        .assign(a.hunter.to_string(), id)?;
    let err = suite
        .escrow
        .call_as(&a.hunter)
        .submit("not-a-cid".to_string(), id, SUB_HASH.to_string())
        .unwrap_err();
    assert!(format!("{err:?}").contains("invalid IPFS CID"), "{err:?}");
    Ok(())
}

#[test]
fn stake_gate_blocks_an_unbonded_assignee() -> anyhow::Result<()> {
    let min = param_min_submit_stake();
    assert!(
        !min.is_zero(),
        "params file should carry a non-zero placeholder"
    );
    let (chain, suite, a) = setup(min)?;
    let id = create(&chain, &suite, &a, "orch-3", 5_000_000)?;
    suite.escrow.call_as(&a.approver).approve(id)?;
    suite
        .escrow
        .call_as(&a.approver)
        .assign(a.hunter.to_string(), id)?;

    let gate = suite.escrow.can_submit(a.hunter.to_string())?;
    assert!(!gate.eligible);
    assert_eq!(gate.staked, Uint256::zero());
    assert_eq!(gate.min_submit_stake, min);

    let err = suite
        .escrow
        .call_as(&a.hunter)
        .submit(CID_RAW.to_string(), id, SUB_HASH.to_string())
        .unwrap_err();
    assert!(
        format!("{err:?}").contains("below min_submit_stake"),
        "{err:?}"
    );
    assert_eq!(suite.escrow.bounty(id)?.status, BountyStatus::Assigned);
    Ok(())
}

#[test]
fn can_upload_reports_the_upload_stake_gate() -> anyhow::Result<()> {
    let min = param_min_upload_stake();
    assert!(
        !min.is_zero(),
        "params file should carry a non-zero placeholder"
    );
    assert!(
        min <= param_min_submit_stake(),
        "anyone who may submit may also upload"
    );
    let (_chain, suite, a) = setup(Uint256::zero())?;
    assert_eq!(suite.escrow.config()?.min_upload_stake, min);

    let gate = suite.escrow.can_upload(a.hunter.to_string())?;
    assert!(!gate.eligible);
    assert_eq!(gate.staked, Uint256::zero());
    assert_eq!(gate.min_upload_stake, min);
    // same bond denom as the submission gate (the Mock staking module's bonded denom)
    assert_eq!(
        gate.bond_denom,
        suite.escrow.can_submit(a.hunter.to_string())?.bond_denom
    );

    // governance turns the gate off (owner + new params_version)
    suite.escrow.call_as(&a.owner).execute(
        &ExecuteMsg::UpdateConfig {
            owner: None,
            approver: None,
            gate: None,
            denoms: None,
            allow_partial_payouts: None,
            fee_bps: None,
            fee_collector: None,
            min_duration_secs: None,
            max_duration_secs: None,
            params_version: Some("orch-upload-off".to_string()),
            params_placeholder: None,
            min_submit_stake: None,
            min_upload_stake: Some(Uint256::zero()),
            creation_gate: None,
        },
        &[],
    )?;
    assert!(suite.escrow.can_upload(a.hunter.to_string())?.eligible);
    Ok(())
}

#[test]
fn dao_proposal_module_queries_through_the_interface() -> anyhow::Result<()> {
    let (chain, suite, a) = setup(Uint256::zero())?;
    // cw2 info (DAO DAO `Info {}`), and `Dao {}` defaults to the instantiator (the deployer here)
    let info = suite.escrow.info()?;
    assert_eq!(info.info.contract, "crates.io:terp-bounty-escrow");
    assert_eq!(suite.escrow.dao()?, chain.sender_addr());
    assert_eq!(
        suite.escrow.proposal_creation_policy()?,
        ProposalCreationPolicy::Anyone {}
    );
    create(&chain, &suite, &a, "p1", 5_000_000)?;
    create(&chain, &suite, &a, "p2", 5_000_000)?;
    assert_eq!(suite.escrow.proposal_count()?, 2);
    assert_eq!(suite.escrow.next_proposal_id()?, 3);
    let rev = suite.escrow.reverse_proposals(None, None)?;
    assert_eq!(
        rev.proposals.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![2, 1]
    );
    // generated fns take args in field-name order: (limit, start_after)
    let list = suite.escrow.list_proposals(Some(1), None)?;
    assert_eq!(list.proposals.len(), 1);
    assert_eq!(list.proposals[0].proposal.bounty_ref, "p1");
    assert_eq!(
        suite.escrow.proposal(2)?.proposal.status,
        BountyStatus::Pending
    );
    assert!(suite.escrow.proposal_hooks()?.hooks.is_empty());
    Ok(())
}
