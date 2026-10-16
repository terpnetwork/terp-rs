//! Drive every brief action once on the mock chain and record the first success.

use std::path::PathBuf;

use anyhow::Context;
use cosmwasm_std::{Addr, Uint128, coin};
use cw_orch::prelude::*;

use crate::daemon::daemon_note;
use crate::loan::{self, BenchLoan};
use crate::marketplace::{self, BenchMarketplace};
use crate::nft::{self, BenchNft};
use crate::pay::DENOM;
use crate::raffle::{self, BenchRaffle};
use crate::table::{Row, Table};

const PRICE: u128 = 1_000;
const FEE_BPS: u16 = 200;
const ROYALTY_BPS: u16 = 500;
const TICKET: u128 = 100;
const RAFFLE_FEE_BPS: u16 = 1_000;
const PRINCIPAL: u128 = 500;
/// 200 bps of 1000, plus 500 bps of 1000. Dust stays with the seller.
const SELLER_SHARE: u128 = 930;
const FEE_SHARE: u128 = 20;
const ROYALTY_SHARE: u128 = 50;
/// 1000 bps of one 100-uterp ticket.
const RAFFLE_FEE: u128 = 10;
const RAFFLE_OWNER_SHARE: u128 = 90;

pub fn run_mock() -> anyhow::Result<Table> {
    let mut table = Table::default();
    table.notes.push(sibling_note());
    table.notes.push(daemon_note());

    let chain = MockBech32::new_with_chain_id("terp", "svgc-preview-1");
    let admin = chain.addr_make("admin");
    let seller = chain.addr_make("seller");
    let buyer = chain.addr_make("buyer");
    let bidder = chain.addr_make("bidder");
    let fee = chain.addr_make("fee");
    let royalty = chain.addr_make("royalty");
    let raffle_owner = chain.addr_make("raffle-owner");
    let ticket_buyer = chain.addr_make("ticket-buyer");
    let borrower = chain.addr_make("borrower");
    let lender = chain.addr_make("lender");

    for addr in [
        &admin,
        &seller,
        &buyer,
        &bidder,
        &raffle_owner,
        &ticket_buyer,
        &borrower,
        &lender,
    ] {
        chain
            .set_balance(addr, vec![coin(1_000_000, DENOM)])
            .context("set balance")?;
    }

    let collection = nft::deploy(&chain, &admin)?;
    let nft_addr = collection.address().context("cw721 address")?;
    for (id, to) in [("0", &seller), ("1", &seller), ("2", &seller)] {
        nft::transfer(&collection, &admin, id, to)?;
    }
    nft::transfer(&collection, &admin, "3", &raffle_owner)?;
    nft::transfer(&collection, &admin, "4", &borrower)?;
    nft::transfer(&collection, &admin, "5", &borrower)?;

    let market = BenchMarketplace::new(chain.clone());
    market
        .call_as(&admin)
        .upload()
        .context("upload marketplace")?;
    market
        .call_as(&admin)
        .instantiate(
            &marketplace::InstantiateMsg {
                fee_bps: FEE_BPS,
                royalty_bps: ROYALTY_BPS,
                fee_addr: fee.to_string(),
                royalty_addr: royalty.to_string(),
            },
            Some(&admin),
            &[],
        )
        .context("instantiate marketplace")?;
    let market_addr = market.address().context("marketplace address")?;

    let raffle = BenchRaffle::new(chain.clone());
    raffle.call_as(&admin).upload().context("upload raffle")?;
    raffle
        .call_as(&admin)
        .instantiate(
            &raffle::InstantiateMsg {
                fee_bps: RAFFLE_FEE_BPS,
                fee_addr: fee.to_string(),
                draw_seed: "bench-seed".into(),
            },
            Some(&admin),
            &[],
        )
        .context("instantiate raffle")?;
    let raffle_addr = raffle.address().context("raffle address")?;

    let loan = BenchLoan::new(chain.clone());
    loan.call_as(&admin).upload().context("upload loan")?;
    loan.call_as(&admin)
        .instantiate(&loan::InstantiateMsg {}, Some(&admin), &[])
        .context("instantiate loan")?;
    let loan_addr = loan.address().context("loan address")?;

    step(
        &mut table,
        "marketplace",
        "list",
        list_three(&market, &seller, &nft_addr),
    )?;
    step(
        &mut table,
        "marketplace",
        "bid",
        place_bid(&market, &bidder, &nft_addr),
    )?;
    step(
        &mut table,
        "marketplace",
        "buy",
        buy_listed(
            &chain,
            &collection,
            &market,
            &market_addr,
            &seller,
            &buyer,
            &fee,
            &royalty,
            &nft_addr,
        ),
    )?;
    step(
        &mut table,
        "marketplace",
        "accept",
        accept_bid(
            &chain,
            &collection,
            &market,
            &market_addr,
            &seller,
            &bidder,
            &fee,
            &royalty,
            &nft_addr,
        ),
    )?;
    step(
        &mut table,
        "marketplace",
        "cancel",
        cancel_ask(&collection, &market, &seller, &nft_addr),
    )?;

    step(
        &mut table,
        "raffle",
        "create",
        raffle_create(&collection, &raffle, &raffle_addr, &raffle_owner, &nft_addr),
    )?;
    step(
        &mut table,
        "raffle",
        "buy_ticket",
        raffle_buy(&chain, &raffle, &ticket_buyer),
    )?;
    chain.wait_seconds(60).context("wait for raffle")?;
    step(
        &mut table,
        "raffle",
        "draw",
        raffle_draw(&raffle, &ticket_buyer),
    )?;
    step(
        &mut table,
        "raffle",
        "claim",
        raffle_claim(
            &chain,
            &collection,
            &raffle,
            &raffle_owner,
            &ticket_buyer,
            &fee,
        ),
    )?;

    step(
        &mut table,
        "loan",
        "offer",
        loan_offers(&loan, &lender, &nft_addr),
    )?;
    step(
        &mut table,
        "loan",
        "accept",
        loan_accepts(&chain, &collection, &loan, &loan_addr, &borrower, &nft_addr),
    )?;
    step(
        &mut table,
        "loan",
        "repay",
        loan_repay(&chain, &collection, &loan, &borrower, &lender, &nft_addr),
    )?;
    chain.wait_seconds(31).context("wait for default")?;
    step(
        &mut table,
        "loan",
        "default",
        loan_default(&collection, &loan, &lender, &nft_addr),
    )?;

    let measured = crate::kv::measure().context("kv gas")?;
    for row in &mut table.rows {
        let gas = measured
            .iter()
            .find(|item| item.protocol == row.protocol && item.action == row.action)
            .with_context(|| format!("kv gas missing for {} {}", row.protocol, row.action))?;
        anyhow::ensure!(
            gas.gas > 0,
            "{} {} kv gas is zero",
            row.protocol,
            row.action
        );
        row.kv_gas = Some(gas.gas);
        row.detail = gas.gas.to_string();
    }

    Ok(table)
}

