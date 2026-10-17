use cosmwasm_std::{coin, coins, to_json_binary, Addr, Binary, Coin, Decimal, Empty, Uint256};
use cw721::msg::{Cw721ExecuteMsg, Cw721QueryMsg, OwnerOfResponse};
use cw_multi_test::{BasicApp, BankSudo, ContractWrapper, Executor, SudoMsg};
use nftfi_raffle::msg::{
    ExecuteMsg, InstantiateMsg, QueryMsg, RaffleOptionsMsg, RaffleResponse, TicketResponse,
};
use nftfi_randomness::{select_tickets, RandomnessSourceMsg, VerifierExecuteMsg};
use nftfi_utils::mock::InstantiateMsg as NftInit;
use nftfi_utils::AssetInfo;

const DENOM: &str = "uterp";

fn app() -> BasicApp {
    BasicApp::default()
}

fn fund(app: &mut BasicApp, addr: &Addr, amount: u128) {
    app.sudo(SudoMsg::Bank(BankSudo::Mint {
        to_address: addr.to_string(),
        amount: coins(amount, DENOM),
    }))
    .unwrap();
}

fn bal(app: &BasicApp, addr: &Addr) -> Uint256 {
    app.wrap().query_balance(addr.as_str(), DENOM).unwrap().amount
}

fn owner_of(app: &BasicApp, nft: &Addr, token_id: &str) -> String {
    let resp: OwnerOfResponse = app
        .wrap()
        .query_wasm_smart(
            nft.to_string(),
            &Cw721QueryMsg::<Empty, Empty, Empty>::OwnerOf {
                token_id: token_id.to_string(),
                include_expired: None,
            },
        )
        .unwrap();
    resp.owner
}

struct World {
    app: BasicApp,
    owner: Addr,
    alice: Addr,
    bob: Addr,
    beacon: Addr,
    fee: Addr,
    nft: Addr,
    raffle: Addr,
}

fn world(randomness: RandomnessSourceMsg, creation: u128, fee_pct: u64) -> World {
    let mut app = app();
    let owner = app.api().addr_make("owner");
    let alice = app.api().addr_make("alice");
    let bob = app.api().addr_make("bob");
    let beacon = app.api().addr_make("beacon");
    let fee = app.api().addr_make("fee");
    fund(&mut app, &owner, 1_000);
    fund(&mut app, &alice, 500);
    fund(&mut app, &bob, 500);

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
    let raffle_id = app.store_code(Box::new(
        ContractWrapper::new_with_empty(
            nftfi_raffle::execute,
            nftfi_raffle::instantiate,
            nftfi_raffle::query,
        )
        .with_reply_empty(nftfi_raffle::reply),
    ));
    let creation_coins = if creation == 0 {
        None
    } else {
        Some(vec![coin(creation, DENOM)])
    };
    let raffle = app
        .instantiate_contract(
            raffle_id,
            owner.clone(),
            &InstantiateMsg {
                name: "raffle".into(),
                owner: None,
                fee_addr: fee.to_string(),
                minimum_raffle_duration: Some(1),
                max_tickets_per_raffle: Some(100),
                raffle_fee: Decimal::percent(fee_pct),
                creation_coins,
                randomness,
            },
            &[],
            "raffle",
            None,
        )
        .unwrap();
    World {
        app,
        owner,
        alice,
        bob,
        beacon,
        fee,
        nft,
        raffle,
    }
}

fn mint(w: &mut World, token_id: &str, to: &Addr) {
    w.app
        .execute_contract(
            w.owner.clone(),
            w.nft.clone(),
            &Cw721ExecuteMsg::<Empty, Empty, Empty>::Mint {
                token_id: token_id.to_string(),
                owner: to.to_string(),
                token_uri: None,
                extension: Empty {},
            },
            &[],
        )
        .unwrap();
}

