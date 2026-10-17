use cosmwasm_std::{Addr, Empty, Uint128, Uint256, coins};
use cw_multi_test::{App, BankSudo, Contract, ContractWrapper, Executor, SudoMsg};
use cw_ownable::OwnershipError;
use cw_shit_denom::{PossibleShit, UncheckedDenom};
use cw_shitstrap::contract::msg::InstantiateMsg as ShitstrapInstantiateMsg;
use cw20::Cw20Coin;

use crate::{
    ContractError,
    msg::{ExecuteMsg, InstantiateMsg, QueryMsg},
    state::ShitstrapContract,
};

const ALICE: &str = "alice";
const BOB: &str = "bob";
const CREATOR: &str = "creator";
const INITIAL_BALANCE: u128 = 1000000000;
const NATIVE_DENOM: &str = "denom";

fn factory_contract() -> Box<dyn Contract<Empty>> {
    let contract = ContractWrapper::new(
        crate::contract::execute,
        crate::contract::instantiate,
        crate::contract::query,
    )
    .with_reply(crate::contract::reply);
    Box::new(contract)
}

fn cw20_contract() -> Box<dyn Contract<Empty>> {
    let contract = ContractWrapper::new(
        cw20_base::contract::execute,
        cw20_base::contract::instantiate,
        cw20_base::contract::query,
    );
    Box::new(contract)
}

pub fn cw_vesting_contract() -> Box<dyn Contract<Empty>> {
    let contract = ContractWrapper::new(
        cw_shitstrap::contract::execute,
        cw_shitstrap::contract::instantiate,
        cw_shitstrap::contract::query,
    );
    Box::new(contract)
}

#[test]
pub fn test_instantiate_native_payroll_contract() {
    let mut app = App::default();
    let code_id = app.store_code(factory_contract());
    let shitstrap_code_id = app.store_code(cw_vesting_contract());
    let a = app.api().addr_make(ALICE);
    let b = app.api().addr_make(BOB);
    let c: Addr = app.api().addr_make(CREATOR);
    // Instantiate factory with only Alice allowed to instantiate payroll contracts
    let instantiate = InstantiateMsg {
        owner: Some(a.to_string()),
        shitstrap_id: shitstrap_code_id,
    };
    let factory_addr = app
        .instantiate_contract(code_id, c, &instantiate, &[], "cw-admin-factory", None)
        .unwrap();

    // Mint alice and bob native tokens
    app.sudo(SudoMsg::Bank({
        BankSudo::Mint {
            to_address: a.to_string(),
            amount: coins(INITIAL_BALANCE, NATIVE_DENOM),
        }
    }))
    .unwrap();
    app.sudo(SudoMsg::Bank({
        BankSudo::Mint {
            to_address: b.to_string(),
            amount: coins(INITIAL_BALANCE, NATIVE_DENOM),
        }
    }))
    .unwrap();

    let amount = Uint128::new(1000000);
    let unchecked_denom = UncheckedDenom::Native(NATIVE_DENOM.to_string());

    let instantiate_payroll_msg = ExecuteMsg::CreateNativeShitStrapContract {
        instantiate_msg: ShitstrapInstantiateMsg {
            owner: Some(a.to_string()),
            title: "title".to_string(),
            description: "desc".to_string(),
            accepted: vec![PossibleShit {
                token: UncheckedDenom::Native("ubtsg".into()),
                shit_rate: Uint128::new(1000000),
            }],
            cutoff: Uint128::new(1000000),
            shitmos: UncheckedDenom::Native("ubtsg".into()),
            daos: Vec::new(),
        },
        label: "Payroll".to_string(),
    };

    let res = app
        .execute_contract(
            a.clone(),
            factory_addr.clone(),
            &instantiate_payroll_msg,
            &coins(amount.into(), NATIVE_DENOM),
        )
        .unwrap();

    // BOB can't instantiate as owner is configured
    let err = app
        .execute_contract(
            b.clone(),
            factory_addr.clone(),
            &instantiate_payroll_msg,
            &coins(amount.into(), NATIVE_DENOM),
        )
        .unwrap_err();
    assert!(
        err.to_string()
            .contains(&ContractError::Unauthorized {}.to_string())
    );

    // Get the payroll address from the instantiate event
    let instantiate_event = &res.events[2];
    assert_eq!(instantiate_event.ty, "instantiate");
    let cw_vesting_addr = instantiate_event.attributes[0].value.clone();

    // Check that admin of contract is owner specified in Instantiation Message
    let contract_info = app
        .wrap()
        .query_wasm_contract_info(cw_vesting_addr)
        .unwrap();
    assert_eq!(contract_info.admin, Some(a.clone()));

    // Test query list of contracts
    let contracts: Vec<ShitstrapContract> = app
        .wrap()
        .query_wasm_smart(
            factory_addr.clone(),
            &QueryMsg::ListShitstrapContracts {
                start_after: None,
                limit: None,
            },
        )
        .unwrap();
    assert_eq!(contracts.len(), 1);

    // Test query by instantiator
    let contracts: Vec<ShitstrapContract> = app
        .wrap()
        .query_wasm_smart(
            factory_addr.clone(),
            &QueryMsg::ListShitstrapContractsByInstantiator {
                instantiator: a.to_string(),
                start_after: None,
                limit: None,
            },
        )
        .unwrap();
    assert_eq!(contracts.len(), 1);

    // Test query by instantiator with no results
    let contracts: Vec<ShitstrapContract> = app
        .wrap()
        .query_wasm_smart(
            factory_addr.clone(),
            &QueryMsg::ListShitstrapContractsByInstantiator {
                instantiator: b.to_string(),
                start_after: None,
                limit: None,
            },
        )
        .unwrap();
    assert_eq!(contracts.len(), 0);

    // // Test query by recipient
    // let contracts: Vec<VestingContract> = app
    //     .wrap()
    //     .query_wasm_smart(
    //         factory_addr.clone(),
    //         &QueryMsg::ListVestingContractsByRecipient {
    //             recipient: BOB.to_string(),
    //             start_after: None,
    //             limit: None,
    //         },
    //     )
    //     .unwrap();
    // assert_eq!(contracts.len(), 1);

    // // Test query by recipient no results
    // let contracts: Vec<VestingContract> = app
    //     .wrap()
    //     .query_wasm_smart(
    //         factory_addr,
    //         &QueryMsg::ListVestingContractsByRecipient {
    //             recipient: ALICE.to_string(),
    //             start_after: None,
    //             limit: None,
    //         },
    //     )
    //     .unwrap();
    // assert_eq!(contracts.len(), 0);
}