fn step(
    table: &mut Table,
    protocol: &str,
    action: &str,
    result: anyhow::Result<()>,
) -> anyhow::Result<()> {
    match result {
        Ok(()) => {
            table.rows.push(Row {
                protocol: protocol.into(),
                action: action.into(),
                mock_ok: true,
                detail: "kv".into(),
                kv_gas: None,
                daemon_gas: None,
            });
            Ok(())
        }
        Err(err) => {
            table.rows.push(Row {
                protocol: protocol.into(),
                action: action.into(),
                mock_ok: false,
                detail: err.to_string(),
                kv_gas: None,
                daemon_gas: None,
            });
            Err(err)
        }
    }
}

fn list_three(
    market: &BenchMarketplace<MockBech32>,
    seller: &Addr,
    nft_addr: &Addr,
) -> anyhow::Result<()> {
    for id in ["0", "1", "2"] {
        market
            .call_as(seller)
            .execute(
                &marketplace::ExecuteMsg::List {
                    collection: nft_addr.to_string(),
                    token_id: id.into(),
                    price: Uint128::new(PRICE),
                },
                &[],
            )
            .with_context(|| format!("list {id}"))?;
    }
    Ok(())
}

fn place_bid(
    market: &BenchMarketplace<MockBech32>,
    bidder: &Addr,
    nft_addr: &Addr,
) -> anyhow::Result<()> {
    market
        .call_as(bidder)
        .execute(
            &marketplace::ExecuteMsg::Bid {
                collection: nft_addr.to_string(),
                token_id: "1".into(),
                price: Uint128::new(PRICE),
            },
            &[coin(PRICE, DENOM)],
        )
        .context("bid")?;
    Ok(())
}

