use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Attribute, Coin, Deps};

use crate::helpers::generate_id;
use crate::msg::{BoundOffset, PriceOffset, QueryOptions};
use crate::query::{
    query_asks_by_collection_denom, query_bids_by_token_price, query_collection_bids_by_price,
};
use crate::state::{asks, bids, collection_bids, TokenId};
use crate::ContractError;

#[cw_serde]
pub struct OrderTerms {
    pub price: Coin,
    pub recipient: Option<Addr>,
    pub finder: Option<Addr>,
}

pub enum MatchingBid {
    Bid(Bid),
    CollectionBid(CollectionBid),
}

#[cw_serde]
pub struct Ask {
    pub id: String,
    pub creator: Addr,
    pub collection: Addr,
    pub token_id: TokenId,
    pub terms: OrderTerms,
}

impl Ask {
    pub fn new(
        creator: Addr,
        collection: Addr,
        token_id: TokenId,
        terms: OrderTerms,
    ) -> Self {
        Self {
            id: generate_id(&[collection.as_bytes(), token_id.as_bytes()]),
            creator,
            collection,
            token_id,
            terms,
        }
    }

    pub fn asset_recipient(&self) -> Addr {
        self.terms
            .recipient
            .clone()
            .unwrap_or_else(|| self.creator.clone())
    }

    pub fn save(&self, storage: &mut dyn cosmwasm_std::Storage) -> Result<(), ContractError> {
        asks().save(storage, self.id.clone(), self)?;
        Ok(())
    }

    pub fn remove(&self, storage: &mut dyn cosmwasm_std::Storage) -> Result<(), ContractError> {
        asks().remove(storage, self.id.clone())?;
        Ok(())
    }

    pub fn match_with_bid(&self, deps: Deps) -> Result<Option<MatchingBid>, ContractError> {
        let amount = crate::helpers::amount_u128(self.terms.price.amount)?;
        let mut bid_q = QueryOptions {
            descending: Some(true),
            limit: Some(1),
            min: Some(BoundOffset::Inclusive(PriceOffset {
                id: String::new(),
                amount,
            })),
            max: None,
        };
        let top_bid = query_bids_by_token_price(
            deps,
            self.collection.clone(),
            self.token_id.clone(),
            self.terms.price.denom.clone(),
            &bid_q,
        )?
        .pop();
        bid_q.descending = Some(true);
        let top_collection = query_collection_bids_by_price(
            deps,
            self.collection.clone(),
            self.terms.price.denom.clone(),
            &bid_q,
        )?
        .pop();
        Ok(match (top_bid, top_collection) {
            (Some(bid), Some(collection_bid)) => {
                if bid.terms.price.amount >= collection_bid.terms.price.amount {
                    Some(MatchingBid::Bid(bid))
                } else {
                    Some(MatchingBid::CollectionBid(collection_bid))
                }
            }
            (Some(bid), None) => Some(MatchingBid::Bid(bid)),
            (None, Some(collection_bid)) => Some(MatchingBid::CollectionBid(collection_bid)),
            (None, None) => None,
        })
    }

    pub fn event_attrs(&self, keys: &[&str]) -> Vec<Attribute> {
        event_attrs(keys, |key| ask_attr(self, key))
    }
}

#[cw_serde]
pub struct Bid {
    pub id: String,
    pub creator: Addr,
    pub collection: Addr,
    pub token_id: TokenId,
    pub terms: OrderTerms,
}

impl Bid {
    pub fn new(
        creator: Addr,
        collection: Addr,
        token_id: TokenId,
        terms: OrderTerms,
        height: u64,
        nonce: u64,
    ) -> Self {
        Self {
            id: generate_id(&[
                collection.as_bytes(),
                token_id.as_bytes(),
                &height.to_be_bytes(),
                &nonce.to_be_bytes(),
            ]),
            creator,
            collection,
            token_id,
            terms,
        }
    }

    pub fn asset_recipient(&self) -> Addr {
        self.terms
            .recipient
            .clone()
            .unwrap_or_else(|| self.creator.clone())
    }

    pub fn save(&self, storage: &mut dyn cosmwasm_std::Storage) -> Result<(), ContractError> {
        bids().save(storage, self.id.clone(), self)?;
        Ok(())
    }

    pub fn remove(&self, storage: &mut dyn cosmwasm_std::Storage) -> Result<(), ContractError> {
        bids().remove(storage, self.id.clone())?;
        Ok(())
    }

    pub fn match_with_ask(&self, deps: Deps) -> Result<Option<Ask>, ContractError> {
        let id = generate_id(&[self.collection.as_bytes(), self.token_id.as_bytes()]);
        let Some(ask) = asks().may_load(deps.storage, id)? else {
            return Ok(None);
        };
        if cosmwasm_std::has_coins(&[self.terms.price.clone()], &ask.terms.price) {
            Ok(Some(ask))
        } else {
            Ok(None)
        }
    }

