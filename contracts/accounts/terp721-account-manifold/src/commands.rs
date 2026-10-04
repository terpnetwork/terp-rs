use crate::{
    hooks::{prepare_ask_hook, prepare_bid_hook, prepare_sale_hook},
    state::*,
    state::{ACCOUNT_COLLECTION, CONFIG, PAUSED, SUDO_PARAMS},
    ContractError,
};
use cosmwasm_std::{
    coin, to_json_binary, Addr, BankMsg, Coin, Decimal, Deps, DepsMut, Env, Event, Fraction,
    MessageInfo, Order, Response, StdError, StdResult, Storage, SubMsg, Uint128, Uint256, WasmMsg,
};
use cw721::msg::{NftInfoResponse, OwnerOfResponse};
use cw_storage_plus::Bound;
use cw_utils::{must_pay, nonpayable};
use terp721_account::{
    helpers::Terp721Account, msg::ExecuteMsg as Bs721AccountExecuteMsg, QueryMsg,
};
use terp_account::{
    charge_fees,
    manifold::{hooks::HookAction, *},
    Ask, AskKey, Bid, BidKey, Bidder, Id, Metadata, PendingBid, TokenId,
    DEFAULT_QUERY_LIMIT, DEPLOYMENT_DAO, MAX_QUERY_LIMIT, NATIVE_DENOM,
};

pub fn execute_mint_and_list(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    account: &str,
) -> Result<Response, ContractError> {
    if PAUSED.load(deps.storage)? {
        return Err(ContractError::MintingPaused {});
    }

    let sender = &info.sender.to_string();
    let config = CONFIG.load(deps.storage)?;
    let params = SUDO_PARAMS.load(deps.storage)?;
    let collection = ACCOUNT_COLLECTION.load(deps.storage)?;
    let acc_len = account.len();

    if env.block.time < config.public_mint_start_time {
        return Err(ContractError::MintingNotStarted {});
    }

    validate_account(
        account,
        params.min_account_length,
        params.max_account_length,
    )?;
    let price = validate_payment(acc_len, &info, params.base_price.u128())?;
    validate_staking(
        deps.as_ref(),
        sender.as_ref(),
        acc_len,
        params.base_delegation,
    )?;

    let mut res = Response::new();
    // burns any tokens sent as fees if required (only uthiol supported currently)
    if let Some(fee) = &price {
        charge_fees(&mut res, fee.amount);
    }

    // SET ASK
    execute_set_ask(
        deps,
        env,
        &account.to_string(),
        info.sender,
        collection.clone(),
    )?;

    // mint token
    let mint_msg_exec = WasmMsg::Execute {
        contract_addr: collection.to_string(),
        msg: to_json_binary(&terp721_account::msg::ExecuteMsg::Mint {
            token_id: account.to_string(),
            owner: sender.to_string(),
            token_uri: None,
            extension: Metadata::default(),
        })?,
        funds: vec![],
    };

    let price = price.unwrap_or_else(|| coin(0u128, NATIVE_DENOM));
    let event = Event::new("mint-and-list")
        .add_attribute("account", account)
        .add_attribute("owner", sender)
        .add_attribute("price", price.amount.to_string());
    Ok(res.add_event(event).add_messages(vec![mint_msg_exec]))
}

/// Pause or unpause minting
pub fn execute_pause(
    deps: DepsMut,
    info: MessageInfo,
    pause: bool,
) -> Result<Response, ContractError> {
    cw_ownable::assert_owner(deps.storage, &info.sender)?;
    PAUSED.save(deps.storage, &pause)?;
    let event = Event::new("pause").add_attribute("pause", pause.to_string());
    Ok(Response::new().add_event(event))
}

pub fn execute_update_config(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    config: Config,
) -> Result<Response, ContractError> {
    cw_ownable::assert_owner(deps.storage, &info.sender)?;
    let start_time = config.public_mint_start_time;
    if env.block.time > start_time {
        return Err(ContractError::InvalidTradingStartTime(
            env.block.time,
            start_time,
        ));
    }
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new()
        .add_event(Event::new("update-config").add_attribute("address", info.sender.to_string())))
}

// This follows the same rules as Internet domain accounts
pub fn validate_account(account: &str, min: u32, max: u32) -> Result<(), ContractError> {
    let len = account.len() as u32;
    if len < min {
        return Err(ContractError::AccountTooShort {});
    } else if len >= max {
        return Err(ContractError::AccountTooLong {});
    }

    account
        .find(invalid_char)
        .map_or(Ok(()), |_| Err(ContractError::InvalidAccount {}))?;

    (if account.starts_with('-') || account.ends_with('-') {
        Err(ContractError::InvalidAccount {})
    } else {
        Ok(())
    })?;

    if len > 4u32 && account[2..4].contains("--") {
        return Err(ContractError::InvalidAccount {});
    }

    Ok(())
}
// This follows the same rules as Internet domain accounts
pub fn validate_staking(
    deps: Deps,
    delegator: &str,
    account_len: usize,
    base_delegation: Uint128,
) -> Result<(), ContractError> {
    let sum = deps
        .querier
        .query_all_delegations(delegator)?
        .into_iter()
        .map(|d| d.amount.amount)
        .sum::<Uint256>();
    let expected = match account_len {
        // never 3 or less, already checked earlier
        3 => base_delegation * Uint128::new(5u128),
        4 => base_delegation * Uint128::new(3u128),
        _ => base_delegation,
    };
    if sum < expected.into() {
        return Err(ContractError::IncorrectDelegation {
            got: sum,
            expected: expected.u128(),
        });
    };
    Ok(())
}