fn buy_listed(
    chain: &MockBech32,
    collection: &BenchNft<MockBech32>,
    market: &BenchMarketplace<MockBech32>,
    market_addr: &Addr,
    seller: &Addr,
    buyer: &Addr,
    fee: &Addr,
    royalty: &Addr,
    nft_addr: &Addr,
) -> anyhow::Result<()> {
    nft::approve(collection, seller, market_addr, "0")?;
    let seller_before = bal(chain, seller)?;
    let fee_before = bal(chain, fee)?;
    let royalty_before = bal(chain, royalty)?;
    market
        .call_as(buyer)
        .execute(
            &marketplace::ExecuteMsg::Buy {
                collection: nft_addr.to_string(),
                token_id: "0".into(),
            },
            &[coin(PRICE, DENOM)],
        )
        .context("buy")?;
    anyhow::ensure!(
        nft::owner(collection, "0")? == buyer.as_str(),
        "buyer should hold token 0"
    );
    anyhow::ensure!(
        bal(chain, seller)? - seller_before == SELLER_SHARE,
        "seller share"
    );
    anyhow::ensure!(bal(chain, fee)? - fee_before == FEE_SHARE, "fee share");
    anyhow::ensure!(
        bal(chain, royalty)? - royalty_before == ROYALTY_SHARE,
        "royalty share"
    );
    Ok(())
}

fn accept_bid(
    chain: &MockBech32,
    collection: &BenchNft<MockBech32>,
    market: &BenchMarketplace<MockBech32>,
    market_addr: &Addr,
    seller: &Addr,
    bidder: &Addr,
    fee: &Addr,
    royalty: &Addr,
    nft_addr: &Addr,
) -> anyhow::Result<()> {
    nft::approve(collection, seller, market_addr, "1")?;
    let seller_before = bal(chain, seller)?;
    let fee_before = bal(chain, fee)?;
    let royalty_before = bal(chain, royalty)?;
    market
        .call_as(seller)
        .execute(
            &marketplace::ExecuteMsg::Accept {
                collection: nft_addr.to_string(),
                token_id: "1".into(),
            },
            &[],
        )
        .context("accept")?;
    anyhow::ensure!(
        nft::owner(collection, "1")? == bidder.as_str(),
        "bidder should hold token 1"
    );
    anyhow::ensure!(
        bal(chain, seller)? - seller_before == SELLER_SHARE,
        "accept seller share"
    );
    anyhow::ensure!(
        bal(chain, fee)? - fee_before == FEE_SHARE,
        "accept fee share"
    );
    anyhow::ensure!(
        bal(chain, royalty)? - royalty_before == ROYALTY_SHARE,
        "accept royalty share"
    );
    Ok(())
}

fn cancel_ask(
    collection: &BenchNft<MockBech32>,
    market: &BenchMarketplace<MockBech32>,
    seller: &Addr,
    nft_addr: &Addr,
) -> anyhow::Result<()> {
    market
        .call_as(seller)
        .execute(
            &marketplace::ExecuteMsg::Cancel {
                collection: nft_addr.to_string(),
                token_id: "2".into(),
            },
            &[],
        )
        .context("cancel")?;
    anyhow::ensure!(
        nft::owner(collection, "2")? == seller.as_str(),
        "seller keeps token 2"
    );
    Ok(())
}

