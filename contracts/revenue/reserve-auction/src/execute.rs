use cosmwasm_std::{
    ensure, from_json, has_coins, Addr, Coin, DepsMut, Env, MessageInfo, Response, Uint256,
};
use cw_utils::{maybe_addr, must_pay, nonpayable, one_coin};

use crate::helpers::{assert_approval, fair_burn, only_owner, settle_auction, transfer_nft, validate_reserve};
use crate::msg::{Cw721ReceiveMsg, ExecuteMsg, HookMsg};
use crate::state::{auctions, Auction, HighBid, CONFIG, HALT_MANAGER};
use crate::ContractError;

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::CreateAuction {
            collection,
            token_id,
            reserve_price,
            duration,
            seller_funds_recipient,
            instant_sale,
        } => {
            let collection = deps.api.addr_validate(&collection)?;
            let recipient = maybe_addr(deps.api, seller_funds_recipient)?;
            create_auction(
                deps,
                env,
                info,
                collection,
                token_id,
                reserve_price,
                duration,
                recipient,
                instant_sale,
                false,
            )
        }
        ExecuteMsg::UpdateReservePrice {
            collection,
            token_id,
            reserve_price,
        } => {
            let collection = deps.api.addr_validate(&collection)?;
            update_reserve(deps, info, collection, token_id, reserve_price)
        }
        ExecuteMsg::CancelAuction {
            collection,
            token_id,
        } => {
            let collection = deps.api.addr_validate(&collection)?;
            cancel(deps, info, collection, token_id)
        }
        ExecuteMsg::PlaceBid {
            collection,
            token_id,
        } => {
            let collection = deps.api.addr_validate(&collection)?;
            place_bid(deps, env, info, collection, token_id)
        }
        ExecuteMsg::SettleAuction {
            collection,
            token_id,
        } => {
            let collection = deps.api.addr_validate(&collection)?;
            settle(deps, env, info, collection, token_id)
        }
        ExecuteMsg::ReceiveNft(msg) => receive_nft(deps, env, info, msg),
    }
}

#[allow(clippy::too_many_arguments)]
fn create_auction(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    collection: Addr,
    token_id: String,
    reserve_price: Coin,
    duration: u64,
    seller_funds_recipient: Option<Addr>,
    instant_sale: bool,
    already_escrowed: bool,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let seller = if already_escrowed {
        info.sender.clone()
    } else {
        only_owner(&deps.querier, &info.sender, &collection, &token_id)?;
        assert_approval(
            &deps.querier,
            &env.contract.address,
            &info.sender,
            &collection,
            &token_id,
        )?;
        info.sender.clone()
    };
    if auctions()
        .may_load(deps.storage, (collection.clone(), token_id.clone()))?
        .is_some()
    {
        return Err(ContractError::AuctionAlreadyExists);
    }
    ensure!(
        duration >= config.min_duration && duration <= config.max_duration,
        ContractError::InvalidDuration {
            min: config.min_duration,
            max: config.max_duration,
            got: duration
        }
    );
    validate_reserve(deps.storage, &reserve_price)?;

    let mut response = Response::new();
    if config.create_auction_fee.amount.is_zero() {
        nonpayable(&info)?;
    } else if already_escrowed {
        nonpayable(&info)?;
    } else {
        let paid = one_coin(&info)?;
        ensure!(
            paid == config.create_auction_fee,
            ContractError::WrongFee
        );
        response = response.add_messages(fair_burn(
            paid.amount,
            &paid.denom,
            config.burn_bps,
            &config.fee_manager,
        ));
    }

    let auction = Auction {
        collection: collection.clone(),
        token_id: token_id.clone(),
        seller,
        reserve_price,
        duration,
        end_time: None,
        seller_funds_recipient,
        high_bid: None,
        first_bid_time: None,
        instant_sale,
    };
    auctions().save(
        deps.storage,
        (collection.clone(), token_id.clone()),
        &auction,
    )?;
    if !already_escrowed {
        response = response.add_message(transfer_nft(&collection, &token_id, &env.contract.address)?);
    }
    Ok(response.add_event(
        cosmwasm_std::Event::new("create-auction")
            .add_attribute("collection", auction.collection.to_string())
            .add_attribute("token_id", auction.token_id)
            .add_attribute("seller", auction.seller.to_string())
            .add_attribute("reserve_price", auction.reserve_price.to_string())
            .add_attribute("instant_sale", auction.instant_sale.to_string()),
    ))
}