fn approve(w: &mut World, from: &Addr, token_id: &str) {
    w.app
        .execute_contract(
            from.clone(),
            w.nft.clone(),
            &Cw721ExecuteMsg::<Empty, Empty, Empty>::Approve {
                spender: w.raffle.to_string(),
                token_id: token_id.to_string(),
                expires: None,
            },
            &[],
        )
        .unwrap();
}

fn paid(amount: u128) -> Vec<Coin> {
    if amount == 0 {
        Vec::new()
    } else {
        coins(amount, DENOM)
    }
}

fn options(min: Option<u32>) -> RaffleOptionsMsg {
    RaffleOptionsMsg {
        start: None,
        duration: 100,
        comment: Some("pot".into()),
        max_ticket_number: Some(10),
        max_ticket_per_address: Some(4),
        min_ticket_number: min,
        one_winner_per_asset: false,
    }
}

fn create(w: &mut World, token_id: &str, price: u128, min: Option<u32>, fee: u128) -> u64 {
    let res = w
        .app
        .execute_contract(
            w.owner.clone(),
            w.raffle.clone(),
            &ExecuteMsg::CreateRaffle {
                owner: None,
                assets: vec![AssetInfo::cw721(w.nft.as_str(), token_id)],
                raffle_options: options(min),
                raffle_ticket_price: AssetInfo::coin(price, DENOM),
            },
            &paid(fee),
        )
        .unwrap();
    res.events
        .iter()
        .flat_map(|e| e.attributes.iter())
        .find(|a| a.key == "raffle_id")
        .unwrap()
        .value
        .parse()
        .unwrap()
}

fn buy(w: &mut World, buyer: &Addr, raffle_id: u64, count: u32, each: u128) {
    w.app
        .execute_contract(
            buyer.clone(),
            w.raffle.clone(),
            &ExecuteMsg::BuyTicket {
                raffle_id,
                ticket_count: count,
                on_behalf_of: None,
            },
            &coins(each * count as u128, DENOM),
        )
        .unwrap();
}

fn close(w: &mut World) {
    w.app.update_block(|b| {
        b.time = b.time.plus_seconds(101);
        b.height += 10;
    });
}

#[test]
fn create_buy_draw_claim() {
    let mut w = world(RandomnessSourceMsg::Seed { provider: None }, 100, 10);
    // addr_make("beacon") is not the string "beacon". Use the real address.
    w.app
        .execute_contract(
            w.owner.clone(),
            w.raffle.clone(),
            &ExecuteMsg::UpdateConfig {
                owner: None,
                fee_addr: None,
                minimum_raffle_duration: None,
                max_tickets_per_raffle: None,
                raffle_fee: None,
                creation_coins: None,
                randomness: Some(RandomnessSourceMsg::Seed {
                    provider: Some(w.beacon.to_string()),
                }),
            },
            &[],
        )
        .unwrap();

    let owner = w.owner.clone();
    mint(&mut w, "1", &owner);
    approve(&mut w, &owner, "1");
    let id = create(&mut w, "1", 10, None, 100);
    assert_eq!(owner_of(&w.app, &w.nft, "1"), w.raffle.to_string());

    let early = w.app.execute_contract(
        w.beacon.clone(),
        w.raffle.clone(),
        &ExecuteMsg::Draw {
            raffle_id: id,
            seed: Binary::from([9u8; 32]),
        },
        &[],
    );
    assert!(early.is_err());

    let alice = w.alice.clone();
    let bob = w.bob.clone();
    buy(&mut w, &alice, id, 1, 10);
    buy(&mut w, &bob, id, 1, 10);
    close(&mut w);
    w.app
        .execute_contract(
            w.beacon.clone(),
            w.raffle.clone(),
            &ExecuteMsg::Draw {
                raffle_id: id,
                seed: Binary::from([9u8; 32]),
            },
            &[],
        )
        .unwrap();
    w.app
        .execute_contract(
            w.alice.clone(),
            w.raffle.clone(),
            &ExecuteMsg::ClaimRaffle { raffle_id: id },
            &[],
        )
        .unwrap();

    let idx = select_tickets(&[9u8; 32], id, 2, 1)[0];
    let ticket: TicketResponse = w
        .app
        .wrap()
        .query_wasm_smart(
            w.raffle.to_string(),
            &QueryMsg::Ticket {
                raffle_id: id,
                index: idx,
            },
        )
        .unwrap();
    assert_eq!(owner_of(&w.app, &w.nft, "1"), ticket.owner);
    // 1000 - 100 creation + 18 of the 20 ticket pot (10% fee).
    assert_eq!(bal(&w.app, &w.owner), Uint256::from(918u128));
    assert_eq!(bal(&w.app, &w.fee), Uint256::from(102u128));
    let info: RaffleResponse = w
        .app
        .wrap()
        .query_wasm_smart(w.raffle.to_string(), &QueryMsg::Raffle { raffle_id: id })
        .unwrap();
    assert!(matches!(
        info.state,
        nftfi_raffle::state::RaffleState::Claimed
    ));
}

