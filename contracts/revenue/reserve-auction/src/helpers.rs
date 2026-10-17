use cosmwasm_schema::cw_serde;
use cosmwasm_std::{
    ensure, to_json_binary, Addr, BankMsg, Coin, CosmosMsg, Decimal, Deps, DepsMut, QuerierWrapper,
    Response, StdResult, Storage, Timestamp, Uint256, WasmMsg,
};
use serde::Deserialize;

use crate::state::{auctions, Auction, Config, HaltManager, MIN_RESERVE_PRICES};
use crate::ContractError;

pub fn validate_reserve(storage: &dyn Storage, price: &Coin) -> Result<(), ContractError> {
    let minimum = MIN_RESERVE_PRICES.may_load(storage, price.denom.clone())?;
    let Some(minimum) = minimum else {
        return Err(ContractError::InvalidInput(
            "reserve denom is not supported".into(),
        ));
    };
    ensure!(
        !price.amount.is_zero() && price.amount >= minimum,
        ContractError::InvalidInput("reserve price is below the minimum".into())
    );
    Ok(())
}

pub fn only_owner(
    querier: &QuerierWrapper,
    sender: &Addr,
    collection: &Addr,
    token_id: &str,
) -> Result<(), ContractError> {
    let owner: OwnerOf = querier.query_wasm_smart(
        collection,
        &NftQuery::OwnerOf {
            token_id: token_id.to_string(),
            include_expired: Some(false),
        },
    )?;
    ensure!(owner.owner == sender.as_str(), ContractError::Unauthorized);
    Ok(())
}

pub fn assert_approval(
    querier: &QuerierWrapper,
    auction: &Addr,
    owner: &Addr,
    collection: &Addr,
    token_id: &str,
) -> Result<(), ContractError> {
    let approval: StdResult<Ignored> = querier.query_wasm_smart(
        collection,
        &NftQuery::Approval {
            token_id: token_id.to_string(),
            spender: auction.to_string(),
            include_expired: Some(false),
        },
    );
    if approval.is_ok() {
        return Ok(());
    }
    let operator: StdResult<Ignored> = querier.query_wasm_smart(
        collection,
        &NftQuery::Operator {
            owner: owner.to_string(),
            operator: auction.to_string(),
            include_expired: Some(false),
        },
    );
    if operator.is_ok() {
        return Ok(());
    }
    Err(ContractError::Unauthorized)
}

pub fn transfer_nft(collection: &Addr, token_id: &str, recipient: &Addr) -> StdResult<WasmMsg> {
    Ok(WasmMsg::Execute {
        contract_addr: collection.to_string(),
        msg: to_json_binary(&NftExec::TransferNft {
            recipient: recipient.to_string(),
            token_id: token_id.to_string(),
        })?,
        funds: vec![],
    })
}

pub fn fair_burn(amount: Uint256, denom: &str, burn_bps: u64, fee_manager: &Addr) -> Vec<CosmosMsg> {
    if amount.is_zero() {
        return vec![];
    }
    let burn_amt = amount.mul_ceil(Decimal::bps(burn_bps));
    let rest = amount.saturating_sub(burn_amt);
    let mut msgs = Vec::new();
    if !burn_amt.is_zero() {
        msgs.push(
            BankMsg::Burn {
                amount: vec![Coin::new(burn_amt, denom)],
            }
            .into(),
        );
    }
    if !rest.is_zero() {
        msgs.push(
            BankMsg::Send {
                to_address: fee_manager.to_string(),
                amount: vec![Coin::new(rest, denom)],
            }
            .into(),
        );
    }
    msgs
}

fn load_royalty(deps: Deps, collection: &Addr) -> Option<(Addr, Decimal)> {
    let info: StdResult<CollectionQuery> = deps
        .querier
        .query_wasm_smart(collection, &NftQuery::GetCollectionInfoAndExtension {});
    let ext = info.ok()?.extension?;
    let royalty = ext.royalty_info?;
    let recipient = deps.api.addr_validate(&royalty.payment_address).ok()?;
    Some((recipient, royalty.share))
}

