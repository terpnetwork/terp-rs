use av_event_helpers::get_license_fee;
use cosmwasm_std::{coin, coins, Binary, HexBinary, Timestamp};
use cw4::{Member, MemberListResponse};
use cw_ave::msg::{ExecuteMsg, InstantiateMsg, QueryMsgFns};
use cw_ave::state::{
    EventSegment, EventSegmentAccessType, GuestDetails, RegisteringEventAddressAndPayment,
    RegisteringGuest,
};
use cw_ave_factory::msg::{ExecuteMsg as FactoryExecuteMsg, InstantiateMsg as FactoryInitMsg};
use cw_orch::{anyhow, prelude::*};

use crate::interfaces::CwAveSuite;

fn err_text(err: &impl std::fmt::Debug) -> String {
    format!("{err:?}")
}

struct CheckinEnv {
    suite: CwAveSuite<MockBech32>,
    usher1: Addr,
    usher2: Addr,
    guest: Addr,
    outsider: Addr,
}

impl CheckinEnv {
    fn setup() -> anyhow::Result<Self> {
        let chain = MockBech32::new_with_chain_id("mock", "juno-1");
        chain.set_balance(&chain.sender_addr(), vec![coin(1_000_000_000_000, "ujuno")])?;
        let suite = CwAveSuite::deploy_on(chain.clone(), ())?;

        suite.cw_ave_factory.instantiate(
            &FactoryInitMsg {
                owner: None,
                cw_ave_id: suite.cw_ave.code_id()?,
            },
            Some(&chain.sender_addr()),
            &[get_license_fee(&chain.env_info().chain_id)?],
        )?;

        let usher1 = chain.addr_make("usher1");
        let usher2 = chain.addr_make("usher2");
        let guest = chain.addr_make("guest");
        let outsider = chain.addr_make("not-an-usher");
        chain.set_balance(&usher1, vec![coin(1_000_000_000, "ujuno")])?;
        chain.set_balance(&usher2, vec![coin(1_000_000_000, "ujuno")])?;
        chain.set_balance(&guest, vec![coin(1_000_000_000, "ujuno")])?;
        chain.set_balance(&outsider, vec![coin(1_000_000_000, "ujuno")])?;

        let instantiate_msg = InstantiateMsg {
            event_curator: chain.sender_addr().to_string(),
            title: "Lab two-usher gate".to_string(),
            description: "itlusher consume e2e".to_string(),
            usher_admins: vec![
                Member {
                    addr: usher1.to_string(),
                    weight: 1,
                },
                Member {
                    addr: usher2.to_string(),
                    weight: 1,
                },
            ],
            guest_details: vec![GuestDetails {
                guest_type: "attendee".to_string(),
                guest_weight: 0,
                max_ticket_limit: 8,
                total_ticket_limit: 32,
                ticket_cost: vec![coin(1_000_000, "ujuno")],
                event_segment_access: EventSegmentAccessType::SingleSegment { id: 0 },
            }],
            cw420: suite.cw420.code_id()?,
            event_timeline: vec![EventSegment {
                stage_description: "door".to_string(),
                start: Timestamp::from_seconds(1_000),
                end: Timestamp::from_seconds(2_000),
            }],
        };

        let cw_ave_addr = suite
            .cw_ave_factory
            .execute(
                &FactoryExecuteMsg::CreateNativeAvEventContract {
                    instantiate_msg,
                    label: "lab-two-usher".into(),
                },
                &[get_license_fee(&chain.env_info().chain_id)?],
            )?
            .event_attr_value("wasm", "new_ave_contract")?;
        suite.cw_ave.set_address(&Addr::unchecked(&cw_ave_addr));

        Ok(Self {
            suite,
            usher1,
            usher2,
            guest,
            outsider,
        })
    }