#[test]
pub fn test_instantiate_cw20_payroll_contract() {
    let mut app = App::default();
    let a = app.api().addr_make(ALICE);
    let b = app.api().addr_make(BOB);
    let c: Addr = app.api().addr_make(CREATOR);
    let code_id = app.store_code(factory_contract());
    let cw20_code_id = app.store_code(cw20_contract());
    let cw_vesting_code_id = app.store_code(cw_vesting_contract());

    // Instantiate cw20 contract with balances for Alice
    let cw20_addr = app
        .instantiate_contract(
            cw20_code_id,
            a.clone(),
            &cw20_base::msg::InstantiateMsg {
                name: "cw20 token".to_string(),
                symbol: "cwtwenty".to_string(),
                decimals: 6,
                initial_balances: vec![Cw20Coin {
                    address: a.to_string(),
                    amount: Uint256::new(INITIAL_BALANCE),
                }],
                mint: None,
                marketing: None,
            },
            &[],
            "cw20-base",
            None,
        )
        .unwrap();

    let instantiate = InstantiateMsg {
        owner: Some(a.to_string()),
        shitstrap_id: cw_vesting_code_id,
    };
    let factory_addr = app
        .instantiate_contract(
            code_id,
            c.clone(),
            &instantiate,
            &[],
            "cw-admin-factory",
            None,
        )
        .unwrap();

    // Mint alice native tokens
    app.sudo(SudoMsg::Bank({
        BankSudo::Mint {
            to_address: a.to_string(),
            amount: coins(INITIAL_BALANCE, NATIVE_DENOM),
        }
    }))
    .unwrap();

    let amount = Uint128::new(1000000);
    let unchecked_denom = UncheckedDenom::Cw20(cw20_addr.to_string());

    let instantiate_payroll_msg = ShitstrapInstantiateMsg {
        owner: Some(a.to_string()),
        title: "title".to_string(),
        description: "desc".to_string(),
        accepted: vec![PossibleShit {
            token: UncheckedDenom::Native("ubtsg".into()),
            shit_rate: Uint128::new(1000000),
        }],
        cutoff: Uint128::new(1000000),
        shitmos: UncheckedDenom::Native("ubtsg".into()),
        daos: Vec::new(),
    };

    app.execute_contract(
        a.clone(),
        factory_addr.clone(),
        &ExecuteMsg::CreateNativeShitStrapContract {
            instantiate_msg: instantiate_payroll_msg.clone(),
            label: "Payroll".to_string(),
        },
        &coins(amount.into(), NATIVE_DENOM),
    )
    .unwrap();

    // // Get the payroll address from the instantiate event
    // let instantiate_event = &res.events[4];
    // assert_eq!(instantiate_event.ty, "instantiate");
    // let cw_vesting_addr = instantiate_event.attributes[0].value.clone();

    // // Check that admin of contract is owner specified in Instantiation Message
    // let contract_info = app
    //     .wrap()
    //     .query_wasm_contract_info(cw_vesting_addr.clone())
    //     .unwrap();
    // assert_eq!(contract_info.admin, Some(ALICE.to_string()));

    // Test query by instantiator
    let contracts: Vec<ShitstrapContract> = app
        .wrap()
        .query_wasm_smart(
            factory_addr,
            &QueryMsg::ListShitstrapContractsByInstantiator {
                instantiator: a.to_string(),
                start_after: None,
                limit: None,
            },
        )
        .unwrap();
    assert_eq!(contracts.len(), 1);
}

