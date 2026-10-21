//! Loan repay and raffle draw against each lab SVG collection.
#![cfg(feature = "orch-suite")]

use cosmwasm_std::{Addr, Binary, Uint256, coin, coins};
use cw_orch::prelude::*;
use cw721::msg::OwnerOfResponse;
use cw721_svg::{ExecuteMsg as SvgExec, QueryMsg as SvgQuery};
use nftfi_loan::msg::{CollateralResponse, ExecuteMsg as LoanMsg, QueryMsg as LoanQuery};
use nftfi_loan::state::{LoanState, LoanTerms};
use nftfi_raffle::msg::{
    ExecuteMsg as RaffleMsg, QueryMsg as RaffleQuery, RaffleOptionsMsg, TicketResponse,
};
use nftfi_randomness::{RandomnessSourceMsg, select_tickets};
use nftfi_utils::AssetInfo;
use nftfi_utils::svg::{self, SvgDrop};
use terp_orch::{NftfiDeployData, NftfiSuite};

const DENOM: &str = "uterp";

fn fund(chain: &MockBech32, addr: &Addr, amount: u128) -> anyhow::Result<()> {
    chain.add_balance(addr, coins(amount, DENOM))?;
    Ok(())
}

fn bal(chain: &MockBech32, addr: &Addr) -> anyhow::Result<Uint256> {
    Ok(chain.balance(addr, Some(DENOM.to_string()))?[0].amount)
}

fn owner_of(suite: &NftfiSuite<MockBech32>, token_id: &str) -> anyhow::Result<String> {
    let resp: OwnerOfResponse = suite.collection.query(&SvgQuery::OwnerOf {
        token_id: token_id.to_string(),
        include_expired: None,
    })?;
    Ok(resp.owner)
}

fn mint(chain: &MockBech32, suite: &NftfiSuite<MockBech32>, to: &Addr) -> anyhow::Result<()> {
    chain
        .call_as(to)
        .execute(&svg::mint_msg(1), &[], &suite.collection.address()?)?;
    Ok(())
}

#[test]
fn loan_and_raffle_on_each_svg_collection() -> anyhow::Result<()> {
    for drop in svg::lab() {
        loan_repay(&drop)?;
        raffle_draw(&drop)?;
    }
    Ok(())
}

fn loan_repay(drop: &SvgDrop) -> anyhow::Result<()> {
    let chain = MockBech32::new("terp");
    let admin = chain.sender_addr();
    let borrower = chain.addr_make("borrower");
    let lender = chain.addr_make("lender");
    let treasury = chain.addr_make("treasury");
    fund(&chain, &borrower, 5_000)?;
    fund(&chain, &lender, 5_000)?;
    let mut data = NftfiDeployData::lab(admin, drop.clone());
    data.treasury = treasury.to_string();
    data.creation_coins = None;
    let suite = NftfiSuite::deploy_on(chain.clone(), data)?;

    mint(&chain, &suite, &borrower)?;
    let token = "0";
    chain.call_as(&borrower).execute(
        &SvgExec::Approve {
            spender: suite.loan.address()?.to_string(),
            token_id: token.into(),
            expires: None,
        },
        &[],
        &suite.collection.address()?,
    )?;
    chain.call_as(&borrower).execute(
        &LoanMsg::ListCollaterals {
            tokens: vec![AssetInfo::cw721(
                suite.collection.address()?.as_str(),
                token,
            )],
            terms: None,
            comment: None,
        },
        &coins(50, DENOM),
        &suite.loan.address()?,
    )?;
    let terms = LoanTerms {
        principle: coin(1_000, DENOM),
        interest: Uint256::from(100u128),
        duration_in_blocks: 10,
    };
    chain.call_as(&lender).execute(
        &LoanMsg::MakeOffer {
            borrower: borrower.to_string(),
            loan_id: 0,
            terms,
            comment: None,
            on_behalf_of: None,
        },
        &coins(1_000, DENOM),
        &suite.loan.address()?,
    )?;
    chain.call_as(&borrower).execute(
        &LoanMsg::AcceptOffer {
            global_offer_id: "1".into(),
        },
        &[],
        &suite.loan.address()?,
    )?;
    assert_eq!(owner_of(&suite, token)?, suite.loan.address()?.to_string());
    chain.call_as(&borrower).execute(
        &LoanMsg::Repay { loan_id: 0 },
        &coins(1_100, DENOM),
        &suite.loan.address()?,
    )?;
    assert_eq!(owner_of(&suite, token)?, borrower.to_string());
    assert_eq!(bal(&chain, &borrower)?, Uint256::from(4_850u128));
    assert_eq!(bal(&chain, &lender)?, Uint256::from(5_090u128));
    assert_eq!(bal(&chain, &treasury)?, Uint256::from(60u128));
    let row: CollateralResponse = suite.loan.query(&LoanQuery::Collateral {
        borrower: borrower.to_string(),
        loan_id: 0,
    })?;
    assert!(matches!(row.state, LoanState::Ended));
    Ok(())
}