    fn buy_guest_ticket(&self) -> anyhow::Result<()> {
        self.suite.cw_ave.execute(
            &ExecuteMsg::PurchaseTickets {
                guests: vec![RegisteringGuest {
                    guest_weight: 0,
                    reap: vec![RegisteringEventAddressAndPayment {
                        ticket_addr: self.guest.to_string(),
                        payment_asset: "ujuno".to_string(),
                    }],
                }],
            },
            &coins(1_000_000, "ujuno"),
        )?;
        Ok(())
    }
}

fn dummy_consume() -> (Binary, HexBinary, HexBinary) {
    (
        Binary::from(vec![1, 2, 3]),
        HexBinary::from(vec![0xab; 32]),
        HexBinary::from(vec![0xcd; 32]),
    )
}

#[test]
fn is_usher_queryable_guest_listmembers_empty() -> anyhow::Result<()> {
    let env = CheckinEnv::setup()?;
    env.buy_guest_ticket()?;
    assert!(env.suite.cw_ave.is_usher(env.usher1.to_string())?);
    assert!(env.suite.cw_ave.is_usher(env.usher2.to_string())?);
    assert!(!env.suite.cw_ave.is_usher(env.outsider.to_string())?);
    assert!(!env.suite.cw_ave.is_usher(env.guest.to_string())?);

    let cfg = env.suite.cw_ave.config()?;
    env.suite.cw420.set_address(&cfg.event_guest_contract);
    let roster: MemberListResponse = env.suite.cw420.query(&cw420::msg::QueryMsg::ListMembers {
        start_after: None,
        limit: None,
    })?;
    assert!(
        roster.members.is_empty(),
        "guest ListMembers must stay empty, got {:?}",
        roster.members
    );
    Ok(())
}

#[test]
fn non_usher_verify_consume_returns_not_an_event_usher() -> anyhow::Result<()> {
    let env = CheckinEnv::setup()?;
    let (proof, nullifier, event_id) = dummy_consume();
    let err = env
        .suite
        .cw_ave
        .call_as(&env.outsider)
        .execute(
            &ExecuteMsg::VerifyConsume {
                proof,
                nullifier,
                event_id,
            },
            &[],
        )
        .expect_err("outsider must fail");
    let msg = err_text(&err);
    assert!(
        msg.contains("NotAnEventUsher") || msg.contains("not an usher"),
        "unexpected error: {msg}"
    );
    Ok(())
}

#[test]
fn usher_verify_consume_without_root_fails() -> anyhow::Result<()> {
    let env = CheckinEnv::setup()?;
    let (proof, nullifier, event_id) = dummy_consume();
    let err = env
        .suite
        .cw_ave
        .call_as(&env.usher1)
        .execute(
            &ExecuteMsg::VerifyConsume {
                proof,
                nullifier,
                event_id,
            },
            &[],
        )
        .expect_err("no guest_root");
    let msg = err_text(&err);
    assert!(
        msg.contains("GuestRootNotSet") || msg.contains("guest_root"),
        "unexpected error: {msg}"
    );
    Ok(())
}

#[test]
fn mockbech32_is_not_halo2_consume_success() -> anyhow::Result<()> {
    let env = CheckinEnv::setup()?;
    env.suite.cw_ave.execute(
        &ExecuteMsg::UpdateGuestRoot {
            guest_root: HexBinary::from(vec![0x11; 32]),
        },
        &[],
    )?;
    let (proof, nullifier, event_id) = dummy_consume();
    let err = env
        .suite
        .cw_ave
        .call_as(&env.usher1)
        .execute(
            &ExecuteMsg::VerifyConsume {
                proof,
                nullifier,
                event_id,
            },
            &[],
        )
        .expect_err("MockBech32 has no verify_halo2_proof");
    let msg = err_text(&err);
    assert!(
        msg.contains("verify_halo2_proof") || msg.contains("zk-wasmvm") || msg.contains("Pasta"),
        "unexpected error: {msg}"
    );
    assert!(!env
        .suite
        .cw_ave
        .nullifier_spent(HexBinary::from(vec![0xab; 32]))?);
    Ok(())
}
