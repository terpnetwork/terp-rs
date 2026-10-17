use cosmwasm_std::{coin, coins, Addr, Decimal, Empty, Uint256};
use cw721::msg::{Cw721ExecuteMsg, Cw721QueryMsg, OwnerOfResponse};
use cw_multi_test::{BasicApp, BankSudo, ContractWrapper, Executor, SudoMsg};
use nftfi_loan::msg::{CollateralResponse, ExecuteMsg, InstantiateMsg, QueryMsg};
use nftfi_loan::state::LoanState;
use nftfi_utils::mock::InstantiateMsg as NftInit;
use nftfi_utils::AssetInfo;
use nftfi_loan::state::LoanTerms;

const DENOM: &str = "uterp";

struct World {
    app: BasicApp,
    borrower: Addr,
    lender: Addr,
    treasury: Addr,
    nft: Addr,
    loan: Addr,
}

fn world(listing_fee: u128) -> World {
    let mut app = BasicApp::default();
    let borrower = app.api().addr_make("borrower");
    let lender = app.api().addr_make("lender");
    let treasury = app.api().addr_make("treasury");
    let minter = app.api().addr_make("minter");
    for addr in [&borrower, &lender, &minter] {
        app.sudo(SudoMsg::Bank(BankSudo::Mint {
            to_address: addr.to_string(),
            amount: coins(5_000, DENOM),
        }))
        .unwrap();
    }
    let nft_id = app.store_code(nftfi_utils::mock::contract());
    let nft = app
        .instantiate_contract(
            nft_id,
            minter.clone(),
            &NftInit {
                minter: minter.to_string(),
            },
            &[],
            "cw721",
            None,
        )
        .unwrap();
    app.execute_contract(
        minter,
        nft.clone(),
        &Cw721ExecuteMsg::<Empty, Empty, Empty>::Mint {
            token_id: "1".into(),
            owner: borrower.to_string(),
            token_uri: None,
            extension: Empty {},
        },
        &[],
    )
    .unwrap();
    let loan_id = app.store_code(Box::new(ContractWrapper::new_with_empty(
        nftfi_loan::execute,
        nftfi_loan::instantiate,
        nftfi_loan::query,
    )));
    let listing_fee_coins = if listing_fee == 0 {
        None
    } else {
        Some(vec![coin(listing_fee, DENOM)])
    };
    let loan = app
        .instantiate_contract(
            loan_id,
            borrower.clone(),
            &InstantiateMsg {
                name: "loans".into(),
                owner: None,
                treasury_addr: treasury.to_string(),
                fee_rate: Decimal::percent(10),
                listing_fee_coins,
            },
            &[],
            "loan",
            None,
        )
        .unwrap();
    app.execute_contract(
        borrower.clone(),
        nft.clone(),
        &Cw721ExecuteMsg::<Empty, Empty, Empty>::Approve {
            spender: loan.to_string(),
            token_id: "1".into(),
            expires: None,
        },
        &[],
    )
    .unwrap();
    World {
        app,
        borrower,
        lender,
        treasury,
        nft,
        loan,
    }
}

fn owner_of(app: &BasicApp, nft: &Addr) -> String {
    let resp: OwnerOfResponse = app
        .wrap()
        .query_wasm_smart(
            nft.to_string(),
            &Cw721QueryMsg::<Empty, Empty, Empty>::OwnerOf {
                token_id: "1".into(),
                include_expired: None,
            },
        )
        .unwrap();
    resp.owner
}

fn bal(app: &BasicApp, addr: &Addr) -> Uint256 {
    app.wrap().query_balance(addr.as_str(), DENOM).unwrap().amount
}

fn terms(principal: u128, interest: u128, blocks: u64) -> LoanTerms {
    LoanTerms {
        principle: coin(principal, DENOM),
        interest: Uint256::from(interest),
        duration_in_blocks: blocks,
    }
}

fn list(w: &mut World, ask: Option<LoanTerms>, fee: u128) {
    let funds = if fee == 0 {
        Vec::new()
    } else {
        coins(fee, DENOM)
    };
    w.app
        .execute_contract(
            w.borrower.clone(),
            w.loan.clone(),
            &ExecuteMsg::ListCollaterals {
                tokens: vec![AssetInfo::cw721(w.nft.as_str(), "1")],
                terms: ask,
                comment: None,
            },
            &funds,
        )
        .unwrap();
}