pub fn execute_update_owner(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    action: cw_ownable::Action,
) -> Result<Response, ContractError> {
    let ownership = cw_ownable::update_ownership(deps, &env.block, &info.sender, action)?;
    Ok(Response::default().add_attributes(ownership.into_attributes()))
}

pub enum Discount {
    Percent(Decimal),
}

pub fn validate_payment(
    account_len: usize,
    info: &MessageInfo,
    base_price: u128,
    // discount: Option<Discount>,
) -> Result<Option<Coin>, ContractError> {
    // Because we know we are left with ASCII chars, a simple byte count is enough
    let amount: Uint128 = (match account_len {
        0..=2 => {
            return Err(ContractError::AccountTooShort {});
        }
        3 => base_price * 100,
        4 => base_price * 10,
        _ => base_price,
    })
    .into();

    if amount.is_zero() {
        return Ok(None);
    }

    let payment = must_pay(info, NATIVE_DENOM)?;
    if payment != Uint256::new(amount.u128()) {
        return Err(ContractError::IncorrectPayment {
            got: payment,
            expected: amount.into(),
        });
    }

    Ok(Some(coin(amount.u128(), NATIVE_DENOM)))
}

pub fn invalid_char(c: char) -> bool {
    let is_valid = c.is_ascii_digit() || c.is_ascii_lowercase() || c == '-';
    !is_valid
}

pub fn query_collection(deps: Deps) -> StdResult<Addr> {
    ACCOUNT_COLLECTION.load(deps.storage)
}

pub fn query_params(deps: Deps) -> StdResult<SudoParams> {
    SUDO_PARAMS.load(deps.storage)
}

pub fn query_config(deps: Deps) -> StdResult<Config> {
    CONFIG.load(deps.storage)
}

pub fn sudo_update_params(
    deps: DepsMut,
    min_account_length: Option<u32>,
    max_account_length: Option<u32>,
    base_price: Option<Uint128>,
    base_delegation: Option<Uint128>,
    trading_fee_bps: Option<u64>,
    min_price: Option<Uint128>,
    ask_interval: Option<u64>,
    cooldown_duration: Option<u64>,
    cooldown_cancel_fee: Option<Coin>,
) -> Result<Response, ContractError> {
    if let Some(trading_fee_bps) = trading_fee_bps {
        if trading_fee_bps > MAX_FEE_BPS {
            return Err(ContractError::InvalidTradingFeeBps(trading_fee_bps));
        }
    }
    let mut params = SUDO_PARAMS.load(deps.storage)?;
    params.ask_interval = ask_interval.unwrap_or(params.ask_interval);
    params.base_delegation = base_delegation.unwrap_or(params.base_delegation);
    params.base_price = base_price.unwrap_or(params.base_price);
    params.cooldown_duration = cooldown_duration.unwrap_or(params.cooldown_duration);
    params.cooldown_fee = cooldown_cancel_fee.unwrap_or(params.cooldown_fee);
    params.min_price = min_price.unwrap_or(params.min_price);
    params.max_account_length = max_account_length.unwrap_or(params.max_account_length);
    params.min_account_length = min_account_length.unwrap_or(params.min_account_length);
    params.min_price = min_price.unwrap_or(params.min_price);
    params.trading_fee_percent = trading_fee_bps
        .map(|bps| Decimal::percent(bps) / Uint128::from(100u128))
        .unwrap_or(params.trading_fee_percent);
    SUDO_PARAMS.save(deps.storage, &params)?;
    Ok(Response::new().add_event(
        Event::new("update-params")
            .add_attribute(
                "trading_fee_percent",
                params.trading_fee_percent.to_string(),
            )
            .add_attribute("min_price", params.min_price),
    ))
}

pub fn sudo_update_account_collection(
    deps: DepsMut,
    collection: Addr,
) -> Result<Response, ContractError> {
    ACCOUNT_COLLECTION.save(deps.storage, &collection)?;
    let event = Event::new("update-account-collection").add_attribute("collection", collection);
    Ok(Response::new().add_event(event))
}