fn update_reserve(
    deps: DepsMut,
    info: MessageInfo,
    collection: Addr,
    token_id: String,
    reserve_price: Coin,
) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    let mut auction = auctions().load(deps.storage, (collection, token_id))?;
    ensure!(auction.seller == info.sender, ContractError::Unauthorized);
    ensure!(auction.first_bid_time.is_none(), ContractError::AuctionStarted);
    ensure!(
        auction.reserve_price.denom == reserve_price.denom,
        ContractError::InvalidInput("reserve denom cannot change".into())
    );
    validate_reserve(deps.storage, &reserve_price)?;
    auction.reserve_price = reserve_price;
    let key = (auction.collection.clone(), auction.token_id.clone());
    auctions().save(deps.storage, key, &auction)?;
    Ok(Response::new().add_event(
        cosmwasm_std::Event::new("update-reserve-price")
            .add_attribute("collection", auction.collection.to_string())
            .add_attribute("token_id", auction.token_id)
            .add_attribute("reserve_price", auction.reserve_price.to_string()),
    ))
}

fn cancel(
    deps: DepsMut,
    info: MessageInfo,
    collection: Addr,
    token_id: String,
) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    let auction = auctions().load(deps.storage, (collection.clone(), token_id.clone()))?;
    ensure!(auction.seller == info.sender, ContractError::Unauthorized);
    ensure!(auction.first_bid_time.is_none(), ContractError::AuctionStarted);
    auctions().remove(deps.storage, (collection.clone(), token_id.clone()))?;
    Ok(Response::new()
        .add_message(transfer_nft(&collection, &token_id, &auction.seller)?)
        .add_event(
            cosmwasm_std::Event::new("cancel-auction")
                .add_attribute("collection", collection.to_string())
                .add_attribute("token_id", token_id),
        ))
}

fn place_bid(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    collection: Addr,
    token_id: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let mut auction = auctions().load(deps.storage, (collection, token_id))?;
    ensure!(auction.seller != info.sender, ContractError::SellerShouldNotBid);
    let now = env.block.time;
    if let Some(end) = auction.end_time {
        ensure!(end.seconds() > now.seconds(), ContractError::AuctionEnded);
    }
    let paid = must_pay(&info, &auction.denom())?;
    let min = auction.min_bid_amount(config.min_bid_increment_percent);
    ensure!(
        has_coins(&[Coin::new(paid, &auction.denom())], &Coin::new(min, &auction.denom())),
        ContractError::BidTooLow
    );

    let mut response = Response::new();
    if auction.instant_sale && auction.high_bid.is_none() {
        ensure!(
            paid == auction.reserve_price.amount,
            ContractError::InstantSaleBidMustMatchReserve
        );
    }

    if let Some(prev) = auction.high_bid.clone() {
        response = response.add_message(cosmwasm_std::BankMsg::Send {
            to_address: prev.bidder.to_string(),
            amount: vec![prev.coin],
        });
        let remaining = auction
            .end_time
            .unwrap()
            .seconds()
            .saturating_sub(now.seconds());
        if remaining < config.extend_duration {
            auction.end_time = Some(now.plus_seconds(config.extend_duration));
        }
    } else {
        auction.first_bid_time = Some(now);
        auction.end_time = Some(if auction.instant_sale {
            now
        } else {
            now.plus_seconds(auction.duration)
        });
    }
    auction.high_bid = Some(HighBid {
        coin: Coin::new(paid, auction.denom()),
        bidder: info.sender.clone(),
    });
    let key = (auction.collection.clone(), auction.token_id.clone());
    auctions().save(deps.storage, key, &auction)?;
    response = response.add_event(
        cosmwasm_std::Event::new("place-bid")
            .add_attribute("collection", auction.collection.to_string())
            .add_attribute("token_id", auction.token_id.clone())
            .add_attribute("bidder", info.sender.to_string())
            .add_attribute("bid_amount", paid.to_string()),
    );

    if auction.instant_sale && auction.first_bid_time == Some(now) {
        let halt = HALT_MANAGER.load(deps.storage)?;
        let config = CONFIG.load(deps.storage)?;
        return settle_auction(deps, now, auction, &config, &halt, response);
    }
    Ok(response)
}

fn settle(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    collection: Addr,
    token_id: String,
) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    let config = CONFIG.load(deps.storage)?;
    let halt = HALT_MANAGER.load(deps.storage)?;
    let auction = auctions().load(deps.storage, (collection, token_id))?;
    settle_auction(deps, env.block.time, auction, &config, &halt, Response::new())
}

fn receive_nft(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: Cw721ReceiveMsg,
) -> Result<Response, ContractError> {
    let collection = info.sender.clone();
    let seller = deps.api.addr_validate(&msg.sender)?;
    let hook: HookMsg = from_json(&msg.msg)?;
    let HookMsg::CreateAuction {
        denom,
        reserve_price,
        duration,
        is_instant_sale,
    } = hook;
    let config = CONFIG.load(deps.storage)?;
    let duration = duration.unwrap_or(config.min_duration);
    // The hook message is executed by the collection, with empty funds.
    let seller_info = MessageInfo {
        sender: seller,
        funds: info.funds,
    };
    create_auction(
        deps.branch(),
        env,
        seller_info,
        collection,
        msg.token_id,
        Coin::new(Uint256::from(reserve_price), denom),
        duration,
        None,
        is_instant_sale,
        true,
    )
}
