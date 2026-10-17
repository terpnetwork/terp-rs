//! KV gas for one multi-test transaction.
//!
//! The workspace cw-multi-test fork reports `gas_used: 0`. This meter charges
//! the Cosmos SDK KV schedule on the shared `MockStorage`: read 1000 + 3 per
//! byte of key and value, write 2000 + 30 per byte, delete 1000. The figure
//! is the whole transaction, including bank and wasm-keeper writes.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use cosmwasm_std::testing::MockStorage;
use cosmwasm_std::{coin, coins, Addr, Binary, Decimal, Empty, Order, Record, Storage, Uint256};
use cw721::msg::Cw721ExecuteMsg;
use cw_multi_test::{AppBuilder, BankSudo, ContractWrapper, Executor, SudoMsg};
use nftfi_loan::msg::ExecuteMsg as LoanMsg;
use nftfi_loan::msg::InstantiateMsg as LoanInit;
use nftfi_loan::state::LoanTerms;
use nftfi_raffle::msg::{ExecuteMsg, InstantiateMsg, RaffleOptionsMsg};
use nftfi_randomness::RandomnessSourceMsg;
use nftfi_utils::mock::InstantiateMsg as NftInit;
use nftfi_utils::AssetInfo;

const DENOM: &str = "uterp";

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

struct Row {
    action: &'static str,
    gas: u64,
}