#[test]
fn cancel_before_sale() {
    let mut w = world(
        RandomnessSourceMsg::Seed { provider: None },
        100,
        10,
    );
    let owner = w.owner.clone();
    mint(&mut w, "1", &owner);
    approve(&mut w, &owner, "1");
    let id = create(&mut w, "1", 10, None, 100);
    w.app
        .execute_contract(
            w.owner.clone(),
            w.raffle.clone(),
            &ExecuteMsg::CancelRaffle { raffle_id: id },
            &[],
        )
        .unwrap();
    assert_eq!(owner_of(&w.app, &w.nft, "1"), w.owner.to_string());
    assert_eq!(bal(&w.app, &w.owner), Uint256::from(900u128));
    assert_eq!(bal(&w.app, &w.fee), Uint256::from(100u128));
}

#[test]
fn refund_when_below_min() {
    let mut w = world(RandomnessSourceMsg::Seed { provider: None }, 0, 10);
    let owner = w.owner.clone();
    let alice = w.alice.clone();
    mint(&mut w, "1", &owner);
    approve(&mut w, &owner, "1");
    let id = create(&mut w, "1", 10, Some(2), 0);
    buy(&mut w, &alice, id, 1, 10);
    close(&mut w);
    w.app
        .execute_contract(
            w.owner.clone(),
            w.raffle.clone(),
            &ExecuteMsg::ClaimRaffle { raffle_id: id },
            &[],
        )
        .unwrap();
    assert_eq!(owner_of(&w.app, &w.nft, "1"), w.owner.to_string());
    assert_eq!(bal(&w.app, &w.alice), Uint256::from(500u128));
}

