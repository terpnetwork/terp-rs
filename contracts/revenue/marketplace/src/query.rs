use cosmwasm_std::{to_json_binary, Addr, Binary, Deps, Env, StdResult};

use crate::helpers::{id_bound, price_bound};
use crate::msg::QueryMsg;
use crate::orders::{Ask, Bid, CollectionBid};
use crate::state::{asks, bids, collection_bids, Denom, OrderId, COLLECTION_DENOMS, CONFIG};

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&CONFIG.load(deps.storage)?),
        QueryMsg::CollectionDenom { collection } => to_json_binary(
            &COLLECTION_DENOMS.may_load(deps.storage, deps.api.addr_validate(&collection)?)?,
        ),
        QueryMsg::Ask { id } => to_json_binary(&asks().may_load(deps.storage, id)?),
        QueryMsg::Asks { ids } => to_json_binary(&load_asks(deps, ids)?),
        QueryMsg::AsksByCollectionDenom {
            collection,
            denom,
            query_options,
        } => to_json_binary(&query_asks_by_collection_denom(
            deps,
            deps.api.addr_validate(&collection)?,
            denom,
            &query_options.unwrap_or_default(),
        )?),
        QueryMsg::AsksByCreatorCollection {
            creator,
            collection,
            query_options,
        } => to_json_binary(&query_asks_by_creator_collection(
            deps,
            deps.api.addr_validate(&creator)?,
            deps.api.addr_validate(&collection)?,
            &query_options.unwrap_or_default(),
        )?),
        QueryMsg::Bid { id } => to_json_binary(&bids().may_load(deps.storage, id)?),
        QueryMsg::Bids { ids } => to_json_binary(&load_bids(deps, ids)?),
        QueryMsg::BidsByTokenPrice {
            collection,
            token_id,
            denom,
            query_options,
        } => to_json_binary(&query_bids_by_token_price(
            deps,
            deps.api.addr_validate(&collection)?,
            token_id,
            denom,
            &query_options.unwrap_or_default(),
        )?),
        QueryMsg::BidsByCreatorCollection {
            creator,
            collection,
            query_options,
        } => to_json_binary(&query_bids_by_creator_collection(
            deps,
            deps.api.addr_validate(&creator)?,
            deps.api.addr_validate(&collection)?,
            &query_options.unwrap_or_default(),
        )?),
        QueryMsg::CollectionBid { id } => {
            to_json_binary(&collection_bids().may_load(deps.storage, id)?)
        }
        QueryMsg::CollectionBids { ids } => to_json_binary(&load_collection_bids(deps, ids)?),
        QueryMsg::CollectionBidsByPrice {
            collection,
            denom,
            query_options,
        } => to_json_binary(&query_collection_bids_by_price(
            deps,
            deps.api.addr_validate(&collection)?,
            denom,
            &query_options.unwrap_or_default(),
        )?),
        QueryMsg::CollectionBidsByCreatorCollection {
            creator,
            collection,
            query_options,
        } => to_json_binary(&query_collection_bids_by_creator_collection(
            deps,
            deps.api.addr_validate(&creator)?,
            deps.api.addr_validate(&collection)?,
            &query_options.unwrap_or_default(),
        )?),
    }
}

fn load_asks(deps: Deps, ids: Vec<OrderId>) -> StdResult<Vec<Ask>> {
    ids.into_iter()
        .filter_map(|id| asks().may_load(deps.storage, id).transpose())
        .collect()
}

fn load_bids(deps: Deps, ids: Vec<OrderId>) -> StdResult<Vec<Bid>> {
    ids.into_iter()
        .filter_map(|id| bids().may_load(deps.storage, id).transpose())
        .collect()
}

fn load_collection_bids(deps: Deps, ids: Vec<OrderId>) -> StdResult<Vec<CollectionBid>> {
    ids.into_iter()
        .filter_map(|id| collection_bids().may_load(deps.storage, id).transpose())
        .collect()
}

pub fn query_asks_by_collection_denom(
    deps: Deps,
    collection: Addr,
    denom: Denom,
    options: &crate::msg::QueryOptions<crate::msg::PriceOffset>,
) -> StdResult<Vec<Ask>> {
    let min = options.min.clone().map(price_bound);
    let max = options.max.clone().map(price_bound);
    asks()
        .idx
        .collection_denom_price
        .sub_prefix((collection, denom))
        .range(deps.storage, min, max, options.order())
        .take(options.limit())
        .map(|item| item.map(|(_, ask)| ask))
        .collect()
}

pub fn query_asks_by_creator_collection(
    deps: Deps,
    creator: Addr,
    collection: Addr,
    options: &crate::msg::QueryOptions<String>,
) -> StdResult<Vec<Ask>> {
    let min = options.min.clone().map(id_bound);
    let max = options.max.clone().map(id_bound);
    asks()
        .idx
        .creator_collection
        .prefix((creator, collection))
        .range(deps.storage, min, max, options.order())
        .take(options.limit())
        .map(|item| item.map(|(_, ask)| ask))
        .collect()
}

pub fn query_bids_by_token_price(
    deps: Deps,
    collection: Addr,
    token_id: String,
    denom: Denom,
    options: &crate::msg::QueryOptions<crate::msg::PriceOffset>,
) -> StdResult<Vec<Bid>> {
    let min = options.min.clone().map(price_bound);
    let max = options.max.clone().map(price_bound);
    bids()
        .idx
        .token_denom_price
        .sub_prefix((
            crate::helpers::collection_token_index(collection.as_ref(), &token_id),
            denom,
        ))
        .range(deps.storage, min, max, options.order())
        .take(options.limit())
        .map(|item| item.map(|(_, bid)| bid))
        .collect()
}

pub fn query_bids_by_creator_collection(
    deps: Deps,
    creator: Addr,
    collection: Addr,
    options: &crate::msg::QueryOptions<String>,
) -> StdResult<Vec<Bid>> {
    let min = options.min.clone().map(id_bound);
    let max = options.max.clone().map(id_bound);
    bids()
        .idx
        .creator_collection
        .prefix((creator, collection))
        .range(deps.storage, min, max, options.order())
        .take(options.limit())
        .map(|item| item.map(|(_, bid)| bid))
        .collect()
}

pub fn query_collection_bids_by_price(
    deps: Deps,
    collection: Addr,
    denom: Denom,
    options: &crate::msg::QueryOptions<crate::msg::PriceOffset>,
) -> StdResult<Vec<CollectionBid>> {
    let min = options.min.clone().map(price_bound);
    let max = options.max.clone().map(price_bound);
    collection_bids()
        .idx
        .collection_denom_price
        .sub_prefix((collection, denom))
        .range(deps.storage, min, max, options.order())
        .take(options.limit())
        .map(|item| item.map(|(_, bid)| bid))
        .collect()
}

pub fn query_collection_bids_by_creator_collection(
    deps: Deps,
    creator: Addr,
    collection: Addr,
    options: &crate::msg::QueryOptions<String>,
) -> StdResult<Vec<CollectionBid>> {
    let min = options.min.clone().map(id_bound);
    let max = options.max.clone().map(id_bound);
    collection_bids()
        .idx
        .creator_collection
        .prefix((creator, collection))
        .range(deps.storage, min, max, options.order())
        .take(options.limit())
        .map(|item| item.map(|(_, bid)| bid))
        .collect()
}

