use av_event_helpers::get_license_fee;
use cosmwasm_std::{coin, coins, Timestamp};
use cw4::Member;
use cw_ave::msg::{ExecuteMsg, InstantiateMsg, QueryMsgFns};
use cw_ave::state::{
    Config, EventSegment, EventSegmentAccessType, GuestDetails, RegisteringEventAddressAndPayment,
    RegisteringGuest,
};
use cw_ave_factory::msg::{ExecuteMsg as FactoryExecuteMsg, InstantiateMsg as FactoryInitMsg};
use cw_orch::{anyhow, prelude::*};

use crate::interfaces::CwAveSuite;

/// INIT UNIT TESTS
// calling contract with funds:
// -  suite.cw_ave.execute(execute_msg, coins)
// calling contract w/out funds (queries & execute)
// -  suite.cw_ave.config()
// to call as another address
// - suite.cw_ave.call_as()

// only event curator & event ushers can checkin guests
// cannot have any duplicate guest weights
// event segments start & end must be correct sequential
// ensure the previous end date is before or at the next start date
// event stage description length is accurate
// event stage description length is accurate
// config is set and able to be queried accurately
// prevent duplicate fee denoms set for a guest details

// event stages are set accurately (stars at 1, not 0)
// ownership of event usher & guest contracts are set accurately

// PURCHASING TICKETS
// ensure we return and overflow funds sent
// ensure ticket limit is enforced per ticket purchaser
// ensure ticket limit is enforced for guestdetails
// ensure funds are going to correct destination

// test count_tickets_and_remainder function
// - ensure dev fee is accurate
// - ensure expected amount of tickets are set to be allocated

// CHECKIN GUEST
// only event ushers can checkin guest

/// EVENT SEGMENT ACCESS
/// - single - works normal
/// - single - errors on incorrect segment for a guest
/// 

struct TestEnv<Env: CwEnv> {
    mock: Env,
    suite: CwAveSuite<Env>,
}

impl TestEnv<MockBech32> {
    /// Set up the test environment with an Account that has the Standalone installed
    fn setup() -> anyhow::Result<TestEnv<MockBech32>> {
        // Create a sender and mock env
        let chain = MockBech32::new_with_chain_id("mock", "juno-1");
        chain.set_balance(&chain.sender_addr(), vec![coin(1000000000000, "ujuno")])?;
        let suite = CwAveSuite::deploy_on(chain.clone(), ())?;

        // instantiate factory
        suite.cw_ave_factory.instantiate(
            &FactoryInitMsg {
                owner: None,
                cw_ave_id: suite.cw_ave.code_id()?,
            },
            Some(&chain.sender_addr()),
            &[get_license_fee(&chain.env_info().chain_id)?],
        )?;

        // Create sample guest details
        let guest_details = vec![GuestDetails {
            guest_type: "VIP".to_string(),
            guest_weight: 1,
            max_ticket_limit: 5,
            total_ticket_limit: 100,
            ticket_cost: vec![coin(1000000, "ujuno")],
            event_segment_access: EventSegmentAccessType::SingleSegment { id: 0 },
        }];

        // Create sample event timeline
        let event_timeline = vec![EventSegment {
            stage_description: "Main Event".to_string(),
            start: Timestamp::from_seconds(1000),
            end: Timestamp::from_seconds(2000),
        }];

        // Instantiate the cw-ave contract
        let instantiate_msg = InstantiateMsg {
            event_curator: chain.sender_addr().to_string(),
            title: "Test Event".to_string(),
            description: "Test Description".to_string(),
            usher_admins: vec![Member {
                addr: chain.sender_addr().to_string(),
                weight: 1,
            }],
            guest_details,
            cw420: suite.cw420.code_id()?,
            event_timeline,
        };

        let cw_ave_addr = suite
            .cw_ave_factory
            .execute(
                &FactoryExecuteMsg::CreateNativeAvEventContract {
                    instantiate_msg: instantiate_msg,
                    label: "checkin".into(),
                },
                &[get_license_fee(&chain.env_info().chain_id)?],
            )?
            .event_attr_value("wasm", "new_ave_contract")?;

        suite.cw_ave.set_address(&Addr::unchecked(&cw_ave_addr));

        println!("{:#?}", chain.state());
        Ok(TestEnv { mock: chain, suite })
    }
}

