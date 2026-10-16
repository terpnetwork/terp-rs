//! KV gas for one multi-test transaction.
//!
//! Same schedule as `contracts/nfts/raffle/tests/gas.rs`: the workspace
//! cw-multi-test fork reports `gas_used: 0`. This meter charges the Cosmos SDK
//! KV schedule on the shared `MockStorage`. Read is 1000 + 3 per byte of key
//! and value, write is 2000 + 30 per byte, and delete is 1000. The figure is
//! the whole transaction, including bank and wasm-keeper writes.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cosmwasm_std::testing::{MockApi, MockStorage};
use cosmwasm_std::{Addr, Coin, Order, Record, Storage, Uint128, coins};
use cw_multi_test::{App, AppBuilder, BankKeeper, BankSudo, ContractWrapper, Executor, SudoMsg};
use cw721::msg::OwnerOfResponse;

type BenchApp<S> = App<BankKeeper, MockApi, S>;

use crate::loan;
use crate::marketplace;
use crate::pay::DENOM;
use crate::raffle;

const PRICE: u128 = 1_000;
const FEE_BPS: u16 = 200;
const ROYALTY_BPS: u16 = 500;
const TICKET: u128 = 100;
const RAFFLE_FEE_BPS: u16 = 1_000;
const PRINCIPAL: u128 = 500;
const SELLER_SHARE: u128 = 930;
const FEE_SHARE: u128 = 20;
const ROYALTY_SHARE: u128 = 50;
const RAFFLE_FEE: u128 = 10;
const RAFFLE_OWNER_SHARE: u128 = 90;

pub struct Measured {
    pub protocol: &'static str,
    pub action: &'static str,
    pub gas: u64,
}

struct GasMeter {
    inner: MockStorage,
    gas: Arc<AtomicU64>,
}

impl Storage for GasMeter {
    fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        let value = self.inner.get(key);
        let bytes = key.len() + value.as_ref().map(|v| v.len()).unwrap_or(0);
        self.charge(1_000 + 3 * bytes as u64);
        value
    }

    fn set(&mut self, key: &[u8], value: &[u8]) {
        self.charge(2_000 + 30 * (key.len() + value.len()) as u64);
        self.inner.set(key, value);
    }

    fn remove(&mut self, key: &[u8]) {
        self.charge(1_000);
        self.inner.remove(key);
    }

    fn range<'a>(
        &'a self,
        start: Option<&[u8]>,
        end: Option<&[u8]>,
        order: Order,
    ) -> Box<dyn Iterator<Item = Record> + 'a> {
        let gas = Arc::clone(&self.gas);
        Box::new(self.inner.range(start, end, order).map(move |(k, v)| {
            let bytes = (k.len() + v.len()) as u64;
            gas.fetch_add(1_000 + 3 * bytes, Ordering::Relaxed);
            (k, v)
        }))
    }
}

impl GasMeter {
    fn charge(&self, amount: u64) {
        self.gas.fetch_add(amount, Ordering::Relaxed);
    }
}

fn take(gas: &AtomicU64) -> u64 {
    gas.swap(0, Ordering::SeqCst)
}

fn act(gas: &AtomicU64, run: impl FnOnce() -> anyhow::Result<()>) -> anyhow::Result<u64> {
    take(gas);
    run()?;
    Ok(take(gas))
}