/// A seller may set an Ask on their NFT to list it on Marketplace
pub fn execute_set_ask(
    deps: DepsMut,
    env: Env,
    token_id: &str,
    seller: Addr,
    collection: Addr,
) -> Result<Response, ContractError> {
    // check if collection is approved to transfer on behalf of the seller
    Terp721Account(collection).operator(
        &deps.querier,
        seller.to_string(),
        env.contract.address.to_string(),
        false,
    )?;

    let renewal_time = env.block.time.plus_seconds(31536000u64);

    let ask = Ask {
        token_id: token_id.to_string(),
        id: increment_asks(deps.storage)?,
        seller: seller.clone(),
    };
    store_ask(deps.storage, &ask)?;

    let hook = prepare_ask_hook(deps.storage, &ask, HookAction::Create)?;

    let event = Event::new("set-ask")
        .add_attribute("token_id", token_id)
        .add_attribute("ask_id", ask.id.to_string())
        .add_attribute("renewal_time", renewal_time.to_string())
        .add_attribute("seller", seller);

    Ok(Response::new().add_event(event).add_submessages(hook))
}

/// Removes the ask on a particular NFT
pub fn execute_remove_ask(
    deps: DepsMut,
    info: MessageInfo,
    token_id: &str,
) -> Result<Response, ContractError> {
    nonpayable(&info)?;

    // `ask` can only be removed by burning from the collection
    let collection = ACCOUNT_COLLECTION.load(deps.storage)?;
    if info.sender != collection {
        return Err(ContractError::Unauthorized {});
    }

    // don't allow burning if ask has bids on it
    let bid_count = bids()
        .prefix(token_id.to_string())
        .keys(deps.storage, None, None, Order::Ascending)
        .count();
    if bid_count > 0 {
        return Err(ContractError::ExistingBids {});
    }

    let key = ask_key(token_id);
    let ask = asks().load(deps.storage, key.clone())?;
    asks().remove(deps.storage, key)?;

    let hook = prepare_ask_hook(deps.storage, &ask, HookAction::Delete)?;
    let event = Event::new("remove-ask").add_attribute("token_id", token_id);

    Ok(Response::new().add_event(event).add_submessages(hook))
}

/// When an NFT is transferred, the `ask` has to be updated with the new
/// seller. Also any existing bids must begin the checked bid removal process to refund bidders.
pub fn execute_update_ask(
    deps: DepsMut,
    info: MessageInfo,
    token_id: &str,
    seller: Addr,
) -> Result<Response, ContractError> {
    let collection = ACCOUNT_COLLECTION.load(deps.storage)?;
    if info.sender != collection {
        return Err(ContractError::Unauthorized {});
    }

    // refund any renewal funds and update the seller
    let mut ask = asks().load(deps.storage, ask_key(token_id))?;
    ask.seller = seller.clone();
    asks().save(deps.storage, ask_key(token_id), &ask)?;

    let mut res = Response::new().add_event(
        Event::new("update-ask")
            .add_attribute("token_id", token_id)
            .add_attribute("seller", seller),
    );

    // use remove bids and send any bid hooks
    let bids_to_remove = bids()
        .idx
        .price
        .sub_prefix(token_id.to_string()) // This matches (token_id, _)
        .keys(deps.storage, None, None, cosmwasm_std::Order::Ascending)
        .collect::<Result<Vec<BidKey>, _>>()?;
    checked_bid_removal(deps.storage, bids_to_remove, token_id, &mut res)?;

    Ok(res)
}

/// Places a bid on a account. The bid is escrowed in the contract.
pub fn execute_set_bid(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    token_id: &str,
) -> Result<Response, ContractError> {
    let params = SUDO_PARAMS.load(deps.storage)?;

    let ask_key = ask_key(token_id);
    asks().load(deps.storage, ask_key)?;

    let bid_price = must_pay(&info, NATIVE_DENOM)?;
    if bid_price < params.min_price.into() {
        return Err(ContractError::PriceTooSmall(bid_price));
    }
    let bid_price: Uint128 = bid_price.try_into()?;

    let bidder = info.sender;
    let mut res = Response::new();
    let bid_key = bid_key(token_id, &bidder);

    if let Some(existing_bid) = bids().may_load(deps.storage, bid_key.clone())? {
        bids().remove(deps.storage, bid_key)?;
        let refund_bidder = BankMsg::Send {
            to_address: bidder.to_string(),
            amount: vec![coin(existing_bid.amount.u128(), NATIVE_DENOM)],
        };
        res = res.add_message(refund_bidder)
    }

    let bid = Bid::new(token_id, bidder.clone(), bid_price, env.block.time);
    store_bid(deps.storage, &bid)?;

    let hook = prepare_bid_hook(deps.storage, &bid.clone(), HookAction::Create)?;

    let event = Event::new("set-bid")
        .add_attribute("token_id", token_id)
        .add_attribute("bidder", bidder)
        .add_attribute("bid_price", bid_price.to_string());

    Ok(res.add_event(event).add_submessages(hook))
}

