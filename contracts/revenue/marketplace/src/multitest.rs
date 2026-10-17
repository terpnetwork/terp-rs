//! Multi-test coverage for asks, bids, collection bids, royalties, and the svg-auction hook.
//!
//! This cw-multi-test fork does not meter wasm gas (`gas_used` is 0). The gas bench prints
//! event and attribute counts instead of inventing gas units.

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

use crate::msg::{ExecuteMsg, HookMsg, InstantiateMsg, OrderDetails, QueryMsg};
use crate::state::Config;
use crate::{execute, instantiate, query};

const DENOM: &str = "uterp";

fn coin_amount(amount: u128) -> Coin {
    Coin::new(Uint256::from(amount), DENOM)
}

fn note(action: &str, res: &AppResponse) {
    let attrs: usize = res.events.iter().map(|e| e.attributes.len()).sum();
    println!(
        "GAS marketplace {action} events={} attrs={} gas_used=unmetered",
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
    marketplace: Addr,
    collection: Addr,
    admin: Addr,
    seller: Addr,
    buyer: Addr,
    finder: Addr,
    royalty: Addr,
    fee_manager: Addr,
}

fn world(royalty_share: Option<Decimal>) -> World {
    let mut app = App::default();
    let admin = app.api().addr_make("admin");
    let seller = app.api().addr_make("seller");
    let buyer = app.api().addr_make("buyer");
    let finder = app.api().addr_make("finder");
    let royalty = app.api().addr_make("royalty");
    let fee_manager = app.api().addr_make("fee");
    app.init_modules(|router, _, storage| {
        router
            .bank
            .init_balance(storage, &buyer, coins(500_000, DENOM))
            .unwrap();
        router
            .bank
            .init_balance(storage, &seller, coins(50_000, DENOM))
            .unwrap();
    });

    let cw721_id = app.store_code(Box::new(
        ContractWrapper::new(cw721_execute, cw721_instantiate, cw721_query),
    ));
    let extension = royalty_share.map(|share| CollectionExtensionMsg {
        description: Some("revenue test".into()),
        image: Some("https://example.invalid/img.png".into()),
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
                name: "Test".into(),
                symbol: "TEST".into(),
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

    let market_id = app.store_code(Box::new(ContractWrapper::new_with_empty(
        execute, instantiate, query,
    )));
    let marketplace = app
        .instantiate_contract(
            market_id,
            admin.clone(),
            &InstantiateMsg {
                fee_manager: fee_manager.to_string(),
                royalty_registry: None,
                protocol_fee_bps: 200,
                max_royalty_fee_bps: 1_000,
                maker_reward_bps: 5_000,
                taker_reward_bps: 0,
                burn_bps: 5_000,
                default_denom: DENOM.into(),
            },
            &[],
            "revenue-marketplace",
            Some(admin.to_string()),
        )
        .unwrap();

    World {
        app,
        marketplace,
        collection,
        admin,
        seller,
        buyer,
        finder,
        royalty,
        fee_manager,
    }
}

impl World {
    fn mint(&mut self, token_id: &str) {
        self.app
            .execute_contract(
                self.seller.clone(),
                self.collection.clone(),
                &Cw721ExecuteMsg::<
                    DefaultOptionalNftExtensionMsg,
                    DefaultOptionalCollectionExtensionMsg,
                    Empty,
                >::Mint {
                    token_id: token_id.into(),
                    owner: self.seller.to_string(),
                    token_uri: None,
                    extension: None,
                },
                &[],
            )
            .unwrap();
    }

    fn approve_market(&mut self) {
        self.app
            .execute_contract(
                self.seller.clone(),
                self.collection.clone(),
                &Cw721ExecuteMsg::<
                    DefaultOptionalNftExtensionMsg,
                    DefaultOptionalCollectionExtensionMsg,
                    Empty,
                >::ApproveAll {
                    operator: self.marketplace.to_string(),
                    expires: None,
                },
                &[],
            )
            .unwrap();
    }

    fn exec(
        &mut self,
        sender: &Addr,
        msg: &ExecuteMsg,
        funds: &[Coin],
    ) -> Result<AppResponse, cosmwasm_std::StdError> {
        let sender = sender.clone();
        let market = self.marketplace.clone();
        self.app.execute_contract(sender, market, msg, funds)
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

    fn details(price: u128, finder: Option<&Addr>) -> OrderDetails {
        OrderDetails {
            price: coin_amount(price),
            recipient: None,
            finder: finder.map(|a| a.to_string()),
        }
    }
}

#[test]
fn list_buy_cancel_and_fees() {
    let mut w = world(None);
    w.mint("1");
    w.mint("2");
    w.approve_market();

    let listed = w
        .exec(
            &w.seller.clone(),
            &ExecuteMsg::SetAsk {
                collection: w.collection.to_string(),
                token_id: "1".into(),
                details: World::details(10_000, Some(&w.finder.clone())),
            },
            &[],
        )
        .unwrap();
    note("set-ask", &listed);
    assert_eq!(w.owner("1"), w.marketplace.to_string());

    let err = w
        .exec(
            &w.buyer.clone(),
            &ExecuteMsg::RemoveAsk {
                id: crate::helpers::generate_id(&[w.collection.as_bytes(), b"1"]),
            },
            &[],
        )
        .unwrap_err();
    assert!(err.to_string().contains("Unauthorized"), "{err}");

    let bought = w
        .exec(
            &w.buyer.clone(),
            &ExecuteMsg::AcceptAsk {
                id: crate::helpers::generate_id(&[w.collection.as_bytes(), b"1"]),
                details: World::details(10_000, None),
            },
            &[coin_amount(10_000)],
        )
        .unwrap();
    note("accept-ask", &bought);
    assert_eq!(w.owner("1"), w.buyer.to_string());
    // protocol 200, finder takes half of that (100), burn half of the remaining 100.
    assert_eq!(w.bal(&w.seller), 50_000 + 9_800);
    assert_eq!(w.bal(&w.finder), 100);
    assert_eq!(w.bal(&w.fee_manager), 50);
    assert_eq!(w.bal(&w.buyer), 500_000 - 10_000);

    let under = w
        .exec(
            &w.buyer.clone(),
            &ExecuteMsg::AcceptAsk {
                id: "missing".into(),
                details: World::details(1, None),
            },
            &[coin_amount(1)],
        )
        .unwrap_err();
    assert!(under.to_string().contains("ask not found"), "{under}");

    let zero = w
        .exec(
            &w.seller.clone(),
            &ExecuteMsg::SetAsk {
                collection: w.collection.to_string(),
                token_id: "2".into(),
                details: World::details(0, None),
            },
            &[],
        )
        .unwrap_err();
    assert!(zero.to_string().contains("greater than 0"), "{zero}");

    let listed = w
        .exec(
            &w.seller.clone(),
            &ExecuteMsg::SetAsk {
                collection: w.collection.to_string(),
                token_id: "2".into(),
                details: World::details(4_000, None),
            },
            &[],
        )
        .unwrap();
    note("set-ask-2", &listed);
    let removed = w
        .exec(
            &w.seller.clone(),
            &ExecuteMsg::RemoveAsk {
                id: crate::helpers::generate_id(&[w.collection.as_bytes(), b"2"]),
            },
            &[],
        )
        .unwrap();
    note("remove-ask", &removed);
    assert_eq!(w.owner("2"), w.seller.to_string());
}

#[test]
fn bid_accept_collection_bid_and_royalty() {
    let mut w = world(Some(Decimal::percent(5)));
    w.mint("7");
    w.mint("8");
    w.approve_market();

    let bid = w
        .exec(
            &w.buyer.clone(),
            &ExecuteMsg::SetBid {
                collection: w.collection.to_string(),
                token_id: "7".into(),
                details: World::details(10_000, None),
            },
            &[coin_amount(10_000)],
        )
        .unwrap();
    note("set-bid", &bid);

    let cfg: Config = w
        .app
        .wrap()
        .query_wasm_smart(&w.marketplace, &QueryMsg::Config {})
        .unwrap();
    assert_eq!(cfg.protocol_fee_bps, 200);

    let accepted = w
        .exec(
            &w.seller.clone(),
            &ExecuteMsg::AcceptBid {
                id: event_attr(&bid, "id"),
                details: World::details(1, None),
            },
            &[],
        )
        .unwrap();
    note("accept-bid", &accepted);
    assert_eq!(w.owner("7"), w.buyer.to_string());
    // 5% royalty = 500, protocol 200 with burn 100 and fee manager 100, seller 9300.
    assert_eq!(w.bal(&w.royalty), 500);
    assert_eq!(w.bal(&w.fee_manager), 100);
    assert_eq!(w.bal(&w.seller), 50_000 + 9_300);

    let coll = w
        .exec(
            &w.buyer.clone(),
            &ExecuteMsg::SetCollectionBid {
                collection: w.collection.to_string(),
                details: World::details(8_000, None),
            },
            &[coin_amount(8_000)],
        )
        .unwrap();
    note("set-collection-bid", &coll);
    let sold = w
        .exec(
            &w.seller.clone(),
            &ExecuteMsg::SetAsk {
                collection: w.collection.to_string(),
                token_id: "8".into(),
                details: World::details(8_000, None),
            },
            &[],
        )
        .unwrap();
    note("set-ask-match-collection", &sold);
    assert_eq!(w.owner("8"), w.buyer.to_string());

    let no_match = w
        .exec(
            &w.buyer.clone(),
            &ExecuteMsg::BuySpecificNft {
                collection: w.collection.to_string(),
                token_id: "missing".into(),
                details: World::details(1, None),
            },
            &[coin_amount(1)],
        )
        .unwrap_err();
    assert!(no_match.to_string().contains("No match"), "{no_match}");
}

#[test]
fn receive_nft_instant_sale_and_timed_rejection() {
    let mut w = world(None);
    w.mint("9");
    w.mint("10");

    let timed = w
        .app
        .execute_contract(
            w.seller.clone(),
            w.collection.clone(),
            &Cw721ExecuteMsg::<
                DefaultOptionalNftExtensionMsg,
                DefaultOptionalCollectionExtensionMsg,
                Empty,
            >::SendNft {
                contract: w.marketplace.to_string(),
                token_id: "9".into(),
                msg: to_json_binary(&HookMsg::CreateAuction {
                    denom: DENOM.into(),
                    reserve_price: Uint128::new(1_000),
                    is_instant_sale: false,
                })
                .unwrap(),
            },
            &[],
        )
        .unwrap_err();
    assert!(
        timed.to_string().contains("revenue-reserve-auction"),
        "{timed}"
    );
    // SendNft transfers before the hook. A hook error rolls the subcall back, so the seller keeps the NFT.
    assert_eq!(w.owner("9"), w.seller.to_string());

    let instant = w
        .app
        .execute_contract(
            w.seller.clone(),
            w.collection.clone(),
            &Cw721ExecuteMsg::<
                DefaultOptionalNftExtensionMsg,
                DefaultOptionalCollectionExtensionMsg,
                Empty,
            >::SendNft {
                contract: w.marketplace.to_string(),
                token_id: "10".into(),
                msg: to_json_binary(&HookMsg::CreateAuction {
                    denom: DENOM.into(),
                    reserve_price: Uint128::new(2_500),
                    is_instant_sale: true,
                })
                .unwrap(),
            },
            &[],
        )
        .unwrap();
    note("receive-nft-instant", &instant);
    assert_eq!(w.owner("10"), w.marketplace.to_string());

    let bought = w
        .exec(
            &w.buyer.clone(),
            &ExecuteMsg::AcceptAsk {
                id: crate::helpers::generate_id(&[w.collection.as_bytes(), b"10"]),
                details: World::details(2_500, None),
            },
            &[coin_amount(2_500)],
        )
        .unwrap();
    note("buy-instant-ask", &bought);
    assert_eq!(w.owner("10"), w.buyer.to_string());
}

#[test]
fn listing_fee_and_stranger_cannot_list() {
    let mut w = world(None);
    w.mint("3");
    let stranger = w.app.api().addr_make("stranger");
    let denied = w
        .exec(
            &stranger,
            &ExecuteMsg::SetAsk {
                collection: w.collection.to_string(),
                token_id: "3".into(),
                details: World::details(100, None),
            },
            &[],
        )
        .unwrap_err();
    assert!(denied.to_string().contains("Unauthorized"), "{denied}");

    w.approve_market();
    w.exec(
        &w.admin.clone(),
        &ExecuteMsg::SetListingFee {
            fee: coin_amount(25),
        },
        &[],
    )
    .unwrap();
    let missing_fee = w
        .exec(
            &w.seller.clone(),
            &ExecuteMsg::SetAsk {
                collection: w.collection.to_string(),
                token_id: "3".into(),
                details: World::details(100, None),
            },
            &[],
        )
        .unwrap_err();
    assert!(
        missing_fee.to_string().contains("No funds") || missing_fee.to_string().contains("funds"),
        "{missing_fee}"
    );
    let listed = w
        .exec(
            &w.seller.clone(),
            &ExecuteMsg::SetAsk {
                collection: w.collection.to_string(),
                token_id: "3".into(),
                details: World::details(100, None),
            },
            &[coin_amount(25)],
        )
        .unwrap();
    note("set-ask-with-fee", &listed);
    // burn_bps 5000 of the 25 listing fee: 13 burned (ceil) and the rest to the fee manager.
    assert!(w.bal(&w.fee_manager) > 0);
    assert_eq!(w.owner("3"), w.marketplace.to_string());
}

fn event_attr(res: &AppResponse, key: &str) -> String {
    res.events
        .iter()
        .flat_map(|e| e.attributes.iter())
        .find(|a| a.key == key)
        .unwrap_or_else(|| panic!("missing attr {key}"))
        .value
        .clone()
}

#[cfg(feature = "interface")]
#[test]
fn cw_orch_mock_instantiates() {
    use cw_orch::prelude::*;

    let chain = Mock::new("revenue");
    let fee = chain.addr_make("fee");
    let market = crate::RevenueMarketplace::new(chain);
    market.upload().unwrap();
    market
        .instantiate(
            &InstantiateMsg {
                fee_manager: fee.to_string(),
                royalty_registry: None,
                protocol_fee_bps: 200,
                max_royalty_fee_bps: 1_000,
                maker_reward_bps: 0,
                taker_reward_bps: 0,
                burn_bps: 0,
                default_denom: DENOM.into(),
            },
            None,
            &[],
        )
        .unwrap();
    let cfg: Config = market.query(&QueryMsg::Config {}).unwrap();
    assert_eq!(cfg.fee_manager, fee);
}
