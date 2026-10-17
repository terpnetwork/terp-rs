use cosmwasm_schema::cw_serde;
use cosmwasm_std::{
    ensure, ensure_eq, to_json_binary, Addr, BankMsg, Coin, CosmosMsg, Decimal, Deps, Env, QuerierWrapper,
    Response, StdResult, Storage, Timestamp, Uint128, Uint256, WasmMsg,
};
use cw_storage_plus::Bound;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::msg::{BoundOffset, PriceOffset};
use crate::orders::{Ask, MatchingBid};
use crate::state::{Config, COLLECTION_DENOMS, MAX_BASIS_POINTS};
use crate::ContractError;

pub fn collection_token_index(collection: &str, token_id: &str) -> String {
    format!("{collection}/{token_id}")
}

pub fn generate_id(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hex::encode(hasher.finalize())
}

pub fn amount_u128(amount: Uint256) -> Result<u128, ContractError> {
    Uint128::try_from(amount.to_string().as_str())
        .map(|v| v.u128())
        .map_err(|_| ContractError::InvalidInput("amount exceeds u128".into()))
}

pub fn only_contract_admin(
    querier: &QuerierWrapper,
    env: &Env,
    sender: &Addr,
) -> Result<(), ContractError> {
    let info = querier.query_wasm_contract_info(&env.contract.address)?;
    let Some(admin) = info.admin else {
        return Err(ContractError::Unauthorized("contract admin unset".into()));
    };
    ensure_eq!(
        sender,
        admin,
        ContractError::Unauthorized("only the contract admin can perform this action".into())
    );
    Ok(())
}

pub fn only_valid_price(
    storage: &dyn Storage,
    config: &Config,
    collection: &Addr,
    price: &Coin,
    denom: Option<&str>,
) -> Result<(), ContractError> {
    ensure!(
        !price.amount.is_zero(),
        ContractError::InvalidInput("order price must be greater than 0".into())
    );
    amount_u128(price.amount)?;
    let expected = if let Some(denom) = denom {
        denom.to_string()
    } else {
        COLLECTION_DENOMS
            .may_load(storage, collection.clone())?
            .unwrap_or_else(|| config.default_denom.clone())
    };
    ensure_eq!(
        expected,
        price.denom,
        ContractError::InvalidInput("invalid denom".into())
    );
    Ok(())
}

pub fn price_bound(bound: BoundOffset<PriceOffset>) -> Bound<'static, (u128, String)> {
    match bound {
        BoundOffset::Inclusive(offset) => Bound::inclusive((offset.amount, offset.id)),
        BoundOffset::Exclusive(offset) => Bound::exclusive((offset.amount, offset.id)),
    }
}

pub fn id_bound(bound: BoundOffset<String>) -> Bound<'static, String> {
    match bound {
        BoundOffset::Inclusive(id) => Bound::inclusive(id),
        BoundOffset::Exclusive(id) => Bound::exclusive(id),
    }
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
    TransferNft {
        recipient: String,
        token_id: String,
    },
}

#[derive(Deserialize)]
struct IgnoredJson {}

#[derive(Deserialize)]
struct OwnerOfResponse {
    owner: String,
}

#[derive(Deserialize)]
struct CollectionQuery {
    extension: Option<CollectionExt>,
}

#[derive(Deserialize)]
struct CollectionExt {
    royalty_info: Option<RoyaltyLite>,
    start_trading_time: Option<Timestamp>,
}

#[derive(Clone, Deserialize)]
struct RoyaltyLite {
    payment_address: String,
    share: Decimal,
}

#[derive(Deserialize)]
struct RoyaltyPaymentResponse {
    royalty_default: Option<RoyaltyWrap>,
    royalty_protocol: Option<RoyaltyWrap>,
}

#[derive(Deserialize)]
struct RoyaltyWrap {
    royalty_entry: RoyaltyEntryLite,
}

#[derive(Deserialize)]
struct RoyaltyEntryLite {
    recipient: String,
    share: Decimal,
}

#[cw_serde]
enum RoyaltyQuery {
    RoyaltyPayment {
        collection: String,
        protocol: Option<String>,
    },
}

pub fn only_owner(
    querier: &QuerierWrapper,
    sender: &Addr,
    collection: &Addr,
    token_id: &str,
) -> Result<(), ContractError> {
    let owner: OwnerOfResponse = querier.query_wasm_smart(
        collection,
        &NftQuery::OwnerOf {
            token_id: token_id.to_string(),
            include_expired: Some(false),
        },
    )?;
    if owner.owner != sender.as_str() {
        return Err(ContractError::Unauthorized("sender is not owner".into()));
    }
    Ok(())
}