fn raffle_create(
    collection: &BenchNft<MockBech32>,
    raffle: &BenchRaffle<MockBech32>,
    raffle_addr: &Addr,
    owner: &Addr,
    nft_addr: &Addr,
) -> anyhow::Result<()> {
    nft::approve(collection, owner, raffle_addr, "3")?;
    raffle
        .call_as(owner)
        .execute(
            &raffle::ExecuteMsg::Create {
                collection: nft_addr.to_string(),
                token_id: "3".into(),
                ticket_price: Uint128::new(TICKET),
                duration_secs: 60,
            },
            &[],
        )
        .context("create raffle")?;
    anyhow::ensure!(
        nft::owner(collection, "3")? == raffle_addr.as_str(),
        "raffle should escrow token 3"
    );
    Ok(())
}

fn raffle_buy(
    chain: &MockBech32,
    raffle: &BenchRaffle<MockBech32>,
    buyer: &Addr,
) -> anyhow::Result<()> {
    let before = bal(chain, buyer)?;
    raffle
        .call_as(buyer)
        .execute(
            &raffle::ExecuteMsg::BuyTicket { raffle_id: 1 },
            &[coin(TICKET, DENOM)],
        )
        .context("buy ticket")?;
    anyhow::ensure!(before - bal(chain, buyer)? == TICKET, "ticket payment");
    Ok(())
}

fn raffle_draw(raffle: &BenchRaffle<MockBech32>, sender: &Addr) -> anyhow::Result<()> {
    raffle
        .call_as(sender)
        .execute(&raffle::ExecuteMsg::Draw { raffle_id: 1 }, &[])
        .context("draw")?;
    Ok(())
}

fn raffle_claim(
    chain: &MockBech32,
    collection: &BenchNft<MockBech32>,
    raffle: &BenchRaffle<MockBech32>,
    owner: &Addr,
    winner: &Addr,
    fee: &Addr,
) -> anyhow::Result<()> {
    let owner_before = bal(chain, owner)?;
    let fee_before = bal(chain, fee)?;
    raffle
        .call_as(winner)
        .execute(&raffle::ExecuteMsg::Claim { raffle_id: 1 }, &[])
        .context("claim")?;
    anyhow::ensure!(
        nft::owner(collection, "3")? == winner.as_str(),
        "winner should hold token 3"
    );
    anyhow::ensure!(
        bal(chain, owner)? - owner_before == RAFFLE_OWNER_SHARE,
        "raffle owner share"
    );
    anyhow::ensure!(bal(chain, fee)? - fee_before == RAFFLE_FEE, "raffle fee");
    Ok(())
}

fn loan_offers(loan: &BenchLoan<MockBech32>, lender: &Addr, nft_addr: &Addr) -> anyhow::Result<()> {
    loan.call_as(lender)
        .execute(
            &loan::ExecuteMsg::Offer {
                collection: nft_addr.to_string(),
                token_id: "4".into(),
                principal: Uint128::new(PRINCIPAL),
                duration_secs: 3_600,
            },
            &[coin(PRINCIPAL, DENOM)],
        )
        .context("offer repay loan")?;
    loan.call_as(lender)
        .execute(
            &loan::ExecuteMsg::Offer {
                collection: nft_addr.to_string(),
                token_id: "5".into(),
                principal: Uint128::new(PRINCIPAL),
                duration_secs: 30,
            },
            &[coin(PRINCIPAL, DENOM)],
        )
        .context("offer default loan")?;
    Ok(())
}

