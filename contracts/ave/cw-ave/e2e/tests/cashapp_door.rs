//! After Cash App ingress marks paid, door consume is usher-only `verify_consume`.
//! MockBech32 here is not Halo2 success (no `verify_halo2_proof` host).
//! Live chain: ict-rs Docker Terp + `MsgStoreCodeWithCircuit`.

use cosmwasm_std::{Binary, HexBinary, Timestamp, coin, coins};
use cw_ave::msg::{ExecuteMsg, InstantiateMsg, QueryMsgFns};
use cw_ave::state::{
    EventSegment, EventSegmentAccessType, GuestDetails, RegisteringEventAddressAndPayment,
    RegisteringGuest,
};
use cw_ave_factory::msg::{ExecuteMsg as FactoryExecuteMsg, InstantiateMsg as FactoryInitMsg};
use cw_orch::{anyhow, prelude::*};
use cw4::{Member, MemberListResponse};

use cw_ave_e2e::{license_fee, upload_ave};

/// Paid-from-ingress guests (8) buy; guest cw420 stays empty; MockBech32 cannot consume.
#[test]
fn cashapp_paid_guests_no_listmembers_mockbech32_not_consume_success() -> anyhow::Result<()> {
    let chain = MockBech32::new_with_chain_id("mock", "juno-1");
    chain.set_balance(&chain.sender_addr(), vec![coin(1_000_000_000_000, "ujuno")])?;
    let suite = upload_ave(chain.clone())?;
    suite.products.ave_factory.instantiate(
        &FactoryInitMsg {
            owner: None,
            cw_ave_id: suite.products.ave.code_id()?,
        },
        Some(&chain.sender_addr()),
        &[license_fee(&chain.env_info().chain_id)?],
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
        cw420: suite.products.cw420.code_id()?,
        zkid: 0,
        event_timeline: vec![EventSegment {
            stage_description: "door".into(),
            start: Timestamp::from_seconds(1_000),
            end: Timestamp::from_seconds(9_000),
        }],
    };

    let cw_ave_addr = suite
        .products
        .ave_factory
        .execute(
            &FactoryExecuteMsg::CreateNativeAvEventContract {
                instantiate_msg,
                label: "cashapp-rush".into(),
            },
            &[license_fee(&chain.env_info().chain_id)?],
        )?
        .event_attr_value("wasm", "new_ave_contract")
        .map_err(|err| anyhow::anyhow!("{err}"))?;
    suite
        .products
        .ave
        .set_address(&Addr::unchecked(&cw_ave_addr));

    for g in &guests {
        suite.products.ave.call_as(g).execute(
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

    suite.products.ave.execute(
        &ExecuteMsg::UpdateGuestRoot {
            guest_root: HexBinary::from(vec![0x22; 32]),
        },
        &[],
    )?;

    assert!(suite.products.ave.is_usher(usher.to_string())?);
    let cfg = suite.products.ave.config()?;
    suite.products.cw420.set_address(&cfg.event_guest_contract);
    let roster: MemberListResponse =
        suite
            .products
            .cw420
            .query(&cw420::msg::QueryMsg::ListMembers {
                start_after: None,
                limit: None,
            })?;
    assert!(roster.members.is_empty());

    let err = suite
        .products
        .ave
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
    // zkid 0 is rejected in cw-ave before the host import (`VerificationError::GenericErr`).
    assert!(
        msg.contains("verify_halo2_proof") || msg.contains("zk-wasmvm") || msg.contains("Generic"),
        "{msg}"
    );
    Ok(())
}