pub fn assert_transfer_approval(
    querier: &QuerierWrapper,
    marketplace: &Addr,
    owner: &Addr,
    collection: &Addr,
    token_id: &str,
) -> Result<(), ContractError> {
    let approval: StdResult<IgnoredJson> = querier.query_wasm_smart(
        collection,
        &NftQuery::Approval {
            token_id: token_id.to_string(),
            spender: marketplace.to_string(),
            include_expired: Some(false),
        },
    );
    if approval.is_ok() {
        return Ok(());
    }
    let operator: StdResult<IgnoredJson> = querier.query_wasm_smart(
        collection,
        &NftQuery::Operator {
            owner: owner.to_string(),
            operator: marketplace.to_string(),
            include_expired: Some(false),
        },
    );
    if operator.is_ok() {
        return Ok(());
    }
    Err(ContractError::Unauthorized(
        "marketplace is not approved to transfer this nft".into(),
    ))
}

pub fn only_tradable(
    querier: &QuerierWrapper,
    block_time: Timestamp,
    collection: &Addr,
) -> Result<(), ContractError> {
    let info: StdResult<CollectionQuery> =
        querier.query_wasm_smart(collection, &NftQuery::GetCollectionInfoAndExtension {});
    let Ok(info) = info else {
        return Ok(());
    };
    if let Some(ext) = info.extension {
        if let Some(start) = ext.start_trading_time {
            if start > block_time {
                return Err(ContractError::InvalidInput(
                    "collection is not tradable yet".into(),
                ));
            }
        }
    }
    Ok(())
}

pub fn transfer_nft(collection: &Addr, token_id: &str, recipient: &Addr, response: Response) -> StdResult<Response> {
    Ok(response.add_message(WasmMsg::Execute {
        contract_addr: collection.to_string(),
        msg: to_json_binary(&NftExec::TransferNft {
            recipient: recipient.to_string(),
            token_id: token_id.to_string(),
        })?,
        funds: vec![],
    }))
}

struct Royalty {
    recipient: Addr,
    share: Decimal,
}

fn load_royalty(
    deps: Deps,
    config: &Config,
    collection: &Addr,
    protocol: &Addr,
) -> StdResult<Option<Royalty>> {
    if let Some(registry) = &config.royalty_registry {
        if let Ok(payment) = deps.querier.query_wasm_smart::<RoyaltyPaymentResponse>(
            registry,
            &RoyaltyQuery::RoyaltyPayment {
                collection: collection.to_string(),
                protocol: Some(protocol.to_string()),
            },
        ) {
            let entry = payment
                .royalty_protocol
                .map(|p| p.royalty_entry)
                .or_else(|| payment.royalty_default.map(|d| d.royalty_entry));
            if let Some(entry) = entry {
                return Ok(Some(Royalty {
                    recipient: deps.api.addr_validate(&entry.recipient)?,
                    share: entry.share,
                }));
            }
        }
    }
    let info: StdResult<CollectionQuery> = deps
        .querier
        .query_wasm_smart(collection, &NftQuery::GetCollectionInfoAndExtension {});
    let Ok(info) = info else {
        return Ok(None);
    };
    let Some(ext) = info.extension else {
        return Ok(None);
    };
    let Some(royalty) = ext.royalty_info else {
        return Ok(None);
    };
    Ok(Some(Royalty {
        recipient: deps.api.addr_validate(&royalty.payment_address)?,
        share: royalty.share,
    }))
}