#[test]
fn test_successful_instantiate() -> anyhow::Result<()> {
    let env = TestEnv::setup()?;

    // Test that config is set correctly
    let config: Config = env.suite.cw_ave.config()?;
    assert_eq!(config.title, "Test Event");
    assert_eq!(config.curator, env.mock.sender_addr());

    Ok(())
}

#[test]
fn test_duplicate_guest_weight_fails() -> anyhow::Result<()> {
    let t = TestEnv::setup()?;

    println!("{:#?}", t.mock.state());
    // Create guest details with duplicate weights
    let guest_details = vec![
        GuestDetails {
            guest_type: "VIP".to_string(),
            guest_weight: 1,
            max_ticket_limit: 5,
            total_ticket_limit: 100,
            ticket_cost: vec![coin(1000000, "ujuno")],
            event_segment_access: EventSegmentAccessType::SingleSegment { id: 0 },
        },
        GuestDetails {
            guest_type: "Regular".to_string(),
            guest_weight: 1, // Duplicate weight
            max_ticket_limit: 10,
            total_ticket_limit: 500,
            ticket_cost: vec![coin(500000, "ujuno")],
            event_segment_access: EventSegmentAccessType::SingleSegment { id: 0 },
        },
    ];

    let event_timeline = vec![EventSegment {
        stage_description: "Main Event".to_string(),
        start: Timestamp::from_seconds(1000),
        end: Timestamp::from_seconds(2000),
    }];

    let instantiate_msg = InstantiateMsg {
        event_curator: t.mock.sender_addr().to_string(),
        title: "Test Event".to_string(),
        description: "Test Description".to_string(),
        usher_admins: vec![Member {
            addr: t.mock.sender_addr().to_string(),
            weight: 1,
        }],
        guest_details,
        cw420: t.suite.cw420.code_id()?,
        event_timeline,
    };

    // This should fail due to duplicate guest weight
    let result = t.suite.cw_ave.instantiate(
        &instantiate_msg,
        Some(&t.mock.sender_addr()),
        &[get_license_fee(&t.mock.env_info().chain_id)?],
    );

    assert!(result.is_err());
    Ok(())
}

#[test]
fn test_invalid_event_dates_fails() -> anyhow::Result<()> {
    let chain = MockBech32::new_with_chain_id("mock", "juno-1");
    let t = TestEnv::setup()?;

    let guest_details = vec![GuestDetails {
        guest_type: "VIP".to_string(),
        guest_weight: 1,
        max_ticket_limit: 5,
        total_ticket_limit: 100,
        ticket_cost: vec![coin(1000000, "ujuno")],
        event_segment_access: EventSegmentAccessType::SingleSegment { id: 0 },
    }];

    // Create event timeline with invalid dates (start > end)
    let event_timeline = vec![EventSegment {
        stage_description: "Main Event".to_string(),
        start: Timestamp::from_seconds(2000),
        end: Timestamp::from_seconds(1000), // End before start
    }];

    let instantiate_msg = InstantiateMsg {
        event_curator: chain.sender_addr().to_string(),
        title: "Test Event".to_string(),
        description: "Test Description".to_string(),
        usher_admins: vec![Member {
            addr: chain.sender_addr().to_string(),
            weight: 1,
        }],
        guest_details,
        cw420: t.suite.cw420.code_id()?,
        event_timeline,
    };

    // This should fail due to invalid event dates
    let result = t.suite.cw_ave.instantiate(
        &instantiate_msg,
        Some(&chain.sender_addr()),
        &[get_license_fee(&chain.env_info().chain_id)?],
    );

    assert!(result.is_err());
    Ok(())
}