/// Cancels an accepted bid in cooldown period. Only seller may call.
/// Requires seller to provide fee, which is split between bidder and developemnt team.
pub fn execute_cancel_cooldown(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    token_id: &str,
) -> Result<Response, ContractError> {
    let cd_key = &ask_key(token_id);
    match COOLDOWN_BID.may_load(deps.storage, cd_key)? {
        Some(p) => {
            // sender must be current token owner
            if info.sender != p.ask.seller {
                return Err(ContractError::Unauthorized {});
            };
            let mut res = Response::default();
            let params = SUDO_PARAMS.load(deps.storage)?;
            if params.cooldown_duration != 0 {
                // must have sent cancel cooldown fee
                let payment = must_pay(&info, NATIVE_DENOM)?;

                if payment != params.cooldown_fee.amount {
                    return Err(ContractError::IncorrectPayment {
                        got: payment,
                        expected: params.cooldown_fee.amount,
                    });
                }

                // cannot cancel if cooldown period is over
                if env.block.time >= p.unlock_time {
                    return Err(ContractError::InvalidDuration {});
                }

                // refund bidder
                let dev_cut = params
                    .cooldown_fee
                    .amount
                    .checked_div(Uint256::new(2u128))
                    .expect("fatal division error");

                let dev_cut_msg = BankMsg::Send {
                    to_address: DEPLOYMENT_DAO.to_string(),
                    amount: vec![Coin::new(dev_cut, NATIVE_DENOM.to_string())],
                };
                // refund bidder
                let seller_share_msg = BankMsg::Send {
                    to_address: p.new_owner.to_string(),
                    amount: vec![Coin::new(
                        (params.cooldown_fee.amount - dev_cut) + Uint256::new(p.amount.u128()),
                        NATIVE_DENOM.to_string(),
                    )],
                };
                res.messages.extend(vec![
                    SubMsg::new(seller_share_msg),
                    SubMsg::new(dev_cut_msg),
                ]);
            }
            COOLDOWN_BID.remove(deps.storage, cd_key);
            Ok(res)
        }
        None => return Err(ContractError::AskNotFound {}),
    }
}

/// Removes bids from an account. Only the account nft contract can call this.
///  Occurs either when:
/// - ExecuteMsg::AssociateAddress - an account updates the address it has associated itself with
/// - ExecuteMsg::UpdateAbsAccSupport - an account token updates their EOA (abstract-account) association parameters
pub fn execute_remove_bids(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    token_id: &str,
) -> Result<Response, ContractError> {
    if info.sender != ACCOUNT_COLLECTION.load(deps.storage)? {
        return Err(ContractError::Unauthorized {});
    };

    let bids_to_remove = bids()
        .idx
        .price
        .sub_prefix(token_id.to_string()) // This matches (token_id, _)
        .keys(deps.storage, None, None, cosmwasm_std::Order::Ascending)
        .collect::<Result<Vec<BidKey>, _>>()?;
    let mut res = Response::default();
    checked_bid_removal(deps.storage, bids_to_remove, token_id, &mut res)?;
    Ok(res)
}

/// Processes any stale bids set in the overflow buffer.
pub fn execute_removed_overflow_bids(
    deps: DepsMut,
    token_id: &str,
) -> Result<Response, ContractError> {
    // add any bids we will not compute into overflow store
    let mut bids_to_remove = Vec::new();
    let limit = MAX_REMOVE_BID_LIMIT as usize;
    OVERFLOW_BIDS_REMOVE.update(deps.storage, token_id.to_string(), |a| -> StdResult<_> {
        let mut list: Vec<(String, Addr)> = a.unwrap_or_default();
        if list.len() <= limit {
            // Return all, leave empty
            bids_to_remove = list;
            Ok(Vec::new())
        } else {
            // Split: take first `limit`, keep the rest
            bids_to_remove = list.drain(..limit).collect();
            Ok(list) // remaining
        }
    })?;

    let mut res = Response::default();
    checked_bid_removal(deps.storage, bids_to_remove, token_id, &mut res)?;
    Ok(res)
}

pub fn execute_remove_bid(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    token_id: &str,
) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    let bidder = info.sender;

    let key = bid_key(token_id, &bidder);
    let bid = bids().load(deps.storage, key.clone())?;
    bids().remove(deps.storage, key)?;

    let refund_bidder_msg = BankMsg::Send {
        to_address: bid.bidder.to_string(),
        amount: vec![coin(bid.amount.u128(), NATIVE_DENOM)],
    };

    let hook = prepare_bid_hook(deps.storage, &bid, HookAction::Delete)?;

    let event = Event::new("remove-bid")
        .add_attribute("token_id", token_id)
        .add_attribute("bidder", bidder);

    let res = Response::new()
        .add_message(refund_bidder_msg)
        .add_submessages(hook)
        .add_event(event);

    Ok(res)
}