#[test]
fn test_instantiate_wrong_ownership_native() {
    let mut app = App::default();
    let a = app.api().addr_make(ALICE);
    let b = app.api().addr_make(BOB);
    let c: Addr = app.api().addr_make(CREATOR);
    let e: Addr = app.api().addr_make("ekez");
    let code_id = app.store_code(factory_contract());
    let cw_vesting_code_id = app.store_code(cw_vesting_contract());

    let amount = Uint128::new(1000000);
    let unchecked_denom = UncheckedDenom::Native(NATIVE_DENOM.to_string());

    app.sudo(SudoMsg::Bank({
        BankSudo::Mint {
            to_address: e.to_string(),
            amount: coins(amount.u128() * 2, NATIVE_DENOM),
        }
    }))
    .unwrap();
    app.sudo(SudoMsg::Bank({
        BankSudo::Mint {
            to_address: a.to_string(),
            amount: coins(amount.u128() * 2, NATIVE_DENOM),
        }
    }))
    .unwrap();

    // Alice is the owner. Contracts are only allowed if their owner
    // is alice or none and the sender is alice.
    let instantiate = InstantiateMsg {
        owner: Some(a.to_string()),
        shitstrap_id: cw_vesting_code_id,
    };
    let factory_addr = app
        .instantiate_contract(
            code_id,
            c.clone(),
            &instantiate,
            &[],
            "cw-admin-factory",
            None,
        )
        .unwrap();

    let err = app
        .execute_contract(
            e.clone(),
            factory_addr,
            &ExecuteMsg::CreateNativeShitStrapContract {
                instantiate_msg: ShitstrapInstantiateMsg {
                    owner: Some(a.to_string()),
                    title: "title".to_string(),
                    description: "desc".to_string(),
                    accepted: vec![PossibleShit {
                        token: UncheckedDenom::Native("ubtsg".into()),
                        shit_rate: Uint128::new(1000000),
                    }],
                    cutoff: Uint128::new(1000000),
                    shitmos: UncheckedDenom::Native("ubtsg".into()),
                    daos: Vec::new(),
                },
                label: "vesting".to_string(),
            },
            &coins(amount.u128(), NATIVE_DENOM),
        )
        .unwrap_err();

    // Can't instantiate if you are not the owner.
    assert!(
        err.to_string()
            .contains(&ContractError::Unauthorized {}.to_string())
    );
}

#[test]
fn test_update_vesting_code_id() {
    let mut app = App::default();
    let a = app.api().addr_make(ALICE);
    let b = app.api().addr_make(BOB);
    let c: Addr = app.api().addr_make(CREATOR);
    let e: Addr = app.api().addr_make("ekez");
    let code_id = app.store_code(factory_contract());
    let cw_vesting_code_id = app.store_code(cw_vesting_contract());
    let cw_vesting_code_two = app.store_code(cw_vesting_contract());

    // Instantiate factory with only Alice allowed to instantiate payroll contracts
    let instantiate = InstantiateMsg {
        owner: Some(a.to_string()),
        shitstrap_id: cw_vesting_code_id,
    };
    let factory_addr = app
        .instantiate_contract(code_id, c, &instantiate, &[], "cw-admin-factory", None)
        .unwrap();

    // Update the code ID to a new one.
    app.execute_contract(
        a.clone(),
        factory_addr.clone(),
        &ExecuteMsg::UpdateCodeId {
            shitstrap_code_id: cw_vesting_code_two,
        },
        &[],
    )
    .unwrap();

    let err = app
        .execute_contract(
            b.clone(),
            factory_addr.clone(),
            &ExecuteMsg::UpdateCodeId {
                shitstrap_code_id: cw_vesting_code_two,
            },
            &[],
        )
        .unwrap_err();
    assert!(
        err.to_string()
            .contains(&ContractError::Ownable(OwnershipError::NotOwner).to_string())
    );

    app.sudo(SudoMsg::Bank({
        BankSudo::Mint {
            to_address: a.to_string(),
            amount: coins(INITIAL_BALANCE, NATIVE_DENOM),
        }
    }))
    .unwrap();

    let amount = Uint128::new(1000000);
    let unchecked_denom = UncheckedDenom::Native(NATIVE_DENOM.to_string());

    let instantiate_payroll_msg = ExecuteMsg::CreateNativeShitStrapContract {
        instantiate_msg: ShitstrapInstantiateMsg {
            owner: Some(a.to_string()),
            title: "title".to_string(),
            description: "desc".to_string(),
            accepted: vec![PossibleShit {
                token: UncheckedDenom::Native("ubtsg".into()),
                shit_rate: Uint128::new(1000000),
            }],
            cutoff: Uint128::new(1000000),
            shitmos: UncheckedDenom::Native("ubtsg".into()),
            daos: Vec::new(),
        },
        label: "Payroll".to_string(),
    };

    let res = app
        .execute_contract(
            a.clone(),
            factory_addr,
            &instantiate_payroll_msg,
            &coins(amount.into(), NATIVE_DENOM),
        )
        .unwrap();

    // Check that the contract was instantiated using the new code ID.
    let instantiate_event = &res.events[2];
    assert_eq!(instantiate_event.ty, "instantiate");
    let cw_vesting_addr = instantiate_event.attributes[0].value.clone();
    let info = app
        .wrap()
        .query_wasm_contract_info(cw_vesting_addr)
        .unwrap();
    assert_eq!(info.code_id, cw_vesting_code_two);
}