fn loan_accepts(
    chain: &MockBech32,
    collection: &BenchNft<MockBech32>,
    loan: &BenchLoan<MockBech32>,
    loan_addr: &Addr,
    borrower: &Addr,
    nft_addr: &Addr,
) -> anyhow::Result<()> {
    nft::approve(collection, borrower, loan_addr, "4")?;
    nft::approve(collection, borrower, loan_addr, "5")?;
    let before = bal(chain, borrower)?;
    for id in ["4", "5"] {
        loan.call_as(borrower)
            .execute(
                &loan::ExecuteMsg::Accept {
                    collection: nft_addr.to_string(),
                    token_id: id.into(),
                },
                &[],
            )
            .with_context(|| format!("accept loan {id}"))?;
        anyhow::ensure!(
            nft::owner(collection, id)? == loan_addr.as_str(),
            "loan should escrow {id}"
        );
    }
    anyhow::ensure!(
        bal(chain, borrower)? - before == PRINCIPAL * 2,
        "borrower receives both principals"
    );
    Ok(())
}

fn loan_repay(
    chain: &MockBech32,
    collection: &BenchNft<MockBech32>,
    loan: &BenchLoan<MockBech32>,
    borrower: &Addr,
    lender: &Addr,
    nft_addr: &Addr,
) -> anyhow::Result<()> {
    let lender_before = bal(chain, lender)?;
    loan.call_as(borrower)
        .execute(
            &loan::ExecuteMsg::Repay {
                collection: nft_addr.to_string(),
                token_id: "4".into(),
            },
            &[coin(PRINCIPAL, DENOM)],
        )
        .context("repay")?;
    anyhow::ensure!(
        nft::owner(collection, "4")? == borrower.as_str(),
        "borrower should hold token 4"
    );
    anyhow::ensure!(
        bal(chain, lender)? - lender_before == PRINCIPAL,
        "lender repaid"
    );
    Ok(())
}

fn loan_default(
    collection: &BenchNft<MockBech32>,
    loan: &BenchLoan<MockBech32>,
    lender: &Addr,
    nft_addr: &Addr,
) -> anyhow::Result<()> {
    loan.call_as(lender)
        .execute(
            &loan::ExecuteMsg::Default {
                collection: nft_addr.to_string(),
                token_id: "5".into(),
            },
            &[],
        )
        .context("default")?;
    anyhow::ensure!(
        nft::owner(collection, "5")? == lender.as_str(),
        "lender should hold token 5"
    );
    Ok(())
}

fn bal(chain: &MockBech32, addr: &Addr) -> anyhow::Result<u128> {
    let coins = chain
        .balance(addr, Some(DENOM.to_string()))
        .context("balance")?;
    Ok(coins
        .into_iter()
        .filter(|c| c.denom == DENOM)
        .map(|c| {
            Uint128::try_from(c.amount)
                .unwrap_or_else(|_| Uint128::zero())
                .u128()
        })
        .sum())
}

fn sibling_note() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let crates = root.join("..");
    let paths = [
        (
            "revenue-marketplace",
            crates.join("terp-rs-nftfi-revenue/contracts/revenue/marketplace/Cargo.toml"),
        ),
        (
            "reserve-auction",
            crates.join("terp-rs-nftfi-revenue/contracts/revenue/reserve-auction/Cargo.toml"),
        ),
        (
            "nftfi-raffle",
            crates.join("terp-rs-nftfi-nfts/contracts/nfts/raffle/Cargo.toml"),
        ),
        (
            "nftfi-loan",
            crates.join("terp-rs-nftfi-nfts/contracts/nfts/nft-loan/Cargo.toml"),
        ),
    ];
    let mut parts = Vec::new();
    for (label, path) in paths {
        if path.is_file() {
            let name = package_name(&path).unwrap_or_else(|| label.to_string());
            parts.push(format!("{name} present, unlinked"));
        } else {
            parts.push(format!("{label} missing"));
        }
    }
    format!(
        "Sibling crates (not path-deps of this bench): {}.",
        parts.join("; ")
    )
}

fn package_name(path: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("name = ") {
            return Some(rest.trim_matches('"').to_string());
        }
    }
    None
}