pub fn execute_finalize_bid(
    deps: DepsMut,
    env: Env,
    token_id: &str,
) -> Result<Response, ContractError> {
    let collection = ACCOUNT_COLLECTION.load(deps.storage)?;
    let cd_key = &ask_key(token_id);
    let pending = COOLDOWN_BID.may_load(deps.storage, cd_key)?;
    let mut res = Response::default();
    match pending {
        Some(mut p) => {
            // check if pending bid is ready to be finalized
            if env.block.time < p.unlock_time {
                return Err(ContractError::InvalidDuration {});
            }
            // Check if token is approved for transfer
            let ops = Terp721Account(collection.clone()).approval(
                &deps.querier,
                token_id,
                &env.contract.address.to_string(),
                None,
            );
            if ops.is_err() || ops?.approval.expires.is_expired(&env.block) {
                // market automatically approves msg for itself
                res.messages
                    .push(SubMsg::new(Terp721Account(collection.clone()).call(
                        Bs721AccountExecuteMsg::ApproveAllViaMarket {
                            owner: p.ask.seller.to_string(),
                            expires: None,
                        },
                    )?));
            };

            let token: NftInfoResponse<Metadata> =
                Terp721Account(collection.clone()).nft_info(&deps.querier, token_id)?;

            // get the associated abstract account we expect to exist
            #[cfg(feature = "abstract")]
            if token.extension.account_ownership
                && terp_account::validate_aa_ownership(
                    deps.as_ref(),
                    &token.token_uri.expect(
                        "should never have aa support enabled and not have account associated",
                    ),
                    token_id,
                    &collection.clone(),
                    true,
                )
                .is_err()
            {
                // abstract account does not use this token for its ownership method.
                // we refund the bidder by setting their address in the current ask, and still transfer the token to the bidder.
                // this penalizes the original owner for changing ownership of their account during the cooldown phase.
                p.ask.seller = p.new_owner.clone();
            }

            // Transfer funds and NFT
            finalize_sale(
                deps.as_ref(),
                p.ask.clone(),
                p.amount,
                p.new_owner.clone(),
                &mut res,
            )?;
            COOLDOWN_BID.remove(deps.storage, cd_key);

            let bid_to_remove = bids()
                .idx
                .price
                .sub_prefix(token_id.to_string()) // This matches (token_id, _)
                .keys(deps.storage, None, None, cosmwasm_std::Order::Ascending)
                .collect::<Result<Vec<BidKey>, _>>()?;

            checked_bid_removal(deps.storage, bid_to_remove, token_id, &mut res)?;
            store_ask(
                deps.storage,
                &Ask {
                    token_id: token_id.to_string(),
                    id: p.ask.id,
                    seller: p.new_owner.clone(),
                },
            )?;
        }
        None => {
            return Err(ContractError::AskNotFound {});
        }
    }
    Ok(res)
}
/// Seller can accept a bid which transfers funds as well as the token.
/// The bid is removed, then a new ask is created for the same token.
pub fn execute_accept_bid(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    token_id: &str,
    bidder: Addr,
) -> Result<Response, ContractError> {
    // println!("1.0 execute_accept bid ----------------------------");
    nonpayable(&info)?;
    let collection = ACCOUNT_COLLECTION.load(deps.storage)?;
    let cooldown = SUDO_PARAMS.load(deps.storage)?.cooldown_duration;
    only_owner(deps.as_ref(), &info, &collection, token_id)?;

    let ask_key = ask_key(token_id);
    let bid_key = bid_key(token_id, &bidder);

    let ask = asks().load(deps.storage, ask_key.clone())?;
    let bid = bids().load(deps.storage, bid_key.clone())?;

    // Check if token is approved for transfer
    Terp721Account(collection.clone()).approval(
        &deps.querier,
        token_id,
        info.sender.as_ref(),
        None,
    )?;
    let mut res = Response::default();
    // Remove accepted bid
    bids().remove(deps.storage, bid_key)?;
    let bid_to_remove = bids()
        .idx
        .price
        .sub_prefix(token_id.to_string()) // This matches (token_id, _)
        .keys(deps.storage, None, None, cosmwasm_std::Order::Ascending)
        .collect::<Result<Vec<BidKey>, _>>()?;
    checked_bid_removal(deps.storage, bid_to_remove, token_id, &mut res)?;

    // begin cooldown period
    let unlock_time = env.block.time.plus_seconds(cooldown);
    let pending = PendingBid::new(ask.clone(), bidder.clone(), bid.amount, unlock_time);
    COOLDOWN_BID.save(deps.storage, &ask_key, &pending)?;

    Ok(res.add_event(
        Event::new("accept-bid")
            .add_attribute("token_id", token_id)
            .add_attribute("bidder", bidder)
            .add_attribute("price", bid.amount.to_string()),
    ))
}

