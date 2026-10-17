use av_event_helpers::LICENSE_CANONICAL_ADDR;
use cosmwasm_std::testing::mock_dependencies;
use cosmwasm_std::{coins, Addr, Api, CanonicalAddr, Coin, Empty, Timestamp, Uint128};
use cw4::Member;
use cw_ave::msg::InstantiateMsg as AvEventInstantiateMsg;
use cw_ave::state::{EventSegment, EventSegmentAccessType, GuestDetails};
use cw_ave::ContractError as CwAveContractError;
use cw_multi_test::{App, BankSudo, Contract, ContractWrapper, Executor, SudoMsg};
use cw_ownable::OwnershipError;

use crate::{
    msg::{ExecuteMsg, InstantiateMsg, QueryMsg},
    state::AvEventContract,
    ContractError,
};

use easy_addr::addr;

const ALICE: &str = addr!("alice");
const BOB: &str = addr!("bob");
const CHARLIE: &str = addr!("charlie");
const INITIAL_BALANCE: u128 = 1000000000;
const NATIVE_DENOM: &str = "uatom";

fn factory_contract() -> Box<dyn Contract<Empty>> {
    let contract = ContractWrapper::new(
        crate::contract::execute,
        crate::contract::instantiate,
        crate::contract::query,
    )
    .with_reply(crate::contract::reply);
    Box::new(contract)
}

fn cw420_contract() -> Box<dyn Contract<Empty>> {
    let contract = ContractWrapper::new(
        cw420::contract::execute,
        cw420::contract::instantiate,
        cw420::contract::query,
    );
    Box::new(contract)
}

fn cw_ave_contract() -> Box<dyn Contract<Empty>> {
    let contract = ContractWrapper::new(
        cw_ave::contract::execute,
        cw_ave::contract::instantiate,
        cw_ave::contract::query,
    );
    Box::new(contract)
}

fn setup_app() -> App {
    let mut app = App::default();
    // Mint initial balances
    app.sudo(SudoMsg::Bank(BankSudo::Mint {
        to_address: ALICE.to_string(),
        amount: coins(INITIAL_BALANCE, NATIVE_DENOM),
    }))
    .unwrap();
    app.sudo(SudoMsg::Bank(BankSudo::Mint {
        to_address: BOB.to_string(),
        amount: coins(INITIAL_BALANCE, NATIVE_DENOM),
    }))
    .unwrap();
    app.sudo(SudoMsg::Bank(BankSudo::Mint {
        to_address: CHARLIE.to_string(),
        amount: coins(INITIAL_BALANCE, NATIVE_DENOM),
    }))
    .unwrap();
    app
}

fn create_valid_ave_instantiate_msg(cw420_code_id: u64) -> AvEventInstantiateMsg {
    AvEventInstantiateMsg {
        title: "Test Event".to_string(),
        description: "A test event".to_string(),
        event_curator: ALICE.to_string(),
        usher_admins: vec![Member {
            addr: ALICE.to_string(),
            weight: 1,
        }],
        guest_details: vec![
            GuestDetails {
                guest_type: "VIP".to_string(),
                guest_weight: 100,
                max_ticket_limit: 5,
                ticket_cost: vec![Coin {
                    denom: NATIVE_DENOM.to_string(),
                    amount: Uint128::new(1000).into(),
                }],
                event_segment_access: EventSegmentAccessType::SingleSegment { id: 0u64 },
                total_ticket_limit: 10,
            },
            GuestDetails {
                guest_type: "General".to_string(),
                guest_weight: 50,
                max_ticket_limit: 10,
                ticket_cost: vec![Coin {
                    denom: NATIVE_DENOM.to_string(),
                    amount: Uint128::new(500).into(),
                }],
                total_ticket_limit: 10,
                event_segment_access: EventSegmentAccessType::SingleSegment { id: 1u64 },
            },
        ],
        cw420: cw420_code_id,
        zkid: 0,
        event_timeline: vec![
            EventSegment {
                stage_description: "Opening".to_string(),
                start: Timestamp::from_seconds(1000),
                end: Timestamp::from_seconds(2000),
            },
            EventSegment {
                stage_description: "Main Event".to_string(),
                start: Timestamp::from_seconds(2000),
                end: Timestamp::from_seconds(3000),
            },
        ],
    }
}

