use cosmwasm_std::{
    ensure, ensure_eq, from_json, has_coins, Addr, Coin, DepsMut, Env, MessageInfo, Response,
    Uint256,
};
use cw_utils::{nonpayable, one_coin, NativeBalance};
use std::ops::{Add, Sub};

use crate::helpers::{
    assert_transfer_approval, fair_burn, finalize_sale, generate_id, only_contract_admin,
    only_owner, only_tradable, only_valid_price, transfer_nft,
};
use crate::msg::{Cw721ReceiveMsg, ExecuteMsg, HookMsg, InstantiateMsg, OrderDetails};
use crate::orders::{Ask, Bid, CollectionBid, MatchingBid, OrderTerms};
use crate::state::{
    asks, bids, collection_bids, Config, Denom, OrderId, TokenId, COLLECTION_DENOMS, CONFIG,
    LISTING_FEES, NONCE,
};
use crate::ContractError;

const ORDER_KEYS: &[&str] = &[
    "id",
    "creator",
    "collection",
    "token_id",
    "price",
    "recipient",
    "finder",
];

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    let api = deps.api;
    match msg {
        ExecuteMsg::UpdateConfig { config } => update_config(deps, &env, &info, config),
        ExecuteMsg::UpdateCollectionDenom { collection, denom } => {
            update_collection_denom(deps, &env, &info, api.addr_validate(&collection)?, denom)
        }
        ExecuteMsg::SetListingFee { fee } => set_listing_fee(deps, &env, &info, fee),
        ExecuteMsg::RemoveListingFee { denom } => remove_listing_fee(deps, &env, &info, denom),
        ExecuteMsg::SetAsk {
            collection,
            token_id,
            details,
        } => set_ask(
            deps,
            env,
            info,
            api.addr_validate(&collection)?,
            token_id,
            details.validate(api)?,
            false,
            None,
        ),
        ExecuteMsg::UpdateAsk { id, details } => {
            update_ask(deps, env, info, id, details.validate(api)?)
        }
        ExecuteMsg::RemoveAsk { id } => remove_ask(deps, info, id),
        ExecuteMsg::AcceptAsk { id, details } => {
            accept_ask(deps, env, info, id, details.validate(api)?)
        }
        ExecuteMsg::SetBid {
            collection,
            token_id,
            details,
        } => set_bid(
            deps,
            env,
            info,
            api.addr_validate(&collection)?,
            token_id,
            details.validate(api)?,
            false,
        ),
        ExecuteMsg::UpdateBid { id, details } => {
            update_bid(deps, env, info, id, details.validate(api)?)
        }
        ExecuteMsg::RemoveBid { id } => remove_bid(deps, info, id),
        ExecuteMsg::AcceptBid { id, details } => {
            accept_bid(deps, env, info, id, details.validate(api)?)
        }
        ExecuteMsg::SetCollectionBid {
            collection,
            details,
        } => set_collection_bid(
            deps,
            env,
            info,
            api.addr_validate(&collection)?,
            details.validate(api)?,
            false,
        ),
        ExecuteMsg::UpdateCollectionBid { id, details } => {
            update_collection_bid(deps, env, info, id, details.validate(api)?)
        }
        ExecuteMsg::RemoveCollectionBid { id } => remove_collection_bid(deps, info, id),
        ExecuteMsg::AcceptCollectionBid {
            id,
            token_id,
            details,
        } => accept_collection_bid(deps, env, info, id, token_id, details.validate(api)?),
        ExecuteMsg::SellNft {
            collection,
            token_id,
            details,
        } => set_ask(
            deps,
            env,
            info,
            api.addr_validate(&collection)?,
            token_id,
            details.validate(api)?,
            true,
            None,
        ),
        ExecuteMsg::BuySpecificNft {
            collection,
            token_id,
            details,
        } => set_bid(
            deps,
            env,
            info,
            api.addr_validate(&collection)?,
            token_id,
            details.validate(api)?,
            true,
        ),
        ExecuteMsg::BuyCollectionNft {
            collection,
            details,
        } => set_collection_bid(
            deps,
            env,
            info,
            api.addr_validate(&collection)?,
            details.validate(api)?,
            true,
        ),
        ExecuteMsg::ReceiveNft(msg) => receive_nft(deps, env, info, msg),
    }
}

