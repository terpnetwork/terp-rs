//! After Cash App ingress marks paid, door consume is usher-only `verify_consume`.
//! MockBech32 here is not Halo2 success (no `verify_halo2_proof` host).
//! Live chain: ict-rs Docker Terp + `MsgStoreCodeWithCircuit`.

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

/// Paid-from-ingress guests (8) buy; guest cw420 stays empty; MockBech32 cannot consume.
#[test]
fn cashapp_paid_guests_no_listmembers_mockbech32_not_consume_success() -> anyhow::Result<()> {
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

    let usher = chain.addr_make("usher1");
    chain.set_balance(&usher, vec![coin(1_000_000_000, "ujuno")])?;
    let guests: Vec<Addr> = (0..8)
        .map(|i| {
            let g = chain.addr_make(&format!("cashapp-guest-{i}"));
            chain
                .set_balance(&g, vec![coin(50_000_000, "ujuno")])
                .unwrap();
            g
        })
        .collect();

    let instantiate_msg = InstantiateMsg {
        event_curator: chain.sender_addr().to_string(),
        title: "Cash App rush door".into(),
        description: "ingress paid → consume".into(),
        usher_admins: vec![Member {
            addr: usher.to_string(),
            weight: 1,
        }],
        guest_details: vec![GuestDetails {
            guest_type: "attendee".into(),
            guest_weight: 0,
            max_ticket_limit: 8,
            total_ticket_limit: 32,
            ticket_cost: vec![coin(1_000_000, "ujuno")],
            event_segment_access: EventSegmentAccessType::SingleSegment { id: 0 },
        }],
        cw420: suite.cw420.code_id()?,
        event_timeline: vec![EventSegment {
            stage_description: "door".into(),
            start: Timestamp::from_seconds(1_000),
            end: Timestamp::from_seconds(9_000),
        }],
    };

    let cw_ave_addr = suite
        .cw_ave_factory
        .execute(
            &FactoryExecuteMsg::CreateNativeAvEventContract {
                instantiate_msg,
                label: "cashapp-rush".into(),
            },
            &[get_license_fee(&chain.env_info().chain_id)?],
        )?
        .event_attr_value("wasm", "new_ave_contract")?;
    suite.cw_ave.set_address(&Addr::unchecked(&cw_ave_addr));

    for g in &guests {
        suite.cw_ave.call_as(g).execute(
            &ExecuteMsg::PurchaseTickets {
                guests: vec![RegisteringGuest {
                    guest_weight: 0,
                    reap: vec![RegisteringEventAddressAndPayment {
                        ticket_addr: g.to_string(),
                        payment_asset: "ujuno".into(),
                    }],
                }],
            },
            &coins(1_000_000, "ujuno"),
        )?;
    }

    suite.cw_ave.execute(
        &ExecuteMsg::UpdateGuestRoot {
            guest_root: HexBinary::from(vec![0x22; 32]),
        },
        &[],
    )?;

    assert!(suite.cw_ave.is_usher(usher.to_string())?);
    let cfg = suite.cw_ave.config()?;
    suite.cw420.set_address(&cfg.event_guest_contract);
    let roster: MemberListResponse = suite.cw420.query(&cw420::msg::QueryMsg::ListMembers {
        start_after: None,
        limit: None,
    })?;
    assert!(roster.members.is_empty());

    let err = suite
        .cw_ave
        .call_as(&usher)
        .execute(
            &ExecuteMsg::VerifyConsume {
                proof: Binary::from(vec![9, 9, 9]),
                nullifier: HexBinary::from(vec![0x33; 32]),
                event_id: HexBinary::from(vec![0x44; 32]),
            },
            &[],
        )
        .expect_err("MockBech32 is not consume success");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("verify_halo2_proof") || msg.contains("zk-wasmvm"),
        "{msg}"
    );
    Ok(())
}