    pub fn event_attrs(&self, keys: &[&str]) -> Vec<Attribute> {
        event_attrs(keys, |key| bid_attr(self, key))
    }
}

#[cw_serde]
pub struct CollectionBid {
    pub id: String,
    pub creator: Addr,
    pub collection: Addr,
    pub terms: OrderTerms,
}

impl CollectionBid {
    pub fn new(
        creator: Addr,
        collection: Addr,
        terms: OrderTerms,
        height: u64,
        nonce: u64,
    ) -> Self {
        Self {
            id: generate_id(&[
                collection.as_bytes(),
                &height.to_be_bytes(),
                &nonce.to_be_bytes(),
            ]),
            creator,
            collection,
            terms,
        }
    }

    pub fn asset_recipient(&self) -> Addr {
        self.terms
            .recipient
            .clone()
            .unwrap_or_else(|| self.creator.clone())
    }

    pub fn save(&self, storage: &mut dyn cosmwasm_std::Storage) -> Result<(), ContractError> {
        collection_bids().save(storage, self.id.clone(), self)?;
        Ok(())
    }

    pub fn remove(&self, storage: &mut dyn cosmwasm_std::Storage) -> Result<(), ContractError> {
        collection_bids().remove(storage, self.id.clone())?;
        Ok(())
    }

    pub fn match_with_ask(&self, deps: Deps) -> Result<Option<Ask>, ContractError> {
        let amount = crate::helpers::amount_u128(self.terms.price.amount)?;
        let max_amount = amount.saturating_add(1);
        let max = if max_amount == amount {
            None
        } else {
            Some(BoundOffset::Exclusive(PriceOffset {
                id: String::new(),
                amount: max_amount,
            }))
        };
        let options = QueryOptions {
            descending: Some(false),
            limit: Some(1),
            min: None,
            max,
        };
        Ok(query_asks_by_collection_denom(
            deps,
            self.collection.clone(),
            self.terms.price.denom.clone(),
            &options,
        )?
        .pop())
    }

    pub fn event_attrs(&self, keys: &[&str]) -> Vec<Attribute> {
        event_attrs(keys, |key| collection_attr(self, key))
    }
}

fn event_attrs(keys: &[&str], one: impl Fn(&str) -> Option<Attribute>) -> Vec<Attribute> {
    keys.iter().filter_map(|key| one(key)).collect()
}

fn ask_attr(ask: &Ask, key: &str) -> Option<Attribute> {
    let attr = |k: &str, v: String| Attribute {
        key: k.to_string(),
        value: v,
    };
    match key {
        "id" => Some(attr("id", ask.id.clone())),
        "creator" => Some(attr("creator", ask.creator.to_string())),
        "collection" => Some(attr("collection", ask.collection.to_string())),
        "token_id" => Some(attr("token_id", ask.token_id.clone())),
        "price" => Some(attr("price", ask.terms.price.to_string())),
        "recipient" => ask
            .terms
            .recipient
            .as_ref()
            .map(|a| attr("recipient", a.to_string())),
        "finder" => ask
            .terms
            .finder
            .as_ref()
            .map(|a| attr("finder", a.to_string())),
        _ => None,
    }
}

fn bid_attr(bid: &Bid, key: &str) -> Option<Attribute> {
    let attr = |k: &str, v: String| Attribute {
        key: k.to_string(),
        value: v,
    };
    match key {
        "id" => Some(attr("id", bid.id.clone())),
        "creator" => Some(attr("creator", bid.creator.to_string())),
        "collection" => Some(attr("collection", bid.collection.to_string())),
        "token_id" => Some(attr("token_id", bid.token_id.clone())),
        "price" => Some(attr("price", bid.terms.price.to_string())),
        "recipient" => bid
            .terms
            .recipient
            .as_ref()
            .map(|a| attr("recipient", a.to_string())),
        "finder" => bid
            .terms
            .finder
            .as_ref()
            .map(|a| attr("finder", a.to_string())),
        _ => None,
    }
}

fn collection_attr(bid: &CollectionBid, key: &str) -> Option<Attribute> {
    let attr = |k: &str, v: String| Attribute {
        key: k.to_string(),
        value: v,
    };
    match key {
        "id" => Some(attr("id", bid.id.clone())),
        "creator" => Some(attr("creator", bid.creator.to_string())),
        "collection" => Some(attr("collection", bid.collection.to_string())),
        "price" => Some(attr("price", bid.terms.price.to_string())),
        "recipient" => bid
            .terms
            .recipient
            .as_ref()
            .map(|a| attr("recipient", a.to_string())),
        "finder" => bid
            .terms
            .finder
            .as_ref()
            .map(|a| attr("finder", a.to_string())),
        _ => None,
    }
}