/// Transfers funds and NFT, updates bid
fn finalize_sale(
    deps: Deps,
    ask: Ask,
    price: Uint128,
    buyer: Addr,
    res: &mut Response,
) -> StdResult<()> {
    // println!("1.1 finalize sale ----------------------------");
    payout(deps, price, ask.seller.clone(), res)?;

    let cw721_transfer_msg: Bs721AccountExecuteMsg<Metadata> =
        Bs721AccountExecuteMsg::TransferNft {
            token_id: ask.token_id.to_string(),
            recipient: buyer.to_string(),
        };

    let collection = ACCOUNT_COLLECTION.load(deps.storage)?;

    let exec_cw721_transfer = WasmMsg::Execute {
        contract_addr: collection.to_string(),
        msg: to_json_binary(&cw721_transfer_msg)?,
        funds: vec![],
    };
    res.messages.push(SubMsg::new(exec_cw721_transfer));

    res.messages
        .append(&mut prepare_sale_hook(deps.storage, &ask, buyer.clone())?);

    let event = Event::new("finalize-sale")
        .add_attribute("token_id", ask.token_id.to_string())
        .add_attribute("seller", ask.seller.to_string())
        .add_attribute("buyer", buyer.to_string())
        .add_attribute("price", price.to_string());
    res.events.push(event);

    Ok(())
}

// only process up to MAX_REMOVE_BID_LIMIT in initial bid remove.
// Any 'overflow bids' stored into map, and upon either:
//     - new bids being set
//     - ask updated/removed
// we pop MAX_REMOVE_BID_LIMIT again, and logic repeats until no old bids are removed.
pub fn checked_bid_removal(
    storage: &mut dyn Storage,
    bids_to_remove: Vec<BidKey>,
    token_id: &str,
    res: &mut Response,
) -> StdResult<()> {
    let (process, remaining) = bids_to_remove.split_at(std::cmp::min(
        MAX_REMOVE_BID_LIMIT as usize,
        bids_to_remove.len(),
    ));

    // add any bids we will not compute back into overflow store
    OVERFLOW_BIDS_REMOVE.update(storage, token_id.to_string(), |a| {
        let mut overflow = a.unwrap_or_default();
        overflow.extend(remaining.to_vec());
        return Ok::<Vec<BidKey>, StdError>(overflow);
    })?;

    // refund bidders, call bid hooks
    let mut submsgs = Vec::new();
    for key in process {
        let bid = bids().load(storage, key.clone())?;
        submsgs.extend(prepare_bid_hook(storage, &bid, HookAction::Delete)?);
        bids().remove(storage, key.clone())?;
        submsgs.push(SubMsg::new(BankMsg::Send {
            to_address: bid.bidder.to_string(),
            amount: vec![coin(bid.amount.u128(), NATIVE_DENOM)],
        }));
    }

    res.messages.extend(submsgs);

    Ok(())
}

/// Payout a bid
pub fn payout(
    deps: Deps,
    payment: Uint128,
    payment_recipient: Addr,
    res: &mut Response,
) -> StdResult<()> {
    let params = SUDO_PARAMS.load(deps.storage)?;

    let fee = payment.multiply_ratio(
        params.trading_fee_percent.numerator(),
        params.trading_fee_percent.denominator(),
    );
    if fee > payment {
        return Err(StdError::msg("Fees exceed payment"));
    }
    charge_fees(res, fee.into());

    // pay seller
    let seller_share_msg = BankMsg::Send {
        to_address: payment_recipient.to_string(),
        amount: vec![coin((payment - fee).u128(), NATIVE_DENOM.to_string())],
    };
    res.messages.push(SubMsg::new(seller_share_msg));

    Ok(())
}

fn store_bid(store: &mut dyn Storage, bid: &Bid) -> StdResult<()> {
    bids().save(store, bid_key(&bid.token_id, &bid.bidder), bid)
}

pub fn store_ask(store: &mut dyn Storage, ask: &Ask) -> StdResult<()> {
    asks().save(store, ask_key(&ask.token_id), ask)
}

/// Checks to enfore only NFT owner can call
fn only_owner(
    deps: Deps,
    info: &MessageInfo,
    collection: &Addr,
    token_id: &str,
) -> Result<OwnerOfResponse, ContractError> {
    let res: OwnerOfResponse = deps.querier.query_wasm_smart(
        collection,
        &QueryMsg::OwnerOf {
            token_id: token_id.into(),
            include_expired: Some(false),
        },
    )?;

    if res.owner != info.sender.to_string() {
        return Err(ContractError::UnauthorizedOwner {});
    }

    Ok(res)
}

pub fn query_asks(deps: Deps, start_after: Option<Id>, limit: Option<u32>) -> StdResult<Vec<Ask>> {
    let limit = limit.unwrap_or(DEFAULT_QUERY_LIMIT).min(MAX_QUERY_LIMIT) as usize;

    asks()
        .idx
        .id
        .range(
            deps.storage,
            Some(Bound::exclusive(start_after.unwrap_or_default())),
            None,
            Order::Ascending,
        )
        .take(limit)
        .map(|res| res.map(|item| item.1))
        .collect::<StdResult<Vec<_>>>()
}

pub fn query_ask_count(deps: Deps) -> StdResult<u32> {
    ASK_COUNT.load(deps.storage)
}