fn update_config(
    deps: DepsMut,
    env: &Env,
    info: &MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    only_contract_admin(&deps.querier, env, &info.sender)?;
    nonpayable(info)?;
    let config = Config {
        fee_manager: deps.api.addr_validate(&msg.fee_manager)?,
        royalty_registry: msg
            .royalty_registry
            .as_ref()
            .map(|addr| deps.api.addr_validate(addr))
            .transpose()?,
        protocol_fee_bps: msg.protocol_fee_bps,
        max_royalty_fee_bps: msg.max_royalty_fee_bps,
        maker_reward_bps: msg.maker_reward_bps,
        taker_reward_bps: msg.taker_reward_bps,
        burn_bps: msg.burn_bps,
        default_denom: msg.default_denom,
    };
    config.save(deps.storage)?;
    Ok(Response::new().add_attribute("action", "update-config"))
}

fn update_collection_denom(
    deps: DepsMut,
    env: &Env,
    info: &MessageInfo,
    collection: Addr,
    denom: Denom,
) -> Result<Response, ContractError> {
    only_contract_admin(&deps.querier, env, &info.sender)?;
    nonpayable(info)?;
    COLLECTION_DENOMS.save(deps.storage, collection.clone(), &denom)?;
    Ok(Response::new()
        .add_attribute("action", "set-collection-denom")
        .add_attribute("collection", collection)
        .add_attribute("denom", denom))
}

fn set_listing_fee(
    deps: DepsMut,
    env: &Env,
    info: &MessageInfo,
    fee: Coin,
) -> Result<Response, ContractError> {
    only_contract_admin(&deps.querier, env, &info.sender)?;
    nonpayable(info)?;
    ensure!(
        !fee.amount.is_zero(),
        ContractError::InvalidInput("listing fee must be greater than 0".into())
    );
    LISTING_FEES.save(deps.storage, fee.denom.clone(), &fee.amount)?;
    Ok(Response::new()
        .add_attribute("action", "set-listing-fee")
        .add_attribute("fee", fee.to_string()))
}

fn remove_listing_fee(
    deps: DepsMut,
    env: &Env,
    info: &MessageInfo,
    denom: Denom,
) -> Result<Response, ContractError> {
    only_contract_admin(&deps.querier, env, &info.sender)?;
    nonpayable(info)?;
    LISTING_FEES.remove(deps.storage, denom.clone());
    Ok(Response::new()
        .add_attribute("action", "remove-listing-fee")
        .add_attribute("denom", denom))
}