// /// This test was contributed by Oak Security as part of their audit
// /// of cw-vesting. It addresses issue two, "Misconfiguring the total
// /// vested amount to be lower than the sent CW20 amount would cause a
// /// loss of funds".
// #[test]
// pub fn test_inconsistent_cw20_amount() {
//     let mut app = App::default();
//     let a = app.api().addr_make(ALICE);
//     let b = app.api().addr_make(BOB);
//     let c: Addr = app.api().addr_make(CREATOR);
//     let e: Addr = app.api().addr_make("ekez");
//     let code_id = app.store_code(factory_contract());
//     let cw20_code_id = app.store_code(cw20_contract());
//     let cw_vesting_code_id = app.store_code(cw_vesting_contract());
//     // Instantiate cw20 contract with balances for Alice
//     let cw20_addr = app
//         .instantiate_contract(
//             cw20_code_id,
//             a.clone(),
//             &cw20_base::msg::InstantiateMsg {
//                 name: "cw20 token".to_string(),
//                 symbol: "cwtwenty".to_string(),
//                 decimals: 6,
//                 initial_balances: vec![Cw20Coin {
//                     address: a.to_string(),
//                     amount: Uint256::new(INITIAL_BALANCE),
//                 }],
//                 mint: None,
//                 marketing: None,
//             },
//             &[],
//             "cw20-base",
//             None,
//         )
//         .unwrap();
//     let instantiate = InstantiateMsg {
//         owner: Some(a.to_string()),
//         shitstrap_id: cw_vesting_code_id,
//     };
//     let factory_addr = app
//         .instantiate_contract(
//             code_id,
//             c.clone(),
//             &instantiate,
//             &[],
//             "cw-admin-factory",
//             None,
//         )
//         .unwrap();
//     // Mint alice native tokens
//     app.sudo(SudoMsg::Bank({
//         BankSudo::Mint {
//             to_address: a.to_string(),
//             amount: coins(INITIAL_BALANCE, NATIVE_DENOM),
//         }
//     }))
//     .unwrap();
//     let amount = Uint128::new(1000000);
//     let unchecked_denom = UncheckedDenom::Cw20(cw20_addr.to_string());
//     let instantiate_payroll_msg = ShitstrapInstantiateMsg {
//         owner: Some(a.to_string()),
//         title: "title".to_string(),
//         description: "desc".to_string(),
//         accepted: vec![PossibleShit {
//             token: UncheckedDenom::Native("ubtsg".into()),
//             shit_rate: Uint128::new(1000000),
//         }],
//         cutoff: Uint128::new(1000000),
//         shitmos: UncheckedDenom::Native("ubtsg".into()),
//         daos: Vec::new(),
//     };
//     let err: ContractError = app
//         .execute_contract(
//             Addr::unchecked(ALICE),
//             cw20_addr,
//             &Cw20ExecuteMsg::Send {
//                 contract: factory_addr.to_string(),
//                 amount,
//                 msg: to_json_binary(&ReceiveMsg::InstantiatePayrollContract {
//                     instantiate_msg: instantiate_payroll_msg,
//                     label: "Payroll".to_string(),
//                 })
//                 .unwrap(),
//             },
//             &coins(amount.into(), NATIVE_DENOM), // https://github.com/CosmWasm/cw-plus/issues/862
//         )
//         .unwrap_err()
//         .downcast()
//         .unwrap();
//     assert_eq!(
//         err,
//         ContractError::WrongFundAmount {
//             sent: amount,
//             expected: amount - Uint128::one()
//         }
//     );
// }