fn raffle_draw(drop: &SvgDrop) -> anyhow::Result<()> {
    let chain = MockBech32::new("terp");
    let admin = chain.sender_addr();
    let alice = chain.addr_make("alice");
    let bob = chain.addr_make("bob");
    let beacon = chain.addr_make("beacon");
    let fee = chain.addr_make("fee");
    fund(&chain, &admin, 1_000)?;
    fund(&chain, &alice, 500)?;
    fund(&chain, &bob, 500)?;
    let mut data = NftfiDeployData::lab(admin.clone(), drop.clone());
    data.fee_addr = fee.to_string();
    data.listing_fee_coins = None;
    let suite = NftfiSuite::deploy_on(chain.clone(), data)?;
    chain.call_as(&admin).execute(
        &RaffleMsg::UpdateConfig {
            owner: None,
            fee_addr: None,
            minimum_raffle_duration: None,
            max_tickets_per_raffle: None,
            raffle_fee: None,
            creation_coins: None,
            randomness: Some(RandomnessSourceMsg::Seed {
                provider: Some(beacon.to_string()),
            }),
        },
        &[],
        &suite.raffle.address()?,
    )?;

    mint(&chain, &suite, &admin)?;
    let token = "0";
    chain.call_as(&admin).execute(
        &SvgExec::Approve {
            spender: suite.raffle.address()?.to_string(),
            token_id: token.into(),
            expires: None,
        },
        &[],
        &suite.collection.address()?,
    )?;
    chain.call_as(&admin).execute(
        &RaffleMsg::CreateRaffle {
            owner: None,
            assets: vec![AssetInfo::cw721(
                suite.collection.address()?.as_str(),
                token,
            )],
            raffle_options: RaffleOptionsMsg {
                start: None,
                duration: 100,
                comment: Some("pot".into()),
                max_ticket_number: Some(10),
                max_ticket_per_address: Some(4),
                min_ticket_number: None,
                one_winner_per_asset: false,
            },
            raffle_ticket_price: AssetInfo::coin(10, DENOM),
        },
        &coins(100, DENOM),
        &suite.raffle.address()?,
    )?;
    assert_eq!(
        owner_of(&suite, token)?,
        suite.raffle.address()?.to_string()
    );
    for buyer in [&alice, &bob] {
        chain.call_as(buyer).execute(
            &RaffleMsg::BuyTicket {
                raffle_id: 0,
                ticket_count: 1,
                on_behalf_of: None,
            },
            &coins(10, DENOM),
            &suite.raffle.address()?,
        )?;
    }
    chain.wait_seconds(101)?;
    let seed_bytes = [9u8; 32];
    let seed = Binary::from(seed_bytes);
    chain.call_as(&beacon).execute(
        &RaffleMsg::Draw {
            raffle_id: 0,
            seed: seed.clone(),
        },
        &[],
        &suite.raffle.address()?,
    )?;
    chain.call_as(&alice).execute(
        &RaffleMsg::ClaimRaffle { raffle_id: 0 },
        &[],
        &suite.raffle.address()?,
    )?;
    let idx = select_tickets(&seed_bytes, 0, 2, 1)[0];
    let ticket: TicketResponse = suite.raffle.query(&RaffleQuery::Ticket {
        raffle_id: 0,
        index: idx,
    })?;
    assert_eq!(owner_of(&suite, token)?, ticket.owner);
    assert_eq!(bal(&chain, &admin)?, Uint256::from(918u128));
    assert_eq!(bal(&chain, &fee)?, Uint256::from(102u128));
    Ok(())
}