pub fn measure() -> anyhow::Result<Vec<Measured>> {
    let counter = Arc::new(AtomicU64::new(0));
    let mut app = AppBuilder::new()
        .with_storage(GasMeter {
            inner: MockStorage::new(),
            gas: Arc::clone(&counter),
        })
        .build(|_, _, _| {});

    let api = app.api().clone();
    let admin = api.addr_make("admin");
    let seller = api.addr_make("seller");
    let buyer = api.addr_make("buyer");
    let bidder = api.addr_make("bidder");
    let fee = api.addr_make("fee");
    let royalty = api.addr_make("royalty");
    let raffle_owner = api.addr_make("raffle-owner");
    let ticket_buyer = api.addr_make("ticket-buyer");
    let borrower = api.addr_make("borrower");
    let lender = api.addr_make("lender");

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
        app.sudo(SudoMsg::Bank(BankSudo::Mint {
            to_address: addr.to_string(),
            amount: coins(1_000_000, DENOM),
        }))
        .map_err(|e| anyhow::anyhow!("mint bank: {e}"))?;
    }

    let nft_id = app.store_code(Box::new(ContractWrapper::new(
        cw721_base::entry::execute,
        cw721_base::entry::instantiate,
        cw721_base::entry::query,
    )));
    let nft = app
        .instantiate_contract(
            nft_id,
            admin.clone(),
            &cw721_base::msg::InstantiateMsg {
                name: "nftfi-bench".into(),
                symbol: "BENCH".into(),
                collection_info_extension: None,
                minter: Some(admin.to_string()),
                creator: Some(admin.to_string()),
                withdraw_address: Some(admin.to_string()),
            },
            &[],
            "cw721",
            Some(admin.to_string()),
        )
        .map_err(|e| anyhow::anyhow!("instantiate cw721: {e}"))?;
    for id in 0..6 {
        app.execute_contract(
            admin.clone(),
            nft.clone(),
            &cw721_base::msg::ExecuteMsg::Mint {
                token_id: id.to_string(),
                owner: admin.to_string(),
                token_uri: None,
                extension: None,
            },
            &[],
        )
        .map_err(|e| anyhow::anyhow!("mint {id}: {e}"))?;
    }
    for (id, to) in [
        ("0", &seller),
        ("1", &seller),
        ("2", &seller),
        ("3", &raffle_owner),
        ("4", &borrower),
        ("5", &borrower),
    ] {
        transfer(&mut app, &admin, &nft, id, to)?;
    }

    let market_id = app.store_code(Box::new(ContractWrapper::new_with_empty(
        marketplace::execute,
        marketplace::instantiate,
        marketplace::query,
    )));
    let market = app
        .instantiate_contract(
            market_id,
            admin.clone(),
            &marketplace::InstantiateMsg {
                fee_bps: FEE_BPS,
                royalty_bps: ROYALTY_BPS,
                fee_addr: fee.to_string(),
                royalty_addr: royalty.to_string(),
            },
            &[],
            "marketplace",
            Some(admin.to_string()),
        )
        .map_err(|e| anyhow::anyhow!("instantiate marketplace: {e}"))?;

    let raffle_id = app.store_code(Box::new(ContractWrapper::new_with_empty(
        raffle::execute,
        raffle::instantiate,
        raffle::query,
    )));
    let raffle = app
        .instantiate_contract(
            raffle_id,
            admin.clone(),
            &raffle::InstantiateMsg {
                fee_bps: RAFFLE_FEE_BPS,
                fee_addr: fee.to_string(),
                draw_seed: "bench-seed".into(),
            },
            &[],
            "raffle",
            Some(admin.to_string()),
        )
        .map_err(|e| anyhow::anyhow!("instantiate raffle: {e}"))?;

    let loan_id = app.store_code(Box::new(ContractWrapper::new_with_empty(
        loan::execute,
        loan::instantiate,
        loan::query,
    )));
    let loan = app
        .instantiate_contract(
            loan_id,
            admin.clone(),
            &loan::InstantiateMsg {},
            &[],
            "loan",
            Some(admin.to_string()),
        )
        .map_err(|e| anyhow::anyhow!("instantiate loan: {e}"))?;

    let mut rows = Vec::new();

    rows.push(Measured {
        protocol: "marketplace",
        action: "list",
        gas: act(&counter, || {
            for id in ["0", "1", "2"] {
                app.execute_contract(
                    seller.clone(),
                    market.clone(),
                    &marketplace::ExecuteMsg::List {
                        collection: nft.to_string(),
                        token_id: id.into(),
                        price: Uint128::new(PRICE),
                    },
                    &[],
                )
                .map_err(|e| anyhow::anyhow!("list {id}: {e}"))?;
            }
            Ok(())
        })?,
    });
    rows.push(Measured {
        protocol: "marketplace",
        action: "bid",
        gas: act(&counter, || {
            app.execute_contract(
                bidder.clone(),
                market.clone(),
                &marketplace::ExecuteMsg::Bid {
                    collection: nft.to_string(),
                    token_id: "1".into(),
                    price: Uint128::new(PRICE),
                },
                &coins(PRICE, DENOM),
            )
            .map_err(|e| anyhow::anyhow!("bid: {e}"))?;
            Ok(())
        })?,
    });
    approve(&mut app, &seller, &nft, &market, "0")?;
    let seller_before = bal(&app, &seller)?;
    let fee_before = bal(&app, &fee)?;
    let royalty_before = bal(&app, &royalty)?;
    rows.push(Measured {
        protocol: "marketplace",
        action: "buy",
        gas: act(&counter, || {
            app.execute_contract(
                buyer.clone(),
                market.clone(),
                &marketplace::ExecuteMsg::Buy {
                    collection: nft.to_string(),
                    token_id: "0".into(),
                },
                &coins(PRICE, DENOM),
            )
            .map_err(|e| anyhow::anyhow!("buy: {e}"))?;
            Ok(())
        })?,
    });
    anyhow::ensure!(
        owner_of(&app, &nft, "0")? == buyer.as_str(),
        "buyer holds 0"
    );
    anyhow::ensure!(
        bal(&app, &seller)? - seller_before == SELLER_SHARE,
        "seller share"
    );
    anyhow::ensure!(bal(&app, &fee)? - fee_before == FEE_SHARE, "fee share");
    anyhow::ensure!(
        bal(&app, &royalty)? - royalty_before == ROYALTY_SHARE,
        "royalty share"
    );

    approve(&mut app, &seller, &nft, &market, "1")?;
    let seller_before = bal(&app, &seller)?;
    let fee_before = bal(&app, &fee)?;
    let royalty_before = bal(&app, &royalty)?;
    rows.push(Measured {
        protocol: "marketplace",
        action: "accept",
        gas: act(&counter, || {
            app.execute_contract(
                seller.clone(),
                market.clone(),
                &marketplace::ExecuteMsg::Accept {
                    collection: nft.to_string(),
                    token_id: "1".into(),
                },
                &[],
            )
            .map_err(|e| anyhow::anyhow!("accept: {e}"))?;
            Ok(())
        })?,
    });
    anyhow::ensure!(
        owner_of(&app, &nft, "1")? == bidder.as_str(),
        "bidder holds 1"
    );
    anyhow::ensure!(
        bal(&app, &seller)? - seller_before == SELLER_SHARE,
        "accept seller"
    );
    anyhow::ensure!(bal(&app, &fee)? - fee_before == FEE_SHARE, "accept fee");
    anyhow::ensure!(
        bal(&app, &royalty)? - royalty_before == ROYALTY_SHARE,
        "accept royalty"
    );

    rows.push(Measured {
        protocol: "marketplace",
        action: "cancel",
        gas: act(&counter, || {
            app.execute_contract(
                seller.clone(),
                market.clone(),
                &marketplace::ExecuteMsg::Cancel {
                    collection: nft.to_string(),
                    token_id: "2".into(),
                },
                &[],
            )
            .map_err(|e| anyhow::anyhow!("cancel: {e}"))?;
            Ok(())
        })?,
    });
    anyhow::ensure!(
        owner_of(&app, &nft, "2")? == seller.as_str(),
        "seller keeps 2"
    );

    approve(&mut app, &raffle_owner, &nft, &raffle, "3")?;
    rows.push(Measured {
        protocol: "raffle",
        action: "create",
        gas: act(&counter, || {
            app.execute_contract(
                raffle_owner.clone(),
                raffle.clone(),
                &raffle::ExecuteMsg::Create {
                    collection: nft.to_string(),
                    token_id: "3".into(),
                    ticket_price: Uint128::new(TICKET),
                    duration_secs: 60,
                },
                &[],
            )
            .map_err(|e| anyhow::anyhow!("create raffle: {e}"))?;
            Ok(())
        })?,
    });
    anyhow::ensure!(
        owner_of(&app, &nft, "3")? == raffle.as_str(),
        "raffle escrow"
    );

    rows.push(Measured {
        protocol: "raffle",
        action: "buy_ticket",
        gas: act(&counter, || {
            app.execute_contract(
                ticket_buyer.clone(),
                raffle.clone(),
                &raffle::ExecuteMsg::BuyTicket { raffle_id: 1 },
                &coins(TICKET, DENOM),
            )
            .map_err(|e| anyhow::anyhow!("buy ticket: {e}"))?;
            Ok(())
        })?,
    });
    app.update_block(|block| {
        block.time = block.time.plus_seconds(60);
    });
    rows.push(Measured {
        protocol: "raffle",
        action: "draw",
        gas: act(&counter, || {
            app.execute_contract(
                ticket_buyer.clone(),
                raffle.clone(),
                &raffle::ExecuteMsg::Draw { raffle_id: 1 },
                &[],
            )
            .map_err(|e| anyhow::anyhow!("draw: {e}"))?;
            Ok(())
        })?,
    });
    let owner_before = bal(&app, &raffle_owner)?;
    let fee_before = bal(&app, &fee)?;
    rows.push(Measured {
        protocol: "raffle",
        action: "claim",
        gas: act(&counter, || {
            app.execute_contract(
                ticket_buyer.clone(),
                raffle.clone(),
                &raffle::ExecuteMsg::Claim { raffle_id: 1 },
                &[],
            )
            .map_err(|e| anyhow::anyhow!("claim: {e}"))?;
            Ok(())
        })?,
    });
    anyhow::ensure!(
        owner_of(&app, &nft, "3")? == ticket_buyer.as_str(),
        "winner holds 3"
    );
    anyhow::ensure!(
        bal(&app, &raffle_owner)? - owner_before == RAFFLE_OWNER_SHARE,
        "raffle owner"
    );
    anyhow::ensure!(bal(&app, &fee)? - fee_before == RAFFLE_FEE, "raffle fee");

    rows.push(Measured {
        protocol: "loan",
        action: "offer",
        gas: act(&counter, || {
            for (id, duration) in [("4", 3_600u64), ("5", 30u64)] {
                app.execute_contract(
                    lender.clone(),
                    loan.clone(),
                    &loan::ExecuteMsg::Offer {
                        collection: nft.to_string(),
                        token_id: id.into(),
                        principal: Uint128::new(PRINCIPAL),
                        duration_secs: duration,
                    },
                    &coins(PRINCIPAL, DENOM),
                )
                .map_err(|e| anyhow::anyhow!("offer {id}: {e}"))?;
            }
            Ok(())
        })?,
    });
    approve(&mut app, &borrower, &nft, &loan, "4")?;
    approve(&mut app, &borrower, &nft, &loan, "5")?;
    let borrower_before = bal(&app, &borrower)?;
    rows.push(Measured {
        protocol: "loan",
        action: "accept",
        gas: act(&counter, || {
            for id in ["4", "5"] {
                app.execute_contract(
                    borrower.clone(),
                    loan.clone(),
                    &loan::ExecuteMsg::Accept {
                        collection: nft.to_string(),
                        token_id: id.into(),
                    },
                    &[],
                )
                .map_err(|e| anyhow::anyhow!("accept loan {id}: {e}"))?;
            }
            Ok(())
        })?,
    });
    anyhow::ensure!(
        bal(&app, &borrower)? - borrower_before == PRINCIPAL * 2,
        "borrower principal"
    );
    let lender_before = bal(&app, &lender)?;
    rows.push(Measured {
        protocol: "loan",
        action: "repay",
        gas: act(&counter, || {
            app.execute_contract(
                borrower.clone(),
                loan.clone(),
                &loan::ExecuteMsg::Repay {
                    collection: nft.to_string(),
                    token_id: "4".into(),
                },
                &coins(PRINCIPAL, DENOM),
            )
            .map_err(|e| anyhow::anyhow!("repay: {e}"))?;
            Ok(())
        })?,
    });
    anyhow::ensure!(
        owner_of(&app, &nft, "4")? == borrower.as_str(),
        "repaid nft"
    );
    anyhow::ensure!(
        bal(&app, &lender)? - lender_before == PRINCIPAL,
        "lender repaid"
    );
    app.update_block(|block| {
        block.time = block.time.plus_seconds(31);
    });
    rows.push(Measured {
        protocol: "loan",
        action: "default",
        gas: act(&counter, || {
            app.execute_contract(
                lender.clone(),
                loan.clone(),
                &loan::ExecuteMsg::Default {
                    collection: nft.to_string(),
                    token_id: "5".into(),
                },
                &[],
            )
            .map_err(|e| anyhow::anyhow!("default: {e}"))?;
            Ok(())
        })?,
    });
    anyhow::ensure!(
        owner_of(&app, &nft, "5")? == lender.as_str(),
        "defaulted nft"
    );

    Ok(rows)
}