fn charge_listing_fee(
    deps: &DepsMut,
    info: &MessageInfo,
    price_denom: &str,
    config: &Config,
    response: Response,
) -> Result<Response, ContractError> {
    let configured = LISTING_FEES.may_load(deps.storage, price_denom.to_string())?;
    match configured {
        Some(fee) if !fee.is_zero() => {
            let paid = one_coin(info)?;
            ensure_eq!(
                paid,
                coin_u256(fee, price_denom),
                ContractError::InvalidInput("payment does not match listing fee".into())
            );
            Ok(response.add_messages(fair_burn(
                paid.amount,
                &paid.denom,
                config.burn_bps,
                &config.fee_manager,
            )))
        }
        _ => {
            nonpayable(info)?;
            Ok(response)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn set_ask(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    collection: Addr,
    token_id: TokenId,
    terms: OrderTerms,
    sell_now: bool,
    escrowed_by: Option<Addr>,
) -> Result<Response, ContractError> {
    let creator = if let Some(creator) = escrowed_by {
        creator
    } else {
        only_owner(&deps.querier, &info.sender, &collection, &token_id)?;
        assert_transfer_approval(
            &deps.querier,
            &env.contract.address,
            &info.sender,
            &collection,
            &token_id,
        )?;
        info.sender.clone()
    };
    only_tradable(&deps.querier, env.block.time, &collection)?;
    let config = CONFIG.load(deps.storage)?;
    only_valid_price(deps.storage, &config, &collection, &terms.price, None)?;
    let mut response = charge_listing_fee(&deps, &info, &terms.price.denom, &config, Response::new())?;
    let ask = Ask::new(creator, collection, token_id, terms);
    if let Some(matching) = ask.match_with_bid(deps.as_ref())? {
        response = finalize_sale(
            deps,
            &env,
            &ask,
            &config,
            &matching,
            false,
            "set-ask-match",
            response,
        )?;
    } else if sell_now {
        return Err(ContractError::NoMatchFound);
    } else {
        if escrowed_by_missing(&info, &ask) {
            response = transfer_nft(&ask.collection, &ask.token_id, &env.contract.address, response)?;
        }
        response = response.add_event(
            cosmwasm_std::Event::new("set-ask").add_attributes(ask.event_attrs(ORDER_KEYS)),
        );
        asks().update(deps.storage, ask.id.clone(), |existing| match existing {
            Some(_) => Err(ContractError::InternalError("ask id collision".into())),
            None => Ok(ask),
        })?;
    }
    Ok(response)
}

fn escrowed_by_missing(info: &MessageInfo, ask: &Ask) -> bool {
    info.sender != ask.collection
}

fn update_ask(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    id: OrderId,
    terms: OrderTerms,
) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    let config = CONFIG.load(deps.storage)?;
    let mut ask = asks()
        .load(deps.storage, id.clone())
        .map_err(|_| ContractError::InvalidInput(format!("ask not found [{id}]")))?;
    ensure_eq!(
        info.sender,
        ask.creator,
        ContractError::Unauthorized("only the ask creator can update it".into())
    );
    only_valid_price(deps.storage, &config, &ask.collection, &terms.price, None)?;
    ask.terms = terms;
    let mut response = Response::new();
    if let Some(matching) = ask.match_with_bid(deps.as_ref())? {
        response = finalize_sale(deps, &env, &ask, &config, &matching, false, "update-ask", response)?;
    } else {
        ask.save(deps.storage)?;
        response = response.add_event(
            cosmwasm_std::Event::new("update-ask").add_attributes(ask.event_attrs(ORDER_KEYS)),
        );
    }
    Ok(response)
}

fn remove_ask(deps: DepsMut, info: MessageInfo, id: OrderId) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    let ask = asks()
        .load(deps.storage, id.clone())
        .map_err(|_| ContractError::InvalidInput(format!("ask not found [{id}]")))?;
    ensure_eq!(
        info.sender,
        ask.creator,
        ContractError::Unauthorized("only the ask creator can remove it".into())
    );
    let response = transfer_nft(
        &ask.collection,
        &ask.token_id,
        &ask.asset_recipient(),
        Response::new(),
    )?;
    ask.remove(deps.storage)?;
    Ok(response.add_event(
        cosmwasm_std::Event::new("remove-ask").add_attributes(ask.event_attrs(&["id", "collection", "token_id"])),
    ))
}

fn accept_ask(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    id: OrderId,
    mut terms: OrderTerms,
) -> Result<Response, ContractError> {
    let ask = asks()
        .load(deps.storage, id.clone())
        .map_err(|_| ContractError::InvalidInput(format!("ask not found [{id}]")))?;
    ensure!(
        has_coins(&[terms.price.clone()], &ask.terms.price),
        ContractError::InvalidInput("ask price is greater than max price".into())
    );
    let mut funds = NativeBalance(info.funds.clone());
    funds.normalize();
    funds = funds
        .sub(ask.terms.price.clone())
        .map_err(|_| ContractError::InsufficientFunds)?;
    let nonce = next_nonce(deps.storage)?;
    terms.price = ask.terms.price.clone();
    let bid = Bid::new(
        info.sender.clone(),
        ask.collection.clone(),
        ask.token_id.clone(),
        terms,
        env.block.height,
        nonce,
    );
    let config = CONFIG.load(deps.storage)?;
    let mut response = finalize_sale(
        deps,
        &env,
        &ask,
        &config,
        &MatchingBid::Bid(bid),
        true,
        "accept-ask",
        Response::new(),
    )?;
    if !funds.is_empty() {
        response = response.add_message(cosmwasm_std::BankMsg::Send {
            to_address: info.sender.to_string(),
            amount: funds.into_vec(),
        });
    }
    Ok(response)
}

fn set_bid(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    collection: Addr,
    token_id: TokenId,
    terms: OrderTerms,
    buy_now: bool,
) -> Result<Response, ContractError> {
    only_tradable(&deps.querier, env.block.time, &collection)?;
    let config = CONFIG.load(deps.storage)?;
    let mut funds = NativeBalance(info.funds.clone());
    funds.normalize();
    let nonce = next_nonce(deps.storage)?;
    let bid = Bid::new(
        info.sender.clone(),
        collection.clone(),
        token_id,
        terms.clone(),
        env.block.height,
        nonce,
    );
    let mut response = Response::new();
    if let Some(ask) = bid.match_with_ask(deps.as_ref())? {
        only_valid_price(
            deps.storage,
            &config,
            &collection,
            &terms.price,
            Some(&ask.terms.price.denom),
        )?;
        funds = funds
            .sub(ask.terms.price.clone())
            .map_err(|_| ContractError::InsufficientFunds)?;
        let action = if buy_now { "buy-now" } else { "set-bid-match" };
        response = finalize_sale(
            deps,
            &env,
            &ask,
            &config,
            &MatchingBid::Bid(bid),
            true,
            action,
            response,
        )?;
    } else if buy_now {
        return Err(ContractError::NoMatchFound);
    } else {
        only_valid_price(deps.storage, &config, &collection, &terms.price, None)?;
        funds = funds
            .sub(bid.terms.price.clone())
            .map_err(|_| ContractError::InsufficientFunds)?;
        response = response.add_event(
            cosmwasm_std::Event::new("set-bid").add_attributes(bid.event_attrs(ORDER_KEYS)),
        );
        bids().update(deps.storage, bid.id.clone(), |existing| match existing {
            Some(_) => Err(ContractError::InternalError("bid id collision".into())),
            None => Ok(bid),
        })?;
    }
    if !funds.is_empty() {
        response = response.add_message(cosmwasm_std::BankMsg::Send {
            to_address: info.sender.to_string(),
            amount: funds.into_vec(),
        });
    }
    Ok(response)
}

fn update_bid(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    id: OrderId,
    terms: OrderTerms,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let mut bid = bids()
        .load(deps.storage, id.clone())
        .map_err(|_| ContractError::InvalidInput(format!("bid not found [{id}]")))?;
    ensure_eq!(
        info.sender,
        bid.creator,
        ContractError::Unauthorized("only the bid creator can update it".into())
    );
    only_valid_price(deps.storage, &config, &bid.collection, &terms.price, None)?;
    let mut funds = NativeBalance(info.funds.clone());
    funds.normalize();
    funds = funds.add(bid.terms.price.clone());
    bid.terms = terms;
    let mut response = Response::new();
    if let Some(ask) = bid.match_with_ask(deps.as_ref())? {
        funds = funds
            .sub(ask.terms.price.clone())
            .map_err(|_| ContractError::InsufficientFunds)?;
        response = finalize_sale(
            deps,
            &env,
            &ask,
            &config,
            &MatchingBid::Bid(bid),
            true,
            "update-bid",
            response,
        )?;
    } else {
        funds = funds
            .sub(bid.terms.price.clone())
            .map_err(|_| ContractError::InsufficientFunds)?;
        bid.save(deps.storage)?;
        response = response.add_event(
            cosmwasm_std::Event::new("update-bid").add_attributes(bid.event_attrs(ORDER_KEYS)),
        );
    }
    if !funds.is_empty() {
        response = response.add_message(cosmwasm_std::BankMsg::Send {
            to_address: info.sender.to_string(),
            amount: funds.into_vec(),
        });
    }
    Ok(response)
}

fn remove_bid(deps: DepsMut, info: MessageInfo, id: OrderId) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    let bid = bids()
        .load(deps.storage, id.clone())
        .map_err(|_| ContractError::InvalidInput(format!("bid not found [{id}]")))?;
    ensure_eq!(
        info.sender,
        bid.creator,
        ContractError::Unauthorized("only the bid creator can remove it".into())
    );
    let refund = bid.terms.price.clone();
    bid.remove(deps.storage)?;
    Ok(Response::new()
        .add_message(cosmwasm_std::BankMsg::Send {
            to_address: info.sender.to_string(),
            amount: vec![refund],
        })
        .add_event(
            cosmwasm_std::Event::new("remove-bid")
                .add_attributes(bid.event_attrs(&["id", "collection", "token_id"])),
        ))
}