// TODO: figure out how to paginate by `Id` instead of `TokenId`
pub fn query_asks_by_seller(
    deps: Deps,
    seller: Addr,
    start_after: Option<TokenId>,
    limit: Option<u32>,
) -> StdResult<Vec<Ask>> {
    let limit = limit.unwrap_or(DEFAULT_QUERY_LIMIT).min(MAX_QUERY_LIMIT) as usize;

    let start = start_after.map(|start| Bound::exclusive(ask_key(&start)));

    asks()
        .idx
        .seller
        .prefix(seller)
        .range(deps.storage, start, None, Order::Ascending)
        .take(limit)
        .map(|res| res.map(|item| item.1))
        .collect::<StdResult<Vec<_>>>()
}

pub fn query_ask(deps: Deps, token_id: TokenId) -> StdResult<Option<Ask>> {
    asks().may_load(deps.storage, ask_key(&token_id))
}

pub fn query_bid(deps: Deps, token_id: TokenId, bidder: Addr) -> StdResult<Option<Bid>> {
    bids().may_load(deps.storage, (token_id, bidder))
}

pub fn query_bids_by_bidder(
    deps: Deps,
    bidder: Addr,
    start_after: Option<TokenId>,
    limit: Option<u32>,
) -> StdResult<Vec<Bid>> {
    let limit = limit.unwrap_or(DEFAULT_QUERY_LIMIT).min(MAX_QUERY_LIMIT) as usize;

    let start = start_after.map(|start| Bound::exclusive((start, bidder.clone())));

    bids()
        .idx
        .bidder
        .prefix(bidder)
        .range(deps.storage, start, None, Order::Ascending)
        .take(limit)
        .map(|item| item.map(|(_, b)| b))
        .collect::<StdResult<Vec<_>>>()
}

pub fn query_bids_for_seller(
    deps: Deps,
    seller: Addr,
    start_after: Option<BidOffset>,
    limit: Option<u32>,
) -> StdResult<Vec<Bid>> {
    let limit = limit.unwrap_or(DEFAULT_QUERY_LIMIT).min(MAX_QUERY_LIMIT) as usize;
    // Query seller asks, then collect bids starting after token_id
    // Limitation: Can not collect bids in the middle using `start_after: token_id` pattern
    // This leads to imprecise pagination based on token id and not bid count
    let start_token_id =
        start_after.map(|start| Bound::<AskKey>::exclusive(ask_key(&start.token_id)));

    let bids = asks()
        .idx
        .seller
        .prefix(seller)
        .range(deps.storage, start_token_id, None, Order::Ascending)
        .take(limit)
        .map(|res| res.map(|item| item.0).unwrap())
        .flat_map(|token_id| {
            bids()
                .prefix(token_id)
                .range(deps.storage, None, None, Order::Ascending)
                .flat_map(|item| item.map(|(_, b)| b))
                .collect::<Vec<_>>()
        })
        .collect();

    Ok(bids)
}

pub fn query_bids(
    deps: Deps,
    token_id: TokenId,
    start_after: Option<Bidder>,
    limit: Option<u32>,
) -> StdResult<Vec<Bid>> {
    let limit = limit.unwrap_or(DEFAULT_QUERY_LIMIT).min(MAX_QUERY_LIMIT) as usize;
    let start = start_after.map(|s| Bound::ExclusiveRaw(s.into()));

    bids()
        .prefix(token_id)
        .range(deps.storage, start, None, Order::Ascending)
        .take(limit)
        .map(|item| item.map(|(_, b)| b))
        .collect::<StdResult<Vec<_>>>()
}

pub fn query_highest_bid(deps: Deps, token_id: TokenId) -> StdResult<Option<Bid>> {
    let bid = bids()
        .idx
        .price
        .range(deps.storage, None, None, Order::Descending)
        .filter_map(|item| {
            let (key, bid) = item.unwrap();
            if key.0 == token_id {
                Some(bid)
            } else {
                None
            }
        })
        .take(1)
        .collect::<Vec<_>>()
        .first()
        .cloned();

    Ok(bid)
}

pub fn query_bids_sorted_by_price(
    deps: Deps,
    start_after: Option<BidOffset>,
    limit: Option<u32>,
) -> StdResult<Vec<Bid>> {
    let limit = limit.unwrap_or(DEFAULT_QUERY_LIMIT).min(MAX_QUERY_LIMIT) as usize;

    let start = start_after.map(|offset| {
        Bound::exclusive((
            (offset.token_id.clone(), offset.price.u128()),
            bid_key(&offset.token_id, &offset.bidder),
        ))
    });

    bids()
        .idx
        .price
        .range(deps.storage, start, None, Order::Ascending)
        .take(limit)
        .map(|item| item.map(|(_, b)| b))
        .collect::<StdResult<Vec<_>>>()
}