#[test]
fn callback_draw() {
    let mut app = app();
    let owner = app.api().addr_make("owner");
    let alice = app.api().addr_make("alice");
    let fee = app.api().addr_make("fee");
    fund(&mut app, &alice, 100);
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
    let verifier_id = app.store_code(Box::new(ContractWrapper::new_with_empty(
        verifier_execute,
        verifier_instantiate,
        verifier_query,
    )));
    let verifier = app
        .instantiate_contract(
            verifier_id,
            owner.clone(),
            &Empty {},
            &[],
            "verifier",
            None,
        )
        .unwrap();
    let raffle_id = app.store_code(Box::new(
        ContractWrapper::new_with_empty(
            nftfi_raffle::execute,
            nftfi_raffle::instantiate,
            nftfi_raffle::query,
        )
        .with_reply_empty(nftfi_raffle::reply),
    ));
    let raffle = app
        .instantiate_contract(
            raffle_id,
            owner.clone(),
            &InstantiateMsg {
                name: "raffle".into(),
                owner: None,
                fee_addr: fee.to_string(),
                minimum_raffle_duration: Some(1),
                max_tickets_per_raffle: None,
                raffle_fee: Decimal::percent(10),
                creation_coins: None,
                randomness: RandomnessSourceMsg::Callback {
                    verifier: verifier.to_string(),
                },
            },
            &[],
            "raffle",
            None,
        )
        .unwrap();
    app.execute_contract(
        owner.clone(),
        nft.clone(),
        &Cw721ExecuteMsg::<Empty, Empty, Empty>::Mint {
            token_id: "1".into(),
            owner: owner.to_string(),
            token_uri: None,
            extension: Empty {},
        },
        &[],
    )
    .unwrap();
    app.execute_contract(
        owner.clone(),
        nft.clone(),
        &Cw721ExecuteMsg::<Empty, Empty, Empty>::Approve {
            spender: raffle.to_string(),
            token_id: "1".into(),
            expires: None,
        },
        &[],
    )
    .unwrap();
    app.execute_contract(
        owner.clone(),
        raffle.clone(),
        &ExecuteMsg::CreateRaffle {
            owner: None,
            assets: vec![AssetInfo::cw721(nft.as_str(), "1")],
            raffle_options: options(None),
            raffle_ticket_price: AssetInfo::coin(10, DENOM),
        },
        &[],
    )
    .unwrap();
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
    app.update_block(|b| {
        b.time = b.time.plus_seconds(101);
    });
    app.execute_contract(
        alice.clone(),
        raffle.clone(),
        &ExecuteMsg::RequestDraw { raffle_id: 0 },
        &[],
    )
    .unwrap();
    app.execute_contract(
        owner.clone(),
        raffle.clone(),
        &ExecuteMsg::ClaimRaffle { raffle_id: 0 },
        &[],
    )
    .unwrap();
    assert_eq!(owner_of(&app, &nft, "1"), alice.to_string());
    assert_eq!(bal(&app, &owner), Uint256::from(9u128));
    assert_eq!(bal(&app, &fee), Uint256::from(1u128));
}

#[test]
fn cw721_svg_messages_match() {
    let recipient = "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq2z7z6x".to_string();
    let token_id = "7".to_string();
    let plain = Cw721ExecuteMsg::<Empty, Empty, Empty>::TransferNft {
        recipient: recipient.clone(),
        token_id: token_id.clone(),
    };
    let svg = cw721_svg::ExecuteMsg::TransferNft {
        recipient: recipient.clone(),
        token_id: token_id.clone(),
    };
    assert_eq!(to_json_binary(&plain).unwrap(), to_json_binary(&svg).unwrap());
    let q_plain = Cw721QueryMsg::<Empty, Empty, Empty>::OwnerOf {
        token_id: token_id.clone(),
        include_expired: Some(true),
    };
    let q_svg = cw721_svg::QueryMsg::OwnerOf {
        token_id,
        include_expired: Some(true),
    };
    assert_eq!(to_json_binary(&q_plain).unwrap(), to_json_binary(&q_svg).unwrap());
    let _ = recipient;
}

fn verifier_instantiate(
    _deps: cosmwasm_std::DepsMut,
    _env: cosmwasm_std::Env,
    _info: cosmwasm_std::MessageInfo,
    _msg: Empty,
) -> Result<cosmwasm_std::Response, cosmwasm_std::StdError> {
    Ok(cosmwasm_std::Response::new())
}

fn verifier_execute(
    _deps: cosmwasm_std::DepsMut,
    _env: cosmwasm_std::Env,
    _info: cosmwasm_std::MessageInfo,
    msg: VerifierExecuteMsg,
) -> Result<cosmwasm_std::Response, cosmwasm_std::StdError> {
    let VerifierExecuteMsg::Provide { raffle_id, owner } = msg;
    Ok(cosmwasm_std::Response::new()
        .add_attribute("raffle_id", raffle_id.to_string())
        .add_attribute("round", "7")
        .add_attribute("randomness", Binary::from([4u8; 32]).to_base64())
        .add_attribute("owner", owner))
}

fn verifier_query(
    _deps: cosmwasm_std::Deps,
    _env: cosmwasm_std::Env,
    _msg: Empty,
) -> cosmwasm_std::StdResult<Binary> {
    Err(cosmwasm_std::StdError::msg("no query"))
}