fn accept_bid(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    id: OrderId,
    terms: OrderTerms,
) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    let bid = bids()
        .load(deps.storage, id.clone())
        .map_err(|_| ContractError::InvalidInput(format!("bid not found [{id}]")))?;
    ensure!(
        has_coins(&[bid.terms.price.clone()], &terms.price),
        ContractError::InvalidInput("min output is greater than bid price".into())
    );
    let ask = existing_or_new_ask(deps.as_ref(), &info, &env, &bid.collection, &bid.token_id, terms)?;
    let config = CONFIG.load(deps.storage)?;
    finalize_sale(
        deps,
        &env,
        &ask,
        &config,
        &MatchingBid::Bid(bid),
        false,
        "accept-bid",
        Response::new(),
    )
}

fn set_collection_bid(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    collection: Addr,
    terms: OrderTerms,
    buy_now: bool,
) -> Result<Response, ContractError> {
    only_tradable(&deps.querier, env.block.time, &collection)?;
    let config = CONFIG.load(deps.storage)?;
    only_valid_price(deps.storage, &config, &collection, &terms.price, None)?;
    let mut funds = NativeBalance(info.funds.clone());
    funds.normalize();
    let nonce = next_nonce(deps.storage)?;
    let collection_bid = CollectionBid::new(info.sender.clone(), collection, terms, env.block.height, nonce);
    let mut response = Response::new();
    if let Some(ask) = collection_bid.match_with_ask(deps.as_ref())? {
        funds = funds
            .sub(ask.terms.price.clone())
            .map_err(|_| ContractError::InsufficientFunds)?;
        response = finalize_sale(
            deps,
            &env,
            &ask,
            &config,
            &MatchingBid::CollectionBid(collection_bid),
            true,
            "set-collection-bid",
            response,
        )?;
    } else if buy_now {
        return Err(ContractError::NoMatchFound);
    } else {
        funds = funds
            .sub(collection_bid.terms.price.clone())
            .map_err(|_| ContractError::InsufficientFunds)?;
        response = response.add_event(
            cosmwasm_std::Event::new("set-collection-bid")
                .add_attributes(collection_bid.event_attrs(ORDER_KEYS)),
        );
        collection_bids().update(deps.storage, collection_bid.id.clone(), |existing| {
            match existing {
                Some(_) => Err(ContractError::InternalError(
                    "collection bid id collision".into(),
                )),
                None => Ok(collection_bid),
            }
        })?;
    }
    if !funds.is_empty() {
        response = response.add_message(cosmwasm_std::BankMsg::Send {
            to_address: info.sender.to_string(),
            amount: funds.into_vec(),
        });
    }
    Ok(response)
}