#[test]
fn canonical_addr() {
    let canon = CanonicalAddr::from(LICENSE_CANONICAL_ADDR.as_bytes());
    let btsg = mock_dependencies().api.with_prefix("btsg");
    let cosmos = mock_dependencies().api.with_prefix("cosmos");
    let juno = mock_dependencies().api.with_prefix("juno");

    let btsg_hra = btsg.addr_humanize(&canon).unwrap();
    let cosmos_hra = cosmos.addr_humanize(&canon).unwrap();
    let juno_hra = juno.addr_humanize(&canon).unwrap();
    println!("{:#?}", btsg_hra.to_string());
    println!("{:#?}", cosmos_hra.to_string());
    println!("{:#?}", juno_hra.to_string());

    let btsg_can = btsg.addr_canonicalize(btsg_hra.as_str()).unwrap();
    let cosmos_can = cosmos.addr_canonicalize(cosmos_hra.as_str()).unwrap();

    // cosmos addr we are using to collectlicense fee
    let license_addr = "cosmos1tzz4sp3y8l5lf76qy0ydzjwlntcu8zg7agj6am";
    let license_can = cosmos.addr_canonicalize(license_addr).unwrap();
    println!("{:#?}", license_can.to_string());

    assert_eq!(btsg_can, cosmos_can)
}

#[test]
fn test_instantiate_factory() {
    let mut app = setup_app();
    let factory_code_id = app.store_code(factory_contract());
    let cw_ave_code_id = app.store_code(cw_ave_contract());

    // Test successful instantiation
    let instantiate_msg = InstantiateMsg {
        owner: Some(ALICE.to_string()),
        cw_ave_id: cw_ave_code_id,
    };

    let factory_addr = app
        .instantiate_contract(
            factory_code_id,
            Addr::unchecked("creator"),
            &instantiate_msg,
            &[],
            "cw-ave-factory",
            None,
        )
        .unwrap();

    // Query ownership
    let ownership: cw_ownable::Ownership<Addr> = app
        .wrap()
        .query_wasm_smart(factory_addr.clone(), &QueryMsg::Ownership {})
        .unwrap();
    assert_eq!(ownership.owner, Some(Addr::unchecked(ALICE)));

    // Query code ID
    let code_id: u64 = app
        .wrap()
        .query_wasm_smart(factory_addr, &QueryMsg::CodeId {})
        .unwrap();
    assert_eq!(code_id, cw_ave_code_id);
}

#[test]
fn test_instantiate_factory_no_owner() {
    let mut app = setup_app();
    let factory_code_id = app.store_code(factory_contract());
    let cw_ave_code_id = app.store_code(cw_ave_contract());

    // Test instantiation without owner
    let instantiate_msg = InstantiateMsg {
        owner: None,
        cw_ave_id: cw_ave_code_id,
    };

    let factory_addr = app
        .instantiate_contract(
            factory_code_id,
            Addr::unchecked("creator"),
            &instantiate_msg,
            &[],
            "cw-ave-factory",
            None,
        )
        .unwrap();

    // Query ownership - should be None
    let ownership: cw_ownable::Ownership<Addr> = app
        .wrap()
        .query_wasm_smart(factory_addr, &QueryMsg::Ownership {})
        .unwrap();
    assert_eq!(ownership.owner, None);
}

