//! Reserve-auction flows. Gas is unmetered in this multi-test fork; the bench prints event counts.

use cosmwasm_std::{coins, to_json_binary, Addr, Coin, Decimal, Empty, Uint128, Uint256};
use cw721::error::Cw721ContractError;
use cw721::extension::Cw721OnchainExtensions;
use cw721::msg::{
    CollectionExtensionMsg, Cw721ExecuteMsg, Cw721InstantiateMsg, Cw721QueryMsg, OwnerOfResponse,
    RoyaltyInfoResponse,
};
use cw721::traits::{Cw721Execute, Cw721Query};
use cw721::{
    DefaultOptionalCollectionExtension, DefaultOptionalCollectionExtensionMsg,
    DefaultOptionalNftExtension, DefaultOptionalNftExtensionMsg,
};
use cw_multi_test::{App, AppResponse, ContractWrapper, Executor};

use crate::msg::{ExecuteMsg, HookMsg, InstantiateMsg, QueryMsg, SudoMsg};
use crate::state::Auction;
use crate::{execute, instantiate, query, sudo};

const DENOM: &str = "uterp";

fn amount(n: u128) -> Coin {
    Coin::new(Uint256::from(n), DENOM)
}

fn note(action: &str, res: &AppResponse) {
    let attrs: usize = res.events.iter().map(|e| e.attributes.len()).sum();
    println!(
        "GAS reserve-auction {action} events={} attrs={} gas_used=unmetered",
        res.events.len(),
        attrs
    );
}

fn cw721_instantiate(
    deps: cosmwasm_std::DepsMut,
    env: cosmwasm_std::Env,
    info: cosmwasm_std::MessageInfo,
    msg: Cw721InstantiateMsg<DefaultOptionalCollectionExtensionMsg>,
) -> Result<cosmwasm_std::Response, Cw721ContractError> {
    Cw721OnchainExtensions::default().instantiate_with_version(deps, &env, &info, msg, "cw721", "1")
}

fn cw721_execute(
    deps: cosmwasm_std::DepsMut,
    env: cosmwasm_std::Env,
    info: cosmwasm_std::MessageInfo,
    msg: Cw721ExecuteMsg<
        DefaultOptionalNftExtensionMsg,
        DefaultOptionalCollectionExtensionMsg,
        Empty,
    >,
) -> Result<cosmwasm_std::Response, Cw721ContractError> {
    Cw721OnchainExtensions::default().execute(deps, &env, &info, msg)
}

fn cw721_query(
    deps: cosmwasm_std::Deps,
    env: cosmwasm_std::Env,
    msg: Cw721QueryMsg<DefaultOptionalNftExtension, DefaultOptionalCollectionExtension, Empty>,
) -> Result<cosmwasm_std::Binary, Cw721ContractError> {
    Cw721OnchainExtensions::default().query(deps, &env, msg)
}

struct World {
    app: App,
    auction: Addr,
    collection: Addr,
    seller: Addr,
    buyer: Addr,
    buyer2: Addr,
    royalty: Addr,
    fee_manager: Addr,
}

fn world(royalty_share: Option<Decimal>) -> World {
    let mut app = App::default();
    let seller = app.api().addr_make("seller");
    let buyer = app.api().addr_make("buyer");
    let buyer2 = app.api().addr_make("buyer2");
    let royalty = app.api().addr_make("royalty");
    let fee_manager = app.api().addr_make("fee");
    app.init_modules(|router, _, storage| {
        router
            .bank
            .init_balance(storage, &buyer, coins(500_000, DENOM))
            .unwrap();
        router
            .bank
            .init_balance(storage, &buyer2, coins(500_000, DENOM))
            .unwrap();
        router
            .bank
            .init_balance(storage, &seller, coins(10_000, DENOM))
            .unwrap();
    });
    let cw721_id = app.store_code(Box::new(ContractWrapper::new(
        cw721_execute,
        cw721_instantiate,
        cw721_query,
    )));
    let extension = royalty_share.map(|share| CollectionExtensionMsg {
        description: Some("auction".into()),
        image: Some("https://example.invalid/a.png".into()),
        external_link: None,
        banner_url: None,
        explicit_content: None,
        start_trading_time: None,
        royalty_info: Some(RoyaltyInfoResponse {
            payment_address: royalty.to_string(),
            share,
        }),
    });
    let collection = app
        .instantiate_contract(
            cw721_id,
            seller.clone(),
            &Cw721InstantiateMsg::<DefaultOptionalCollectionExtensionMsg> {
                name: "A".into(),
                symbol: "A".into(),
                collection_info_extension: extension,
                minter: Some(seller.to_string()),
                creator: Some(seller.to_string()),
                withdraw_address: None,
            },
            &[],
            "cw721",
            None,
        )
        .unwrap();
    let code = app.store_code(Box::new(
        ContractWrapper::new_with_empty(execute, instantiate, query).with_sudo(sudo),
    ));
    let auction = app
        .instantiate_contract(
            code,
            seller.clone(),
            &InstantiateMsg {
                fee_manager: fee_manager.to_string(),
                trading_fee_percent: Decimal::percent(2),
                min_bid_increment_percent: Decimal::percent(5),
                min_duration: 60,
                max_duration: 3600,
                extend_duration: 60,
                create_auction_fee: Coin::new(Uint256::zero(), DENOM),
                max_auctions_to_settle_per_block: 5,
                halt_duration_threshold: 600,
                halt_buffer_duration: 60,
                halt_postpone_duration: 120,
                burn_bps: 5_000,
                min_reserve_prices: vec![amount(100)],
            },
            &[],
            "reserve-auction",
            None,
        )
        .unwrap();
    World {
        app,
        auction,
        collection,
        seller,
        buyer,
        buyer2,
        royalty,
        fee_manager,
    }
}