fn update_collection_bid(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    id: OrderId,
    terms: OrderTerms,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let mut collection_bid = collection_bids()
        .load(deps.storage, id.clone())
        .map_err(|_| ContractError::InvalidInput(format!("collection bid not found [{id}]")))?;
    ensure_eq!(
        info.sender,
        collection_bid.creator,
        ContractError::Unauthorized("only the collection bid creator can update it".into())
    );
    only_valid_price(
        deps.storage,
        &config,
        &collection_bid.collection,
        &terms.price,
        None,
    )?;
    let mut funds = NativeBalance(info.funds.clone());
    funds.normalize();
    funds = funds.add(collection_bid.terms.price.clone());
    collection_bid.terms = terms;
    let mut response = Response::new();
    if let Some(ask) = collection_bid.match_with_ask(deps.as_ref())? {
        funds = funds
            .sub(ask.terms.price.clone())
            .map_err(|_| ContractError::InsufficientFunds)?;
        response = finalize_sale(
            deps,
            &env,
            &ask,
            &config,
            &MatchingBid::CollectionBid(collection_bid),
            true,
            "update-collection-bid",
            response,
        )?;
    } else {
        funds = funds
            .sub(collection_bid.terms.price.clone())
            .map_err(|_| ContractError::InsufficientFunds)?;
        collection_bid.save(deps.storage)?;
        response = response.add_event(
            cosmwasm_std::Event::new("update-collection-bid")
                .add_attributes(collection_bid.event_attrs(ORDER_KEYS)),
        );
    }
    if !funds.is_empty() {
        response = response.add_message(cosmwasm_std::BankMsg::Send {
            to_address: info.sender.to_string(),
            amount: funds.into_vec(),
        });
    }
    Ok(response)
}