fn transfer<S: Storage>(
    app: &mut BenchApp<S>,
    from: &Addr,
    nft: &Addr,
    token_id: &str,
    to: &Addr,
) -> anyhow::Result<()> {
    app.execute_contract(
        from.clone(),
        nft.clone(),
        &cw721_base::msg::ExecuteMsg::TransferNft {
            recipient: to.to_string(),
            token_id: token_id.to_string(),
        },
        &[],
    )
    .map_err(|e| anyhow::anyhow!("transfer {token_id}: {e}"))?;
    Ok(())
}

fn approve<S: Storage>(
    app: &mut BenchApp<S>,
    from: &Addr,
    nft: &Addr,
    spender: &Addr,
    token_id: &str,
) -> anyhow::Result<()> {
    app.execute_contract(
        from.clone(),
        nft.clone(),
        &cw721_base::msg::ExecuteMsg::Approve {
            spender: spender.to_string(),
            token_id: token_id.to_string(),
            expires: None,
        },
        &[],
    )
    .map_err(|e| anyhow::anyhow!("approve {token_id}: {e}"))?;
    Ok(())
}

fn owner_of<S: Storage>(app: &BenchApp<S>, nft: &Addr, token_id: &str) -> anyhow::Result<String> {
    let resp: OwnerOfResponse = app
        .wrap()
        .query_wasm_smart(
            nft,
            &cw721_base::msg::QueryMsg::OwnerOf {
                token_id: token_id.to_string(),
                include_expired: None,
            },
        )
        .map_err(|e| anyhow::anyhow!("owner of {token_id}: {e}"))?;
    Ok(resp.owner)
}

fn bal<S: Storage>(app: &BenchApp<S>, addr: &Addr) -> anyhow::Result<u128> {
    let coin: Coin = app
        .wrap()
        .query_balance(addr, DENOM)
        .map_err(|e| anyhow::anyhow!("balance: {e}"))?;
    Ok(Uint128::try_from(coin.amount)
        .unwrap_or_else(|_| Uint128::zero())
        .u128())
}