#[test]
fn test_create_native_av_event_contract() {
    let mut app = setup_app();
    let factory_code_id = app.store_code(factory_contract());
    let cw_ave_code_id = app.store_code(cw_ave_contract());
    let cw420_code_id = app.store_code(cw420_contract());

    // Instantiate factory
    let instantiate_msg = InstantiateMsg {
        owner: Some(ALICE.to_string()),
        cw_ave_id: cw_ave_code_id,
    };

    let factory_addr = app
        .instantiate_contract(
            factory_code_id,
            Addr::unchecked("creator"),
            &instantiate_msg,
            &[],
            "cw-ave-factory",
            None,
        )
        .unwrap();

    // Create AV event contract
    let ave_instantiate_msg = create_valid_ave_instantiate_msg(cw420_code_id);
    let create_msg = ExecuteMsg::CreateNativeAvEventContract {
        instantiate_msg: ave_instantiate_msg,
        label: "Test Event".to_string(),
    };

    let res = app
        .execute_contract(
            Addr::unchecked(ALICE),
            factory_addr.clone(),
            &create_msg,
            &coins(1000, NATIVE_DENOM),
        )
        .unwrap();

    // Verify the contract was created
    assert!(res.events.iter().any(|e| e.ty == "instantiate"));

    // Query list of contracts
    let contracts: Vec<AvEventContract> = app
        .wrap()
        .query_wasm_smart(
            factory_addr.clone(),
            &QueryMsg::ListAvEventContracts {
                start_after: None,
                limit: None,
            },
        )
        .unwrap();
    assert_eq!(contracts.len(), 1);
    assert_eq!(contracts[0].instantiator, ALICE.to_string());

    // Query by instantiator
    let contracts_by_instantiator: Vec<AvEventContract> = app
        .wrap()
        .query_wasm_smart(
            factory_addr,
            &QueryMsg::ListAvEventContractsByInstantiator {
                instantiator: ALICE.to_string(),
                start_after: None,
                limit: None,
            },
        )
        .unwrap();
    assert_eq!(contracts_by_instantiator.len(), 1);
    assert_eq!(contracts_by_instantiator[0].instantiator, ALICE.to_string());
}

#[test]
fn test_unauthorized_create_contract() {
    let mut app = setup_app();
    let factory_code_id = app.store_code(factory_contract());
    let cw_ave_code_id = app.store_code(cw_ave_contract());
    let cw420_code_id = app.store_code(cw420_contract());

    // Instantiate factory with Alice as owner
    let instantiate_msg = InstantiateMsg {
        owner: Some(ALICE.to_string()),
        cw_ave_id: cw_ave_code_id,
    };

    let factory_addr = app
        .instantiate_contract(
            factory_code_id,
            Addr::unchecked("creator"),
            &instantiate_msg,
            &[],
            "cw-ave-factory",
            None,
        )
        .unwrap();

    // Try to create contract as Bob (not owner)
    let ave_instantiate_msg = create_valid_ave_instantiate_msg(cw420_code_id);
    let create_msg = ExecuteMsg::CreateNativeAvEventContract {
        instantiate_msg: ave_instantiate_msg,
        label: "Test Event".to_string(),
    };

    let err = app
        .execute_contract(
            Addr::unchecked(BOB),
            factory_addr,
            &create_msg,
            &coins(1000, NATIVE_DENOM),
        )
        .unwrap_err();

    assert!(
        err.to_string()
            .contains(&ContractError::Unauthorized {}.to_string())
    );
}

#[test]
fn test_update_code_id() {
    let mut app = setup_app();
    let factory_code_id = app.store_code(factory_contract());
    let cw_ave_code_id = app.store_code(cw_ave_contract());
    let new_cw_ave_code_id = app.store_code(cw_ave_contract());

    // Instantiate factory
    let instantiate_msg = InstantiateMsg {
        owner: Some(ALICE.to_string()),
        cw_ave_id: cw_ave_code_id,
    };

    let factory_addr = app
        .instantiate_contract(
            factory_code_id,
            Addr::unchecked("creator"),
            &instantiate_msg,
            &[],
            "cw-ave-factory",
            None,
        )
        .unwrap();

    // Update code ID as owner
    let update_msg = ExecuteMsg::UpdateCodeId {
        cw_ave_code_id: new_cw_ave_code_id,
    };

    app.execute_contract(
        Addr::unchecked(ALICE),
        factory_addr.clone(),
        &update_msg,
        &[],
    )
    .unwrap();

    // Verify code ID was updated
    let code_id: u64 = app
        .wrap()
        .query_wasm_smart(factory_addr, &QueryMsg::CodeId {})
        .unwrap();
    assert_eq!(code_id, new_cw_ave_code_id);
}

