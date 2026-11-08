//! Shared bank and cw721 helpers for the harness contracts.

use cosmwasm_std::{
    Addr, BankMsg, Coin, CosmosMsg, Deps, MessageInfo, StdError, StdResult, Uint128, Uint256,
    WasmMsg, to_json_binary,
};
use cw721::msg::OwnerOfResponse;
use cw721_base::msg::{ExecuteMsg as NftExecute, QueryMsg as NftQuery};

pub const DENOM: &str = "uterp";

pub fn token_key(collection: &str, token_id: &str) -> String {
    format!("{collection}/{token_id}")
}

pub fn must_pay_exact(info: &MessageInfo, amount: Uint128) -> StdResult<()> {
    let mut got = Uint256::zero();
    for coin in &info.funds {
        if coin.denom != DENOM {
            return Err(StdError::msg(format!("unexpected denom {}", coin.denom)));
        }
        got += coin.amount;
    }
    let expected = Uint256::from(amount);
    if got != expected {
        return Err(StdError::msg(format!(
            "expected {expected}{DENOM}, got {got}"
        )));
    }
    Ok(())
}

pub fn owner_of(deps: Deps, collection: &str, token_id: &str) -> StdResult<Addr> {
    let resp: OwnerOfResponse = deps.querier.query_wasm_smart(
        collection,
        &NftQuery::OwnerOf {
            token_id: token_id.to_string(),
            include_expired: None,
        },
    )?;
    deps.api.addr_validate(&resp.owner)
}

pub fn transfer(collection: &str, token_id: &str, recipient: &Addr) -> StdResult<CosmosMsg> {
    Ok(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: collection.to_string(),
        msg: to_json_binary(&NftExecute::TransferNft {
            recipient: recipient.to_string(),
            token_id: token_id.to_string(),
        })?,
        funds: vec![],
    }))
}

pub fn send(to: &Addr, amount: Uint128) -> Option<CosmosMsg> {
    if amount.is_zero() {
        None
    } else {
        Some(CosmosMsg::Bank(BankMsg::Send {
            to_address: to.to_string(),
            amount: vec![Coin {
                denom: DENOM.to_string(),
                amount: Uint256::from(amount),
            }],
        }))
    }
}

/// Fee and royalty round down. The remainder, including dust, goes to `seller`.
pub fn split(
    price: Uint128,
    fee_bps: u16,
    royalty_bps: u16,
) -> StdResult<(Uint128, Uint128, Uint128)> {
    let bps = u32::from(fee_bps) + u32::from(royalty_bps);
    if bps > 10_000 {
        return Err(StdError::msg("fee_bps + royalty_bps exceeds 10000"));
    }
    let fee = price.multiply_ratio(u128::from(fee_bps), 10_000u128);
    let royalty = price.multiply_ratio(u128::from(royalty_bps), 10_000u128);
    let seller = price.checked_sub(fee)?.checked_sub(royalty)?;
    Ok((seller, fee, royalty))
}

pub fn payout(
    seller: &Addr,
    fee_addr: &Addr,
    royalty_addr: &Addr,
    price: Uint128,
    fee_bps: u16,
    royalty_bps: u16,
) -> StdResult<Vec<CosmosMsg>> {
    let (seller_amt, fee, royalty) = split(price, fee_bps, royalty_bps)?;
    let mut msgs = Vec::new();
    if let Some(msg) = send(seller, seller_amt) {
        msgs.push(msg);
    }
    if let Some(msg) = send(fee_addr, fee) {
        msgs.push(msg);
    }
    if let Some(msg) = send(royalty_addr, royalty) {
        msgs.push(msg);
    }
    Ok(msgs)
}
