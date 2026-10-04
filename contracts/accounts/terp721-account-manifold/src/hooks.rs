use terp_account::{
    manifold::hooks::{AskHookMsg, BidHookMsg, HookAction, SaleHookMsg},
    Ask, Bid,
};

use cosmwasm_std::{
    Addr, DepsMut, Env, Reply, Response, StdError, StdResult, Storage, SubMsg, WasmMsg,
};

use crate::{state::*, ContractError};

enum HookReply {
    Ask = 1,
    Sale,
    Bid,
}

impl From<u64> for HookReply {
    fn from(item: u64) -> Self {
        match item {
            1 => HookReply::Ask,
            2 => HookReply::Sale,
            3 => HookReply::Bid,
            _ => panic!("invalid reply type"),
        }
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn reply(_deps: DepsMut, _env: Env, msg: Reply) -> Result<Response, ContractError> {
    match HookReply::from(msg.id) {
        HookReply::Ask => {
            return Err(ContractError::Std(StdError::msg(
                msg.result.unwrap_err(),
            )))
        }
        HookReply::Sale => {
            return Err(ContractError::Std(StdError::msg(
                msg.result.unwrap_err(),
            )))
        }
        HookReply::Bid => {
            return Err(ContractError::Std(StdError::msg(
                msg.result.unwrap_err(),
            )))
        }
    }
}

pub fn prepare_ask_hook(
    storage: &dyn Storage,
    ask: &Ask,
    action: HookAction,
) -> StdResult<Vec<SubMsg>> {
    let submsgs = ASK_HOOKS.prepare_hooks(storage, |h| {
        let msg = AskHookMsg { ask: ask.clone() };
        let execute = WasmMsg::Execute {
            contract_addr: h.to_string(),
            msg: msg.into_json_binary(action.clone())?,
            funds: vec![],
        };
        Ok(SubMsg::reply_on_error(execute, 1u64))
    })?;

    Ok(submsgs)
}

pub fn prepare_sale_hook(storage: &dyn Storage, ask: &Ask, buyer: Addr) -> StdResult<Vec<SubMsg>> {
    let submsgs = SALE_HOOKS.prepare_hooks(storage, |h| {
        let msg = SaleHookMsg {
            token_id: ask.token_id.to_string(),
            ask_id: ask.id,
            seller: ask.seller.to_string(),
            buyer: buyer.to_string(),
        };
        let execute = WasmMsg::Execute {
            contract_addr: h.to_string(),
            msg: msg.into_json_binary()?,
            funds: vec![],
        };
        Ok(SubMsg::reply_on_error(execute, HookReply::Sale as u64))
    })?;

    Ok(submsgs)
}

pub fn prepare_bid_hook(
    storage: &dyn Storage,
    bid: &Bid,
    action: HookAction,
) -> StdResult<Vec<SubMsg>> {
    let submsgs = BID_HOOKS.prepare_hooks(storage, |h| {
        let msg = BidHookMsg { bid: bid.clone() };
        let execute = WasmMsg::Execute {
            contract_addr: h.to_string(),
            msg: msg.into_json_binary(action.clone())?,
            funds: vec![],
        };
        Ok(SubMsg::reply_on_error(execute, HookReply::Bid as u64))
    })?;

    Ok(submsgs)
}