#[test]
fn test_unauthorized_update_code_id() {
    let mut app = setup_app();
    let factory_code_id = app.store_code(factory_contract());
    let cw_ave_code_id = app.store_code(cw_ave_contract());
    let new_cw_ave_code_id = app.store_code(cw_ave_contract());

    // Instantiate factory
    let instantiate_msg = InstantiateMsg {
        owner: Some(ALICE.to_string()),
        cw_ave_id: cw_ave_code_id,
    };

    let factory_addr = app
        .instantiate_contract(
            factory_code_id,
            Addr::unchecked("creator"),
            &instantiate_msg,
            &[],
            "cw-ave-factory",
            None,
        )
        .unwrap();

    // Try to update code ID as non-owner
    let update_msg = ExecuteMsg::UpdateCodeId {
        cw_ave_code_id: new_cw_ave_code_id,
    };

    let err = app
        .execute_contract(Addr::unchecked(BOB), factory_addr, &update_msg, &[])
        .unwrap_err();

    assert!(err.to_string().contains(
        &ContractError::Ownable(OwnershipError::NotOwner).to_string()
    ));
}

#[test]
fn test_list_contracts_pagination() {
    let mut app = setup_app();
    let factory_code_id = app.store_code(factory_contract());
    let cw_ave_code_id = app.store_code(cw_ave_contract());
    let cw420_code_id = app.store_code(cw420_contract());

    // Instantiate factory
    let instantiate_msg = InstantiateMsg {
        owner: None, // No owner restriction
        cw_ave_id: cw_ave_code_id,
    };

    let factory_addr = app
        .instantiate_contract(
            factory_code_id,
            Addr::unchecked("creator"),
            &instantiate_msg,
            &[],
            "cw-ave-factory",
            None,
        )
        .unwrap();

    // Create multiple contracts
    for i in 0..5 {
        let ave_instantiate_msg = AvEventInstantiateMsg {
            title: format!("Test Event {}", i),
            description: format!("A test event {}", i),
            event_curator: ALICE.to_string(),
            usher_admins: vec![Member {
                addr: ALICE.to_string(),
                weight: 1,
            }],
            guest_details: vec![GuestDetails {
                guest_type: "General".to_string(),
                guest_weight: 50,
                max_ticket_limit: 10,
                ticket_cost: vec![Coin {
                    denom: NATIVE_DENOM.to_string(),
                    amount: Uint128::new(500).into(),
                }],
                total_ticket_limit: 10,
                event_segment_access: EventSegmentAccessType::SingleSegment { id: 0u64 },
            }],
            cw420: cw420_code_id,
            zkid: 0,
            event_timeline: vec![EventSegment {
                stage_description: "Main Event".to_string(),
                start: Timestamp::from_seconds(1000),
                end: Timestamp::from_seconds(2000),
            }],
        };

        let create_msg = ExecuteMsg::CreateNativeAvEventContract {
            instantiate_msg: ave_instantiate_msg,
            label: format!("Test Event {}", i),
        };

        app.execute_contract(
            Addr::unchecked(ALICE),
            factory_addr.clone(),
            &create_msg,
            &coins(1000, NATIVE_DENOM),
        )
        .unwrap();
    }

    // Test pagination
    let contracts: Vec<AvEventContract> = app
        .wrap()
        .query_wasm_smart(
            factory_addr.clone(),
            &QueryMsg::ListAvEventContracts {
                start_after: None,
                limit: Some(3),
            },
        )
        .unwrap();
    assert_eq!(contracts.len(), 3);

    // Test reverse pagination
    let contracts_reverse: Vec<AvEventContract> = app
        .wrap()
        .query_wasm_smart(
            factory_addr,
            &QueryMsg::ListAvEventContractsReverse {
                start_before: None,
                limit: Some(2),
            },
        )
        .unwrap();
    assert_eq!(contracts_reverse.len(), 2);
}