pub struct Payment {
    pub label: String,
    pub recipient: Addr,
    pub funds: Coin,
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

#[allow(clippy::too_many_arguments)]
pub fn finalize_sale(
    deps: cosmwasm_std::DepsMut,
    env: &Env,
    ask: &Ask,
    config: &Config,
    matching: &MatchingBid,
    ask_was_first: bool,
    action: &str,
    mut response: Response,
) -> Result<Response, ContractError> {
    let (nft_recipient, bid_terms) = match matching {
        MatchingBid::Bid(bid) => (bid.asset_recipient(), &bid.terms),
        MatchingBid::CollectionBid(bid) => (bid.asset_recipient(), &bid.terms),
    };
    let (sale_price, maker, taker) = if ask_was_first {
        (&ask.terms.price, &ask.terms.finder, &bid_terms.finder)
    } else {
        (&bid_terms.price, &bid_terms.finder, &ask.terms.finder)
    };

    let mut payments = Vec::new();
    let mut remaining = sale_price.amount;
    let denom = sale_price.denom.as_str();

    let protocol = Decimal::bps(config.protocol_fee_bps);
    let mut protocol_left = protocol;
    if protocol_left > Decimal::zero() {
        if maker.is_some() && config.maker_reward_bps > 0 {
            let reward = protocol.checked_mul(Decimal::bps(config.maker_reward_bps))?;
            protocol_left = protocol_left.checked_sub(reward)?;
            push_fee(
                &mut payments,
                &mut remaining,
                "maker",
                reward,
                maker.clone().unwrap(),
                sale_price,
            )?;
        }
        if taker.is_some() && config.taker_reward_bps > 0 {
            let reward = protocol.checked_mul(Decimal::bps(config.taker_reward_bps))?;
            protocol_left = protocol_left.checked_sub(reward)?;
            push_fee(
                &mut payments,
                &mut remaining,
                "taker",
                reward,
                taker.clone().unwrap(),
                sale_price,
            )?;
        }
        if protocol_left > Decimal::zero() {
            let fee_amt = sale_price.amount.mul_ceil(protocol_left);
            let fee_amt = if fee_amt > remaining { remaining } else { fee_amt };
            remaining = remaining.checked_sub(fee_amt)?;
            let burn_msgs = fair_burn(fee_amt, denom, config.burn_bps, &config.fee_manager);
            response = response.add_messages(burn_msgs);
            payments.push(Payment {
                label: "protocol".into(),
                recipient: config.fee_manager.clone(),
                funds: Coin::new(fee_amt, denom),
            });
        }
    }

    if let Some(royalty) = load_royalty(deps.as_ref(), config, &ask.collection, &env.contract.address)? {
        let cap = Decimal::bps(config.max_royalty_fee_bps.min(MAX_BASIS_POINTS));
        let share = if royalty.share > cap { cap } else { royalty.share };
        push_fee(
            &mut payments,
            &mut remaining,
            "royalty",
            share,
            royalty.recipient,
            sale_price,
        )?;
    }

    if !remaining.is_zero() {
        payments.push(Payment {
            label: "seller".into(),
            recipient: ask.asset_recipient(),
            funds: Coin::new(remaining, denom),
        });
    }

    for payment in &payments {
        if payment.label == "protocol" {
            continue;
        }
        if payment.funds.amount.is_zero() {
            continue;
        }
        response = response.add_message(BankMsg::Send {
            to_address: payment.recipient.to_string(),
            amount: vec![payment.funds.clone()],
        });
    }

    response = transfer_nft(&ask.collection, &ask.token_id, &nft_recipient, response)?;

    ask.remove(deps.storage)?;
    match matching {
        MatchingBid::Bid(bid) => bid.remove(deps.storage)?,
        MatchingBid::CollectionBid(bid) => bid.remove(deps.storage)?,
    }

    let mut event = cosmwasm_std::Event::new("finalize-sale")
        .add_attribute("collection", ask.collection.to_string())
        .add_attribute("token_id", ask.token_id.to_string())
        .add_attribute("price", sale_price.amount.to_string())
        .add_attribute("nft_recipient", nft_recipient.to_string())
        .add_attribute("marketplace_action", action);
    match matching {
        MatchingBid::Bid(bid) => event = event.add_attribute("bid", bid.id.clone()),
        MatchingBid::CollectionBid(bid) => {
            event = event.add_attribute("collection_bid", bid.id.clone())
        }
    }
    for payment in &payments {
        event = event.add_attribute(&payment.label, payment.funds.amount.to_string());
    }
    Ok(response.add_event(event))
}

fn push_fee(
    payments: &mut Vec<Payment>,
    remaining: &mut Uint256,
    label: &str,
    share: Decimal,
    recipient: Addr,
    sale_price: &Coin,
) -> Result<(), ContractError> {
    if share.is_zero() {
        return Ok(());
    }
    let mut fee_amt = sale_price.amount.mul_ceil(share);
    if fee_amt > *remaining {
        fee_amt = *remaining;
    }
    *remaining = remaining.checked_sub(fee_amt)?;
    payments.push(Payment {
        label: label.into(),
        recipient,
        funds: Coin::new(fee_amt, &sale_price.denom),
    });
    Ok(())
}