pub fn reverse_query_bids_sorted_by_price(
    deps: Deps,
    start_before: Option<BidOffset>,
    limit: Option<u32>,
) -> StdResult<Vec<Bid>> {
    let limit = limit.unwrap_or(DEFAULT_QUERY_LIMIT).min(MAX_QUERY_LIMIT) as usize;

    let end = start_before.map(|offset| {
        Bound::exclusive((
            (offset.token_id.clone(), offset.price.u128()),
            bid_key(&offset.token_id, &offset.bidder),
        ))
    });

    bids()
        .idx
        .price
        .range(deps.storage, None, end, Order::Descending)
        .take(limit)
        .map(|item| item.map(|(_, b)| b))
        .collect::<StdResult<Vec<_>>>()
}

pub fn manage_hooks(
    deps: DepsMut,
    sender: Addr,
    action: ManageHooksAction,
) -> Result<Response, ContractError> {
    // only hooks admin may invoke
    if sender.to_string() != SUDO_PARAMS.load(deps.storage)?.hooks_admin {
        return Err(ContractError::UnauthorizedMinter {});
    }
    let mut res = Response::default();
    match action {
        ManageHooksAction::AddSaleHook(hook) => {
            SALE_HOOKS.add_hook(deps.storage, deps.api.addr_validate(&hook)?)?;
            res.events
                .push(Event::new("add-sale-hook").add_attribute("hook", hook));
        }
        ManageHooksAction::RemoveSaleHook(hook) => {
            SALE_HOOKS.remove_hook(deps.storage, deps.api.addr_validate(&hook)?)?;
            res.events
                .push(Event::new("remove-sale-hook").add_attribute("hook", hook));
        }
        ManageHooksAction::AddAskHook(hook) => {
            ASK_HOOKS.add_hook(deps.storage, deps.api.addr_validate(&hook)?)?;
            res.events
                .push(Event::new("add-ask-hook").add_attribute("hook", hook));
        }
        ManageHooksAction::RemoveAskHook(hook) => {
            ASK_HOOKS.remove_hook(deps.storage, deps.api.addr_validate(&hook)?)?;
            res.events
                .push(Event::new("remove-ask-hook").add_attribute("hook", hook));
        }
        ManageHooksAction::AddBidHook(hook) => {
            BID_HOOKS.add_hook(deps.storage, deps.api.addr_validate(&hook)?)?;
            res.events
                .push(Event::new("add-bid-hook").add_attribute("hook", hook));
        }
        ManageHooksAction::RemoveBidHook(hook) => {
            BID_HOOKS.remove_hook(deps.storage, deps.api.addr_validate(&hook)?)?;
            res.events
                .push(Event::new("remove-bid-hook").add_attribute("hook", hook));
        }
    }
    Ok(res)
}

#[cfg(test)]
mod tests {
    use cosmwasm_std::{coin, Addr, MessageInfo, Uint256};
    use terp_account::CURRENT_BASE_PRICE;

    use crate::commands::{validate_account, validate_payment};

    #[test]
    fn check_validate_account() {
        let min = 3;
        let max = 63;
        assert!(validate_account("bobo", min, max).is_ok());
        assert!(validate_account("-bobo", min, max).is_err());
        assert!(validate_account("bobo-", min, max).is_err());
        assert!(validate_account("bo-bo", min, max).is_ok());
        assert!(validate_account("bo--bo", min, max).is_err());
        assert!(validate_account("bob--o", min, max).is_ok());
        assert!(validate_account("bo", min, max).is_err());
        assert!(validate_account("b", min, max).is_err());
        assert!(validate_account("bob", min, max).is_ok());
        assert!(validate_account(
            "bobobobobobobobobobobobobobobobobobobobobobobobobobobobobobobo",
            min,
            max
        )
        .is_ok());
        assert!(validate_account(
            "bobobobobobobobobobobobobobobobobobobobobobobobobobobobobobobob",
            min,
            max
        )
        .is_err());
        assert!(validate_account("0123456789", min, max).is_ok());
        assert!(validate_account("😬", min, max).is_err());
        assert!(validate_account("BOBO", min, max).is_err());
        assert!(validate_account("b-o----b", min, max).is_ok());
        assert!(validate_account("bobo.stars", min, max).is_err());
    }

    #[test]
    fn check_validate_payment() {
        let base_price = CURRENT_BASE_PRICE as u128;

        let info = MessageInfo {
            sender: Addr::unchecked("sender"),
            funds: vec![coin(base_price, "uthiol")],
        };
        assert_eq!(
            validate_payment(5, &info, base_price)
                .unwrap()
                .unwrap()
                .amount,
            Uint256::new(base_price)
        );

        let info = MessageInfo {
            sender: Addr::unchecked("sender"),
            funds: vec![coin(base_price * 10, "uthiol")],
        };
        assert_eq!(
            validate_payment(4, &info, base_price)
                .unwrap()
                .unwrap()
                .amount,
            Uint256::new(base_price * 10)
        );

        let info = MessageInfo {
            sender: Addr::unchecked("sender"),
            funds: vec![coin(base_price * 100, "uthiol")],
        };
        assert_eq!(
            validate_payment(3, &info, base_price)
                .unwrap()
                .unwrap()
                .amount,
            Uint256::new(base_price * 100)
        );
    }
}