#[test]
fn test_overlapping_event_dates_fails() -> anyhow::Result<()> {
    let chain = MockBech32::new_with_chain_id("mock", "juno-1");
    let t = TestEnv::setup()?;

    let guest_details = vec![GuestDetails {
        guest_type: "VIP".to_string(),
        guest_weight: 1,
        max_ticket_limit: 5,
        total_ticket_limit: 100,
        ticket_cost: vec![coin(1000000, "ujuno")],
        event_segment_access: EventSegmentAccessType::SingleSegment { id: 0 },
    }];

    // Create overlapping event timeline
    let event_timeline = vec![
        EventSegment {
            stage_description: "Event 1".to_string(),
            start: Timestamp::from_seconds(1000),
            end: Timestamp::from_seconds(2000),
        },
        EventSegment {
            stage_description: "Event 2".to_string(),
            start: Timestamp::from_seconds(1500), // Overlaps with previous event
            end: Timestamp::from_seconds(2500),
        },
    ];

    let instantiate_msg = InstantiateMsg {
        event_curator: chain.sender_addr().to_string(),
        title: "Test Event".to_string(),
        description: "Test Description".to_string(),
        usher_admins: vec![Member {
            addr: chain.sender_addr().to_string(),
            weight: 1,
        }],
        guest_details,
        cw420: t.suite.cw420.code_id()?,
        event_timeline,
    };

    // This should fail due to overlapping event dates
    let result = t.suite.cw_ave.instantiate(
        &instantiate_msg,
        Some(&chain.sender_addr()),
        &[get_license_fee(&chain.env_info().chain_id)?],
    );

    assert!(result.is_err());
    Ok(())
}

#[test]
fn test_purchase_tickets_success() -> anyhow::Result<()> {
    let t = TestEnv::setup()?;

    let guest_wallet = t.mock.addr_make("guest1");

    // Create purchase request
    let purchase_request = vec![RegisteringGuest {
        guest_weight: 1,
        reap: vec![RegisteringEventAddressAndPayment {
            ticket_addr: guest_wallet.to_string(),
            payment_asset: "ujuno".to_string(),
        }],
    }];

    // Purchase tickets with sufficient funds
    let result = t.suite.cw_ave.execute(
        &ExecuteMsg::PurchaseTickets {
            guests: purchase_request,
        },
        &coins(1000000, "ujuno"),
    );

    assert!(result.is_ok());
    Ok(())
}

#[test]
fn test_purchase_tickets_insufficient_funds() -> anyhow::Result<()> {
    let t = TestEnv::setup()?;

    let guest_wallet = t.mock.addr_make("guest1");

    // Create purchase request
    let purchase_request = vec![RegisteringGuest {
        guest_weight: 1,
        reap: vec![RegisteringEventAddressAndPayment {
            ticket_addr: guest_wallet.to_string(),
            payment_asset: "ujuno".to_string(),
        }],
    }];

    // Purchase tickets with insufficient funds
    let result = t.suite.cw_ave.execute(
        &ExecuteMsg::PurchaseTickets {
            guests: purchase_request,
        },
        &coins(100000, "ujuno"), // Less than required 1000000
    );

    // Should not error but tickets won't be allocated
    assert!(result.is_ok());
    Ok(())
}

#[test]
fn test_claim_ticket_payments_curator_only() -> anyhow::Result<()> {
    let mut t = TestEnv::setup()?;

    // Try to claim as curator (should succeed)
    let result = t
        .suite
        .cw_ave
        .execute(&ExecuteMsg::ClaimTicketPayments {}, &[]);

    assert!(result.is_ok());

    // Try to claim as different user (should fail)
    let other_user = t.mock.addr_make("other_user");
    t.mock.set_sender(other_user);

    let result = t
        .suite
        .cw_ave
        .execute(&ExecuteMsg::ClaimTicketPayments {}, &[]);

    assert!(result.is_err());
    Ok(())
}

#[test]
fn test_query_functions() -> anyhow::Result<()> {
    let t = TestEnv::setup()?;

    // Test config query
    let config: Config = t.suite.cw_ave.config()?;
    assert_eq!(config.title, "Test Event");

    // Test event segments query
    let segments = t.suite.cw_ave.event_segments()?;
    assert_eq!(segments.len(), 1);

    // Test guest details query
    let guest_details = t.suite.cw_ave.guest_type_details_by_weight(1)?;
    assert_eq!(guest_details.guest_type, "VIP");

    // Test all guest details query
    let all_guest_details = t.suite.cw_ave.guest_type_details_all()?;
    assert_eq!(all_guest_details.len(), 1);

    // Test ticket payment options query
    let payment_options = t.suite.cw_ave.ticket_payment_options_by_guest_weight(1)?;
    assert_eq!(payment_options.guest_type, "VIP");
    assert_eq!(payment_options.payment_options.len(), 1);

    // Test all payment options query
    let all_payment_options = t.suite.cw_ave.all_ticket_payment_options()?;
    assert_eq!(all_payment_options.len(), 1);

    Ok(())
}