#[test]
fn offer_accept_repay() {
    let mut w = world(50);
    list(&mut w, None, 50);
    assert_eq!(owner_of(&w.app, &w.nft), w.borrower.to_string());
    w.app
        .execute_contract(
            w.lender.clone(),
            w.loan.clone(),
            &ExecuteMsg::MakeOffer {
                borrower: w.borrower.to_string(),
                loan_id: 0,
                terms: terms(1_000, 100, 10),
                comment: None,
                on_behalf_of: None,
            },
            &coins(1_000, DENOM),
        )
        .unwrap();
    w.app
        .execute_contract(
            w.borrower.clone(),
            w.loan.clone(),
            &ExecuteMsg::AcceptOffer {
                global_offer_id: "1".into(),
            },
            &[],
        )
        .unwrap();
    assert_eq!(owner_of(&w.app, &w.nft), w.loan.to_string());
    // 5000 - 50 listing + 1000 principal.
    assert_eq!(bal(&w.app, &w.borrower), Uint256::from(5_950u128));
    w.app
        .execute_contract(
            w.borrower.clone(),
            w.loan.clone(),
            &ExecuteMsg::Repay { loan_id: 0 },
            &coins(1_100, DENOM),
        )
        .unwrap();
    assert_eq!(owner_of(&w.app, &w.nft), w.borrower.to_string());
    // 5950 - 1100 repayment.
    assert_eq!(bal(&w.app, &w.borrower), Uint256::from(4_850u128));
    // 5000 - 1000 principal + 1090 (principal + interest - 10% fee).
    assert_eq!(bal(&w.app, &w.lender), Uint256::from(5_090u128));
    // 50 listing fee + 10 interest fee.
    assert_eq!(bal(&w.app, &w.treasury), Uint256::from(60u128));
    let row: CollateralResponse = w
        .app
        .wrap()
        .query_wasm_smart(
            w.loan.to_string(),
            &QueryMsg::Collateral {
                borrower: w.borrower.to_string(),
                loan_id: 0,
            },
        )
        .unwrap();
    assert!(matches!(row.state, LoanState::Ended));
}

#[test]
fn accept_loan_then_default() {
    let mut w = world(0);
    list(&mut w, Some(terms(1_000, 100, 5)), 0);
    w.app
        .execute_contract(
            w.lender.clone(),
            w.loan.clone(),
            &ExecuteMsg::AcceptLoan {
                borrower: w.borrower.to_string(),
                loan_id: 0,
                comment: None,
            },
            &coins(1_000, DENOM),
        )
        .unwrap();
    let start = w.app.block_info().height;
    w.app.update_block(|b| {
        b.height = start + 6;
        b.time = b.time.plus_seconds(60);
    });
    let early = w.app.execute_contract(
        w.borrower.clone(),
        w.loan.clone(),
        &ExecuteMsg::Repay { loan_id: 0 },
        &coins(1_100, DENOM),
    );
    assert!(early.is_err());
    w.app
        .execute_contract(
            w.lender.clone(),
            w.loan.clone(),
            &ExecuteMsg::WithdrawDefaulted {
                borrower: w.borrower.to_string(),
                loan_id: 0,
            },
            &[],
        )
        .unwrap();
    assert_eq!(owner_of(&w.app, &w.nft), w.lender.to_string());
}

#[test]
fn withdraw_listing_and_refused_offer() {
    let mut w = world(0);
    list(&mut w, None, 0);
    w.app
        .execute_contract(
            w.borrower.clone(),
            w.loan.clone(),
            &ExecuteMsg::WithdrawCollaterals { loan_id: 0 },
            &[],
        )
        .unwrap();
    let row: CollateralResponse = w
        .app
        .wrap()
        .query_wasm_smart(
            w.loan.to_string(),
            &QueryMsg::Collateral {
                borrower: w.borrower.to_string(),
                loan_id: 0,
            },
        )
        .unwrap();
    assert!(matches!(row.state, LoanState::Inactive));
    assert_eq!(owner_of(&w.app, &w.nft), w.borrower.to_string());

    // Second token so a fresh listing can take an offer and refuse it.
    // The mock has one token. Re-list is blocked because the first loan is inactive
    // but the token is still owned and approved. Listing again is a new loan id.
    list(&mut w, None, 0);
    w.app
        .execute_contract(
            w.lender.clone(),
            w.loan.clone(),
            &ExecuteMsg::MakeOffer {
                borrower: w.borrower.to_string(),
                loan_id: 1,
                terms: terms(1_000, 50, 4),
                comment: None,
                on_behalf_of: None,
            },
            &coins(1_000, DENOM),
        )
        .unwrap();
    w.app
        .execute_contract(
            w.borrower.clone(),
            w.loan.clone(),
            &ExecuteMsg::RefuseOffer {
                global_offer_id: "1".into(),
            },
            &[],
        )
        .unwrap();
    w.app
        .execute_contract(
            w.lender.clone(),
            w.loan.clone(),
            &ExecuteMsg::WithdrawRefusedOffer {
                global_offer_id: "1".into(),
            },
            &[],
        )
        .unwrap();
    assert_eq!(bal(&w.app, &w.lender), Uint256::from(5_000u128));

    // Withdrawing a listing must not stick the lender's deposit.
    list(&mut w, None, 0);
    w.app
        .execute_contract(
            w.lender.clone(),
            w.loan.clone(),
            &ExecuteMsg::MakeOffer {
                borrower: w.borrower.to_string(),
                loan_id: 2,
                terms: terms(400, 10, 4),
                comment: None,
                on_behalf_of: None,
            },
            &coins(400, DENOM),
        )
        .unwrap();
    w.app
        .execute_contract(
            w.borrower.clone(),
            w.loan.clone(),
            &ExecuteMsg::WithdrawCollaterals { loan_id: 2 },
            &[],
        )
        .unwrap();
    w.app
        .execute_contract(
            w.lender.clone(),
            w.loan.clone(),
            &ExecuteMsg::CancelOffer {
                global_offer_id: "2".into(),
            },
            &[],
        )
        .unwrap();
    assert_eq!(bal(&w.app, &w.lender), Uint256::from(5_000u128));
}