#[test]
fn test_query_by_instantiator() {
    let mut app = setup_app();
    let factory_code_id = app.store_code(factory_contract());
    let cw_ave_code_id = app.store_code(cw_ave_contract());
    let cw420_code_id = app.store_code(cw420_contract());

    // Instantiate factory
    let instantiate_msg = InstantiateMsg {
        owner: None,
        cw_ave_id: cw_ave_code_id,
    };

    let factory_addr = app
        .instantiate_contract(
            factory_code_id,
            Addr::unchecked("creator"),
            &instantiate_msg,
            &[],
            "cw-ave-factory",
            None,
        )
        .unwrap();

    // Create contracts from different instantiators
    let ave_instantiate_msg = create_valid_ave_instantiate_msg(cw420_code_id);
    let create_msg = ExecuteMsg::CreateNativeAvEventContract {
        instantiate_msg: ave_instantiate_msg.clone(),
        label: "Alice Event".to_string(),
    };

    app.execute_contract(
        Addr::unchecked(ALICE),
        factory_addr.clone(),
        &create_msg,
        &coins(1000, NATIVE_DENOM),
    )
    .unwrap();

    let create_msg_bob = ExecuteMsg::CreateNativeAvEventContract {
        instantiate_msg: ave_instantiate_msg,
        label: "Bob Event".to_string(),
    };

    app.execute_contract(
        Addr::unchecked(BOB),
        factory_addr.clone(),
        &create_msg_bob,
        &coins(1000, NATIVE_DENOM),
    )
    .unwrap();

    // Query Alice's contracts
    let alice_contracts: Vec<AvEventContract> = app
        .wrap()
        .query_wasm_smart(
            factory_addr.clone(),
            &QueryMsg::ListAvEventContractsByInstantiator {
                instantiator: ALICE.to_string(),
                start_after: None,
                limit: None,
            },
        )
        .unwrap();
    assert_eq!(alice_contracts.len(), 1);
    assert_eq!(alice_contracts[0].instantiator, ALICE.to_string());

    // Query Bob's contracts
    let bob_contracts: Vec<AvEventContract> = app
        .wrap()
        .query_wasm_smart(
            factory_addr.clone(),
            &QueryMsg::ListAvEventContractsByInstantiator {
                instantiator: BOB.to_string(),
                start_after: None,
                limit: None,
            },
        )
        .unwrap();
    assert_eq!(bob_contracts.len(), 1);
    assert_eq!(bob_contracts[0].instantiator, BOB.to_string());

    // Query Charlie's contracts (should be empty)
    let charlie_contracts: Vec<AvEventContract> = app
        .wrap()
        .query_wasm_smart(
            factory_addr,
            &QueryMsg::ListAvEventContractsByInstantiator {
                instantiator: CHARLIE.to_string(),
                start_after: None,
                limit: None,
            },
        )
        .unwrap();
    assert_eq!(charlie_contracts.len(), 0);
}