/// Pay the winning bid, transfer the NFT, and delete the auction.
/// A halt window that contains `end_time` postpones settlement instead.
pub fn settle_auction(
    deps: DepsMut,
    block_time: Timestamp,
    mut auction: Auction,
    config: &Config,
    halt: &HaltManager,
    mut response: Response,
) -> Result<Response, ContractError> {
    let end = auction.end_time.ok_or(ContractError::AuctionNotEnded)?;
    ensure!(end <= block_time, ContractError::AuctionNotEnded);
    if halt.contains(end.seconds()) {
        let next = block_time.plus_seconds(config.halt_postpone_duration);
        auction.end_time = Some(next);
        auctions().save(
            deps.storage,
            (auction.collection.clone(), auction.token_id.clone()),
            &auction,
        )?;
        return Ok(response.add_event(
            cosmwasm_std::Event::new("postpone-auction")
                .add_attribute("collection", auction.collection.to_string())
                .add_attribute("token_id", auction.token_id)
                .add_attribute("auction_end_time", next.to_string()),
        ));
    }

    let high = auction
        .high_bid
        .clone()
        .ok_or_else(|| ContractError::InvalidInput("ended auction has no bid".into()))?;
    auctions().remove(
        deps.storage,
        (auction.collection.clone(), auction.token_id.clone()),
    )?;

    let price = high.coin.amount;
    let denom = high.coin.denom.as_str();
    let mut remaining = price;
    let protocol = price.mul_ceil(config.trading_fee_percent).min(remaining);
    remaining = remaining.checked_sub(protocol)?;
    response = response.add_messages(fair_burn(
        protocol,
        denom,
        config.burn_bps,
        &config.fee_manager,
    ));

    if let Some((recipient, share)) = load_royalty(deps.as_ref(), &auction.collection) {
        let mut royalty = price.mul_ceil(share);
        if royalty > remaining {
            royalty = remaining;
        }
        remaining = remaining.checked_sub(royalty)?;
        if !royalty.is_zero() {
            response = response.add_message(BankMsg::Send {
                to_address: recipient.to_string(),
                amount: vec![Coin::new(royalty, denom)],
            });
        }
    }

    if !remaining.is_zero() {
        response = response.add_message(BankMsg::Send {
            to_address: auction.funds_recipient().to_string(),
            amount: vec![Coin::new(remaining, denom)],
        });
    }
    response = response.add_message(transfer_nft(
        &auction.collection,
        &auction.token_id,
        &high.bidder,
    )?);
    Ok(response.add_event(
        cosmwasm_std::Event::new("settle-auction")
            .add_attribute("collection", auction.collection.to_string())
            .add_attribute("token_id", auction.token_id)
            .add_attribute("seller", auction.seller.to_string())
            .add_attribute("bidder", high.bidder.to_string())
            .add_attribute("bid_amount", price.to_string()),
    ))
}

#[cw_serde]
enum NftQuery {
    OwnerOf {
        token_id: String,
        include_expired: Option<bool>,
    },
    Approval {
        token_id: String,
        spender: String,
        include_expired: Option<bool>,
    },
    Operator {
        owner: String,
        operator: String,
        include_expired: Option<bool>,
    },
    GetCollectionInfoAndExtension {},
}

#[cw_serde]
enum NftExec {
    TransferNft { recipient: String, token_id: String },
}

#[derive(Deserialize)]
struct Ignored {}

#[derive(Deserialize)]
struct OwnerOf {
    owner: String,
}

#[derive(Deserialize)]
struct CollectionQuery {
    extension: Option<CollectionExt>,
}

#[derive(Deserialize)]
struct CollectionExt {
    royalty_info: Option<RoyaltyLite>,
}

#[derive(Deserialize)]
struct RoyaltyLite {
    payment_address: String,
    share: Decimal,
}