impl World {
    fn mint_and_approve(&mut self, token_id: &str) {
        let mint = Cw721ExecuteMsg::<
            DefaultOptionalNftExtensionMsg,
            DefaultOptionalCollectionExtensionMsg,
            Empty,
        >::Mint {
            token_id: token_id.into(),
            owner: self.seller.to_string(),
            token_uri: None,
            extension: None,
        };
        self.app
            .execute_contract(self.seller.clone(), self.collection.clone(), &mint, &[])
            .unwrap();
        let approve = Cw721ExecuteMsg::<
            DefaultOptionalNftExtensionMsg,
            DefaultOptionalCollectionExtensionMsg,
            Empty,
        >::ApproveAll {
            operator: self.auction.to_string(),
            expires: None,
        };
        self.app
            .execute_contract(self.seller.clone(), self.collection.clone(), &approve, &[])
            .unwrap();
    }

    fn create(&mut self, token_id: &str, reserve: u128) -> AppResponse {
        let res = self
            .app
            .execute_contract(
                self.seller.clone(),
                self.auction.clone(),
                &ExecuteMsg::CreateAuction {
                    collection: self.collection.to_string(),
                    token_id: token_id.into(),
                    reserve_price: amount(reserve),
                    duration: 100,
                    seller_funds_recipient: None,
                    instant_sale: false,
                },
                &[],
            )
            .unwrap();
        note("create-auction", &res);
        res
    }

    fn bid(&mut self, sender: &Addr, token_id: &str, value: u128) -> Result<AppResponse, cosmwasm_std::StdError> {
        let sender = sender.clone();
        let auction = self.auction.clone();
        let collection = self.collection.to_string();
        let res = self.app.execute_contract(
            sender,
            auction,
            &ExecuteMsg::PlaceBid {
                collection,
                token_id: token_id.into(),
            },
            &[amount(value)],
        )?;
        note("place-bid", &res);
        Ok(res)
    }

    fn owner(&self, token_id: &str) -> String {
        let res: OwnerOfResponse = self
            .app
            .wrap()
            .query_wasm_smart(
                &self.collection,
                &Cw721QueryMsg::<
                    DefaultOptionalNftExtension,
                    DefaultOptionalCollectionExtension,
                    Empty,
                >::OwnerOf {
                    token_id: token_id.into(),
                    include_expired: None,
                },
            )
            .unwrap();
        res.owner
    }

    fn bal(&self, addr: &Addr) -> u128 {
        self.app
            .wrap()
            .query_balance(addr, DENOM)
            .unwrap()
            .amount
            .to_string()
            .parse()
            .unwrap()
    }

    fn auction_of(&self, token_id: &str) -> Option<Auction> {
        self.app
            .wrap()
            .query_wasm_smart(
                &self.auction,
                &QueryMsg::Auction {
                    collection: self.collection.to_string(),
                    token_id: token_id.into(),
                },
            )
            .unwrap()
    }
}