#[test]
fn test_query_by_instantiator_reverse() {
    let mut app = setup_app();
    let factory_code_id = app.store_code(factory_contract());
    let cw_ave_code_id = app.store_code(cw_ave_contract());
    let cw420_code_id = app.store_code(cw420_contract());

    // Instantiate factory
    let instantiate_msg = InstantiateMsg {
        owner: None,
        cw_ave_id: cw_ave_code_id,
    };

    let factory_addr = app
        .instantiate_contract(
            factory_code_id,
            Addr::unchecked("creator"),
            &instantiate_msg,
            &[],
            "cw-ave-factory",
            None,
        )
        .unwrap();

    // Create multiple contracts from Alice
    for i in 0..3 {
        let ave_instantiate_msg = AvEventInstantiateMsg {
            title: format!("Alice Event {}", i),
            description: format!("Alice's event {}", i),
            event_curator: ALICE.to_string(),
            usher_admins: vec![Member {
                addr: ALICE.to_string(),
                weight: 1,
            }],
            guest_details: vec![GuestDetails {
                guest_type: "General".to_string(),
                guest_weight: 50,
                max_ticket_limit: 10,
                ticket_cost: vec![Coin {
                    denom: NATIVE_DENOM.to_string(),
                    amount: Uint128::new(500).into(),
                }],
                total_ticket_limit: 10,
                event_segment_access: EventSegmentAccessType::SingleSegment { id: 0u64 },
            }],
            cw420: cw420_code_id,
            zkid: 0,
            event_timeline: vec![EventSegment {
                stage_description: "Main Event".to_string(),
                start: Timestamp::from_seconds(1000),
                end: Timestamp::from_seconds(2000),
            }],
        };

        let create_msg = ExecuteMsg::CreateNativeAvEventContract {
            instantiate_msg: ave_instantiate_msg,
            label: format!("Alice Event {}", i),
        };

        app.execute_contract(
            Addr::unchecked(ALICE),
            factory_addr.clone(),
            &create_msg,
            &coins(1000, NATIVE_DENOM),
        )
        .unwrap();
    }

    // Query reverse order
    let contracts_reverse: Vec<AvEventContract> = app
        .wrap()
        .query_wasm_smart(
            factory_addr,
            &QueryMsg::ListAvEventContractsByInstantiatorReverse {
                instantiator: ALICE.to_string(),
                start_before: None,
                limit: Some(2),
            },
        )
        .unwrap();
    assert_eq!(contracts_reverse.len(), 2);
    assert_eq!(contracts_reverse[0].instantiator, ALICE.to_string());
}

#[test]
fn test_ownership_transfer() {
    let mut app = setup_app();
    let factory_code_id = app.store_code(factory_contract());
    let cw_ave_code_id = app.store_code(cw_ave_contract());

    // Instantiate factory
    let instantiate_msg = InstantiateMsg {
        owner: Some(ALICE.to_string()),
        cw_ave_id: cw_ave_code_id,
    };

    let factory_addr = app
        .instantiate_contract(
            factory_code_id,
            Addr::unchecked("creator"),
            &instantiate_msg,
            &[],
            "cw-ave-factory",
            None,
        )
        .unwrap();

    // Transfer ownership to Bob
    let transfer_msg = ExecuteMsg::UpdateOwnership(cw_ownable::Action::TransferOwnership {
        new_owner: BOB.to_string(),
        expiry: None,
    });

    app.execute_contract(
        Addr::unchecked(ALICE),
        factory_addr.clone(),
        &transfer_msg,
        &[],
    )
    .unwrap();

    // Accept ownership as Bob
    let accept_msg = ExecuteMsg::UpdateOwnership(cw_ownable::Action::AcceptOwnership {});

    app.execute_contract(Addr::unchecked(BOB), factory_addr.clone(), &accept_msg, &[])
        .unwrap();

    // Verify Bob is now the owner
    let ownership: cw_ownable::Ownership<Addr> = app
        .wrap()
        .query_wasm_smart(factory_addr, &QueryMsg::Ownership {})
        .unwrap();
    assert_eq!(ownership.owner, Some(Addr::unchecked(BOB)));
}