#[test]
fn mock_gas_bench() {
    let counter = Arc::new(AtomicU64::new(0));
    let mut app = AppBuilder::new()
        .with_storage(GasMeter {
            inner: MockStorage::new(),
            gas: Arc::clone(&counter),
        })
        .build(|_, _, _| {});

    let owner = app.api().addr_make("owner");
    let alice = app.api().addr_make("alice");
    let lender = app.api().addr_make("lender");
    let fee = app.api().addr_make("fee");
    for who in [&owner, &alice, &lender] {
        app.sudo(SudoMsg::Bank(BankSudo::Mint {
            to_address: who.to_string(),
            amount: coins(20_000, DENOM),
        }))
        .unwrap();
    }

    let nft_id = app.store_code(nftfi_utils::mock::contract());
    let nft = app
        .instantiate_contract(
            nft_id,
            owner.clone(),
            &NftInit {
                minter: owner.to_string(),
            },
            &[],
            "cw721",
            None,
        )
        .unwrap();
    for (id, to) in [("1", &owner), ("2", &owner), ("3", &alice)] {
        app.execute_contract(
            owner.clone(),
            nft.clone(),
            &Cw721ExecuteMsg::<Empty, Empty, Empty>::Mint {
                token_id: id.into(),
                owner: to.to_string(),
                token_uri: None,
                extension: Empty {},
            },
            &[],
        )
        .unwrap();
    }

    let mut rows = Vec::new();

    take(&counter);
    let raffle_code = app.store_code(Box::new(
        ContractWrapper::new_with_empty(
            nftfi_raffle::execute,
            nftfi_raffle::instantiate,
            nftfi_raffle::query,
        )
        .with_reply_empty(nftfi_raffle::reply),
    ));
    let raffle = app
        .instantiate_contract(
            raffle_code,
            owner.clone(),
            &InstantiateMsg {
                name: "raffle".into(),
                owner: None,
                fee_addr: fee.to_string(),
                minimum_raffle_duration: Some(1),
                max_tickets_per_raffle: Some(100),
                raffle_fee: Decimal::percent(10),
                creation_coins: Some(vec![coin(100, DENOM)]),
                randomness: RandomnessSourceMsg::Seed { provider: None },
            },
            &[],
            "raffle",
            None,
        )
        .unwrap();
    rows.push(Row {
        action: "raffle instantiate",
        gas: take(&counter),
    });

    approve(&mut app, &owner, &nft, &raffle, "1");
    approve(&mut app, &owner, &nft, &raffle, "2");
    rows.push(Row {
        action: "raffle create",
        gas: act(&counter, || {
            create(&mut app, &owner, &raffle, &nft, "1", 100);
        }),
    });
    rows.push(Row {
        action: "raffle buy ticket",
        gas: act(&counter, || {
            app.execute_contract(
                alice.clone(),
                raffle.clone(),
                &ExecuteMsg::BuyTicket {
                    raffle_id: 0,
                    ticket_count: 1,
                    on_behalf_of: None,
                },
                &coins(10, DENOM),
            )
            .unwrap();
        }),
    });
    app.update_block(|b| {
        b.time = b.time.plus_seconds(101);
    });
    rows.push(Row {
        action: "raffle draw",
        gas: act(&counter, || {
            app.execute_contract(
                owner.clone(),
                raffle.clone(),
                &ExecuteMsg::Draw {
                    raffle_id: 0,
                    seed: Binary::from([9u8; 32]),
                },
                &[],
            )
            .unwrap();
        }),
    });
    rows.push(Row {
        action: "raffle claim",
        gas: act(&counter, || {
            app.execute_contract(
                alice.clone(),
                raffle.clone(),
                &ExecuteMsg::ClaimRaffle { raffle_id: 0 },
                &[],
            )
            .unwrap();
        }),
    });

    // A second raffle, still inside its sale window, with no tickets.
    app.update_block(|b| {
        b.time = b.time.plus_seconds(1);
    });
    create(&mut app, &owner, &raffle, &nft, "2", 100);
    rows.push(Row {
        action: "raffle cancel",
        gas: act(&counter, || {
            app.execute_contract(
                owner.clone(),
                raffle.clone(),
                &ExecuteMsg::CancelRaffle { raffle_id: 1 },
                &[],
            )
            .unwrap();
        }),
    });

    take(&counter);
    let loan_code = app.store_code(Box::new(ContractWrapper::new_with_empty(
        nftfi_loan::execute,
        nftfi_loan::instantiate,
        nftfi_loan::query,
    )));
    let loan = app
        .instantiate_contract(
            loan_code,
            owner.clone(),
            &LoanInit {
                name: "loans".into(),
                owner: None,
                treasury_addr: fee.to_string(),
                fee_rate: Decimal::percent(10),
                listing_fee_coins: Some(vec![coin(50, DENOM)]),
            },
            &[],
            "loan",
            None,
        )
        .unwrap();
    rows.push(Row {
        action: "loan instantiate",
        gas: take(&counter),
    });

    approve(&mut app, &alice, &nft, &loan, "3");
    // Mint a second token for the borrower so list / default / withdraw each have a token.
    app.execute_contract(
        owner.clone(),
        nft.clone(),
        &Cw721ExecuteMsg::<Empty, Empty, Empty>::Mint {
            token_id: "4".into(),
            owner: alice.to_string(),
            token_uri: None,
            extension: Empty {},
        },
        &[],
    )
    .unwrap();
    approve(&mut app, &alice, &nft, &loan, "4");
    app.execute_contract(
        owner.clone(),
        nft.clone(),
        &Cw721ExecuteMsg::<Empty, Empty, Empty>::Mint {
            token_id: "5".into(),
            owner: alice.to_string(),
            token_uri: None,
            extension: Empty {},
        },
        &[],
    )
    .unwrap();
    approve(&mut app, &alice, &nft, &loan, "5");

    let ask = LoanTerms {
        principle: coin(1_000, DENOM),
        interest: Uint256::from(100u128),
        duration_in_blocks: 5,
    };
    rows.push(Row {
        action: "loan list",
        gas: act(&counter, || {
            list(&mut app, &alice, &loan, &nft, "3", Some(ask.clone()), 50);
        }),
    });
    rows.push(Row {
        action: "loan offer",
        gas: act(&counter, || {
            app.execute_contract(
                lender.clone(),
                loan.clone(),
                &LoanMsg::MakeOffer {
                    borrower: alice.to_string(),
                    loan_id: 0,
                    terms: ask.clone(),
                    comment: None,
                    on_behalf_of: None,
                },
                &coins(1_000, DENOM),
            )
            .unwrap();
        }),
    });
    rows.push(Row {
        action: "loan accept",
        gas: act(&counter, || {
            app.execute_contract(
                alice.clone(),
                loan.clone(),
                &LoanMsg::AcceptOffer {
                    global_offer_id: "1".into(),
                },
                &[],
            )
            .unwrap();
        }),
    });
    rows.push(Row {
        action: "loan repay",
        gas: act(&counter, || {
            app.execute_contract(
                alice.clone(),
                loan.clone(),
                &LoanMsg::Repay { loan_id: 0 },
                &coins(1_100, DENOM),
            )
            .unwrap();
        }),
    });

    list(&mut app, &alice, &loan, &nft, "4", Some(ask.clone()), 50);
    app.execute_contract(
        lender.clone(),
        loan.clone(),
        &LoanMsg::AcceptLoan {
            borrower: alice.to_string(),
            loan_id: 1,
            comment: None,
        },
        &coins(1_000, DENOM),
    )
    .unwrap();
    let start = app.block_info().height;
    app.update_block(|b| {
        b.height = start + 6;
    });
    rows.push(Row {
        action: "loan default",
        gas: act(&counter, || {
            app.execute_contract(
                lender.clone(),
                loan.clone(),
                &LoanMsg::WithdrawDefaulted {
                    borrower: alice.to_string(),
                    loan_id: 1,
                },
                &[],
            )
            .unwrap();
        }),
    });

    list(&mut app, &alice, &loan, &nft, "5", None, 50);
    rows.push(Row {
        action: "loan withdraw",
        gas: act(&counter, || {
            app.execute_contract(
                alice.clone(),
                loan.clone(),
                &LoanMsg::WithdrawCollaterals { loan_id: 2 },
                &[],
            )
            .unwrap();
        }),
    });

    let mut body = String::from(
        "# Mock gas\n\n\
KV gas for one multi-test transaction. Read is 1000 plus 3 per byte of key and value, \
write is 2000 plus 30 per byte, and delete is 1000. \
The workspace cw-multi-test fork stores `gas_used: 0` on replies, so this is storage gas \
for the transaction, including bank and wasm-keeper writes, not wasmd opcode gas.\n\n\
| action | gas |\n| --- | ---: |\n",
    );
    for row in &rows {
        assert!(row.gas > 0, "{} gas", row.action);
        println!("{:<22} {}", row.action, row.gas);
        body.push_str(&format!("| {} | {} |\n", row.action, row.gas));
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../.herdr/gas.md");
    std::fs::write(&path, body).unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
}

fn act(gas: &AtomicU64, run: impl FnOnce()) -> u64 {
    take(gas);
    run();
    take(gas)
}

fn approve(app: &mut impl Executor<Empty>, from: &Addr, nft: &Addr, spender: &Addr, token_id: &str) {
    app.execute_contract(
        from.clone(),
        nft.clone(),
        &Cw721ExecuteMsg::<Empty, Empty, Empty>::Approve {
            spender: spender.to_string(),
            token_id: token_id.to_string(),
            expires: None,
        },
        &[],
    )
    .unwrap();
}

fn create(
    app: &mut impl Executor<Empty>,
    owner: &Addr,
    raffle: &Addr,
    nft: &Addr,
    token_id: &str,
    fee: u128,
) {
    app.execute_contract(
        owner.clone(),
        raffle.clone(),
        &ExecuteMsg::CreateRaffle {
            owner: None,
            assets: vec![AssetInfo::cw721(nft.as_str(), token_id)],
            raffle_options: RaffleOptionsMsg {
                start: None,
                duration: 100,
                comment: None,
                max_ticket_number: Some(10),
                max_ticket_per_address: None,
                min_ticket_number: None,
                one_winner_per_asset: false,
            },
            raffle_ticket_price: AssetInfo::coin(10, DENOM),
        },
        &coins(fee, DENOM),
    )
    .unwrap();
}

fn list(
    app: &mut impl Executor<Empty>,
    borrower: &Addr,
    loan: &Addr,
    nft: &Addr,
    token_id: &str,
    terms: Option<LoanTerms>,
    fee: u128,
) {
    app.execute_contract(
        borrower.clone(),
        loan.clone(),
        &LoanMsg::ListCollaterals {
            tokens: vec![AssetInfo::cw721(nft.as_str(), token_id)],
            terms,
            comment: None,
        },
        &coins(fee, DENOM),
    )
    .unwrap();
}