#[test]
fn bid_outbid_settle_cancel_and_royalty() {
    let mut w = world(Some(Decimal::percent(5)));
    w.mint_and_approve("1");
    w.mint_and_approve("2");
    w.create("1", 10_000);
    assert_eq!(w.owner("1"), w.auction.to_string());

    let seller_bid = w.bid(&w.seller.clone(), "1", 10_000).unwrap_err();
    assert!(seller_bid.to_string().contains("seller cannot bid"), "{seller_bid}");

    let low = w.bid(&w.buyer.clone(), "1", 9_000).unwrap_err();
    assert!(low.to_string().contains("bid below"), "{low}");

    w.bid(&w.buyer.clone(), "1", 10_000).unwrap();
    assert_eq!(w.bal(&w.buyer), 500_000 - 10_000);
    w.bid(&w.buyer2.clone(), "1", 10_500).unwrap();
    assert_eq!(w.bal(&w.buyer), 500_000, "previous bidder is refunded");
    assert_eq!(w.bal(&w.buyer2), 500_000 - 10_500);

    let early = w
        .app
        .execute_contract(
            w.buyer.clone(),
            w.auction.clone(),
            &ExecuteMsg::SettleAuction {
                collection: w.collection.to_string(),
                token_id: "1".into(),
            },
            &[],
        )
        .unwrap_err();
    assert!(early.to_string().contains("not ended"), "{early}");

    w.app.update_block(|b| {
        b.time = b.time.plus_seconds(200);
        b.height += 20;
    });
    let settled = w
        .app
        .execute_contract(
            w.buyer.clone(),
            w.auction.clone(),
            &ExecuteMsg::SettleAuction {
                collection: w.collection.to_string(),
                token_id: "1".into(),
            },
            &[],
        )
        .unwrap();
    note("settle", &settled);
    assert_eq!(w.owner("1"), w.buyer2.to_string());
    assert!(w.auction_of("1").is_none());
    // 2% of 10500 = 210, burn half (105) fee manager 105, royalty 5% = 525, seller 10500-210-525.
    assert_eq!(w.bal(&w.royalty), 525);
    assert_eq!(w.bal(&w.fee_manager), 105);
    assert_eq!(w.bal(&w.seller), 10_000 + 10_500 - 210 - 525);

    w.create("2", 1_000);
    let removed = w
        .app
        .execute_contract(
            w.seller.clone(),
            w.auction.clone(),
            &ExecuteMsg::CancelAuction {
                collection: w.collection.to_string(),
                token_id: "2".into(),
            },
            &[],
        )
        .unwrap();
    note("cancel", &removed);
    assert_eq!(w.owner("2"), w.seller.to_string());

    w.mint_and_approve("3");
    w.create("3", 2_000);
    w.bid(&w.buyer.clone(), "3", 2_000).unwrap();
    let cancel_late = w
        .app
        .execute_contract(
            w.seller.clone(),
            w.auction.clone(),
            &ExecuteMsg::CancelAuction {
                collection: w.collection.to_string(),
                token_id: "3".into(),
            },
            &[],
        )
        .unwrap_err();
    assert!(cancel_late.to_string().contains("started"), "{cancel_late}");
}

#[test]
fn instant_sale_hook_and_end_block() {
    let mut w = world(None);
    w.mint_and_approve("9");
    let sent = w
        .app
        .execute_contract(
            w.seller.clone(),
            w.collection.clone(),
            &Cw721ExecuteMsg::<
                DefaultOptionalNftExtensionMsg,
                DefaultOptionalCollectionExtensionMsg,
                Empty,
            >::SendNft {
                contract: w.auction.to_string(),
                token_id: "9".into(),
                msg: to_json_binary(&HookMsg::CreateAuction {
                    denom: DENOM.into(),
                    reserve_price: Uint128::new(4_000),
                    duration: None,
                    is_instant_sale: true,
                })
                .unwrap(),
            },
            &[],
        )
        .unwrap();
    note("receive-nft", &sent);
    assert_eq!(w.owner("9"), w.auction.to_string());
    let over = w.bid(&w.buyer.clone(), "9", 4_001).unwrap_err();
    assert!(
        over.to_string().contains("equal the reserve"),
        "{over}"
    );
    let bought = w.bid(&w.buyer.clone(), "9", 4_000).unwrap();
    note("instant-settle", &bought);
    assert_eq!(w.owner("9"), w.buyer.to_string());
    // 2% of 4000 = 80, burn 40, fee manager 40, seller 3920. No royalty.
    assert_eq!(w.bal(&w.fee_manager), 40);
    assert_eq!(w.bal(&w.seller), 10_000 + 3_920);

    w.mint_and_approve("4");
    w.create("4", 1_000);
    w.bid(&w.buyer2.clone(), "4", 1_000).unwrap();
    w.app.update_block(|b| {
        b.time = b.time.plus_seconds(500);
        b.height += 5;
    });
    let sudo_res = w
        .app
        .wasm_sudo(w.auction.clone(), &SudoMsg::EndBlock {})
        .unwrap();
    note("sudo-end-block", &sudo_res);
    assert_eq!(w.owner("4"), w.buyer2.to_string());
}

#[cfg(feature = "interface")]
#[test]
fn cw_orch_mock_instantiates() {
    use cw_orch::prelude::*;
    let chain = Mock::new("reserve");
    let fee = chain.addr_make("fee");
    let auction = crate::RevenueReserveAuction::new(chain);
    auction.upload().unwrap();
    auction
        .instantiate(
            &InstantiateMsg {
                fee_manager: fee.to_string(),
                trading_fee_percent: Decimal::percent(2),
                min_bid_increment_percent: Decimal::percent(5),
                min_duration: 60,
                max_duration: 3600,
                extend_duration: 60,
                create_auction_fee: Coin::new(Uint256::zero(), DENOM),
                max_auctions_to_settle_per_block: 5,
                halt_duration_threshold: 600,
                halt_buffer_duration: 60,
                halt_postpone_duration: 120,
                burn_bps: 5_000,
                min_reserve_prices: vec![amount(100)],
            },
            None,
            &[],
        )
        .unwrap();
    let cfg: crate::state::Config = auction.query(&QueryMsg::Config {}).unwrap();
    assert_eq!(cfg.fee_manager, fee);
}