#[test]
fn test_reply_handler() {
    let mut app = setup_app();
    let factory_code_id = app.store_code(factory_contract());
    let cw_ave_code_id = app.store_code(cw_ave_contract());
    let cw420_code_id = app.store_code(cw420_contract());

    // Instantiate factory
    let instantiate_msg = InstantiateMsg {
        owner: Some(ALICE.to_string()),
        cw_ave_id: cw_ave_code_id,
    };

    let factory_addr = app
        .instantiate_contract(
            factory_code_id,
            Addr::unchecked("creator"),
            &instantiate_msg,
            &[],
            "cw-ave-factory",
            None,
        )
        .unwrap();

    // Create AV event contract to trigger reply
    let ave_instantiate_msg = create_valid_ave_instantiate_msg(cw420_code_id);
    let create_msg = ExecuteMsg::CreateNativeAvEventContract {
        instantiate_msg: ave_instantiate_msg,
        label: "Test Event".to_string(),
    };

    let res = app
        .execute_contract(
            Addr::unchecked(ALICE),
            factory_addr.clone(),
            &create_msg,
            &coins(1000, NATIVE_DENOM),
        )
        .unwrap();

    // Verify reply was processed correctly
    let instantiate_event = res.events.iter().find(|e| e.ty == "instantiate").unwrap();
    let contract_address = &instantiate_event.attributes[0].value;

    // Verify contract was saved in state
    let contracts: Vec<AvEventContract> = app
        .wrap()
        .query_wasm_smart(
            factory_addr,
            &QueryMsg::ListAvEventContracts {
                start_after: None,
                limit: None,
            },
        )
        .unwrap();
    assert_eq!(contracts.len(), 1);
    assert_eq!(contracts[0].contract, *contract_address);
    assert_eq!(contracts[0].instantiator, ALICE.to_string());
}

#[test]
fn test_invalid_guest_details() {
    let mut app = setup_app();
    let factory_code_id = app.store_code(factory_contract());
    let cw_ave_code_id = app.store_code(cw_ave_contract());
    let cw420_code_id = app.store_code(cw420_contract());

    // Instantiate factory
    let instantiate_msg = InstantiateMsg {
        owner: Some(ALICE.to_string()),
        cw_ave_id: cw_ave_code_id,
    };

    let factory_addr = app
        .instantiate_contract(
            factory_code_id,
            Addr::unchecked("creator"),
            &instantiate_msg,
            &[],
            "cw-ave-factory",
            None,
        )
        .unwrap();

    // Create AV event contract with invalid timeline (overlapping dates)
    let ave_instantiate_msg = AvEventInstantiateMsg {
        title: "Test Event".to_string(),
        description: "A test event".to_string(),
        event_curator: ALICE.to_string(),
        usher_admins: vec![Member {
            addr: ALICE.to_string(),
            weight: 1,
        }],
        guest_details: vec![GuestDetails {
            guest_type: "General".to_string(),
            guest_weight: 50,
            max_ticket_limit: 10,
            total_ticket_limit: 10,
            ticket_cost: vec![Coin {
                denom: NATIVE_DENOM.to_string(),
                amount: Uint128::new(500).into(),
            }],
            event_segment_access: EventSegmentAccessType::SingleSegment { id: 0u64 },
        }],
        cw420: cw420_code_id,
        zkid: 0,
        event_timeline: vec![
            EventSegment {
                stage_description: "First Event".to_string(),
                start: Timestamp::from_seconds(1000),
                end: Timestamp::from_seconds(2000),
            },
            EventSegment {
                stage_description: "Overlapping Event".to_string(),
                start: Timestamp::from_seconds(1500), // Overlaps with first event
                end: Timestamp::from_seconds(2500),
            },
        ],
    };

    let create_msg = ExecuteMsg::CreateNativeAvEventContract {
        instantiate_msg: ave_instantiate_msg,
        label: "Test Event".to_string(),
    };

    // This should fail due to overlapping event dates
    let err = app
        .execute_contract(
            Addr::unchecked(ALICE),
            factory_addr,
            &create_msg,
            &coins(1000, NATIVE_DENOM),
        )
        .unwrap_err();
    // The error should propagate from the cw-ave contrasct instantiation
    assert!(
        err.to_string()
            .contains(&CwAveContractError::OverlappingEventDates {}.to_string())
    );
}