fn remove_collection_bid(
    deps: DepsMut,
    info: MessageInfo,
    id: OrderId,
) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    let collection_bid = collection_bids()
        .load(deps.storage, id.clone())
        .map_err(|_| ContractError::InvalidInput(format!("collection bid not found [{id}]")))?;
    ensure_eq!(
        info.sender,
        collection_bid.creator,
        ContractError::Unauthorized("only the collection bid creator can remove it".into())
    );
    let refund = collection_bid.terms.price.clone();
    collection_bid.remove(deps.storage)?;
    Ok(Response::new()
        .add_message(cosmwasm_std::BankMsg::Send {
            to_address: info.sender.to_string(),
            amount: vec![refund],
        })
        .add_event(
            cosmwasm_std::Event::new("remove-collection-bid")
                .add_attributes(collection_bid.event_attrs(&["id", "collection"])),
        ))
}

fn accept_collection_bid(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    id: OrderId,
    token_id: TokenId,
    terms: OrderTerms,
) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    let collection_bid = collection_bids()
        .load(deps.storage, id.clone())
        .map_err(|_| ContractError::InvalidInput(format!("collection bid not found [{id}]")))?;
    ensure!(
        has_coins(&[collection_bid.terms.price.clone()], &terms.price),
        ContractError::InvalidInput("min output is greater than collection bid price".into())
    );
    let ask = existing_or_new_ask(
        deps.as_ref(),
        &info,
        &env,
        &collection_bid.collection,
        &token_id,
        terms,
    )?;
    let config = CONFIG.load(deps.storage)?;
    finalize_sale(
        deps,
        &env,
        &ask,
        &config,
        &MatchingBid::CollectionBid(collection_bid),
        false,
        "accept-collection-bid",
        Response::new(),
    )
}

fn existing_or_new_ask(
    deps: cosmwasm_std::Deps,
    info: &MessageInfo,
    env: &Env,
    collection: &Addr,
    token_id: &str,
    terms: OrderTerms,
) -> Result<Ask, ContractError> {
    let ask_id = generate_id(&[collection.as_bytes(), token_id.as_bytes()]);
    if let Some(ask) = asks().may_load(deps.storage, ask_id)? {
        ensure_eq!(
            info.sender,
            ask.creator,
            ContractError::Unauthorized("sender is not the ask creator".into())
        );
        return Ok(ask);
    }
    only_owner(&deps.querier, &info.sender, collection, token_id)?;
    assert_transfer_approval(
        &deps.querier,
        &env.contract.address,
        &info.sender,
        collection,
        token_id,
    )?;
    Ok(Ask::new(
        info.sender.clone(),
        collection.clone(),
        token_id.to_string(),
        terms,
    ))
}

fn receive_nft(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: Cw721ReceiveMsg,
) -> Result<Response, ContractError> {
    let collection = info.sender.clone();
    let seller = deps.api.addr_validate(&msg.sender)?;
    let hook: HookMsg = from_json(&msg.msg)?;
    let details = match hook {
        HookMsg::SetAsk { details } => details,
        HookMsg::CreateAuction {
            denom,
            reserve_price,
            is_instant_sale,
        } => {
            if !is_instant_sale {
                return Err(ContractError::TimedAuctionUseReserve);
            }
            OrderDetails {
                price: Coin::new(Uint256::from(reserve_price), denom),
                recipient: None,
                finder: None,
            }
        }
    };
    let terms = details.validate(deps.api)?;
    set_ask(
        deps,
        env,
        info,
        collection,
        msg.token_id,
        terms,
        false,
        Some(seller),
    )
}

fn next_nonce(storage: &mut dyn cosmwasm_std::Storage) -> Result<u64, ContractError> {
    let nonce = NONCE.load(storage)?.wrapping_add(1);
    NONCE.save(storage, &nonce)?;
    Ok(nonce)
}

fn coin_u256(amount: Uint256, denom: &str) -> Coin {
    Coin::new(amount, denom)
}
