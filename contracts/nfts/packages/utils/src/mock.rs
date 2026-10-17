//! In-memory cw721 used by raffle and loan tests.
//!
//! Execute and query enums are the real `cw721` trait messages, so a passing
//! test is the same wire call `cw721-svg` accepts for mint, approve, transfer,
//! and owner lookup.

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{
    to_json_binary, Addr, Binary, Deps, DepsMut, Empty, Env, MessageInfo, Response, StdError,
    StdResult,
};
use cw721::msg::{Cw721ExecuteMsg, Cw721QueryMsg, OperatorResponse, OwnerOfResponse};
use cw721::{Approval, Expiration};
use cw_multi_test::{Contract, ContractWrapper};
use cw_storage_plus::{Item, Map};

use crate::Cw721Token;

const MINTER: Item<Addr> = Item::new("minter");
const TOKENS: Map<String, TokenState> = Map::new("tokens");
const OPERATORS: Map<(Addr, Addr), Expiration> = Map::new("operators");

#[cw_serde]
pub struct InstantiateMsg {
    pub minter: String,
}

#[cw_serde]
struct TokenState {
    owner: Addr,
    approvals: Vec<Approval>,
}

pub fn contract() -> Box<dyn Contract<Empty>> {
    Box::new(ContractWrapper::new_with_empty(execute, instantiate, query))
}

pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, StdError> {
    MINTER.save(deps.storage, &deps.api.addr_validate(&msg.minter)?)?;
    Ok(Response::new().add_attribute("action", "instantiate"))
}

pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: Cw721ExecuteMsg<Empty, Empty, Empty>,
) -> Result<Response, StdError> {
    match msg {
        Cw721ExecuteMsg::Mint {
            token_id,
            owner,
            token_uri: _,
            extension: _,
        } => {
            let minter = MINTER.load(deps.storage)?;
            if info.sender != minter {
                return Err(StdError::msg("not minter"));
            }
            if TOKENS.may_load(deps.storage, token_id.clone())?.is_some() {
                return Err(StdError::msg("token exists"));
            }
            TOKENS.save(
                deps.storage,
                token_id.clone(),
                &TokenState {
                    owner: deps.api.addr_validate(&owner)?,
                    approvals: vec![],
                },
            )?;
            Ok(Response::new()
                .add_attribute("action", "mint")
                .add_attribute("token_id", token_id))
        }
        Cw721ExecuteMsg::Approve {
            spender,
            token_id,
            expires,
        } => {
            let mut token = load_token(deps.as_ref(), &token_id)?;
            if info.sender != token.owner {
                return Err(StdError::msg("not owner"));
            }
            let spender = deps.api.addr_validate(&spender)?;
            let expires = expires.unwrap_or_default();
            if expires.is_expired(&env.block) {
                return Err(StdError::msg("expired"));
            }
            token.approvals.retain(|a| a.spender != spender);
            token.approvals.push(Approval {
                spender: spender.clone(),
                expires,
            });
            TOKENS.save(deps.storage, token_id.clone(), &token)?;
            Ok(Response::new()
                .add_attribute("action", "approve")
                .add_attribute("spender", spender)
                .add_attribute("token_id", token_id))
        }
        Cw721ExecuteMsg::ApproveAll { operator, expires } => {
            let operator = deps.api.addr_validate(&operator)?;
            let expires = expires.unwrap_or_default();
            if expires.is_expired(&env.block) {
                return Err(StdError::msg("expired"));
            }
            OPERATORS.save(deps.storage, (info.sender, operator.clone()), &expires)?;
            Ok(Response::new()
                .add_attribute("action", "approve_all")
                .add_attribute("operator", operator))
        }
        Cw721ExecuteMsg::TransferNft {
            recipient,
            token_id,
        } => {
            let mut token = load_token(deps.as_ref(), &token_id)?;
            if !can_send(deps.as_ref(), &env, &info.sender, &token)? {
                return Err(StdError::msg("unauthorized transfer"));
            }
            token.owner = deps.api.addr_validate(&recipient)?;
            token.approvals.clear();
            TOKENS.save(deps.storage, token_id.clone(), &token)?;
            Ok(Response::new()
                .add_attribute("action", "transfer_nft")
                .add_attribute("recipient", token.owner)
                .add_attribute("token_id", token_id))
        }
        _ => Err(StdError::msg("cw721 message is not implemented in the test double")),
    }
}

pub fn query(deps: Deps, env: Env, msg: Cw721QueryMsg<Empty, Empty, Empty>) -> StdResult<Binary> {
    match msg {
        Cw721QueryMsg::OwnerOf {
            token_id,
            include_expired,
        } => {
            let token = load_token(deps, &token_id)?;
            let include_expired = include_expired.unwrap_or(false);
            let approvals = token
                .approvals
                .into_iter()
                .filter(|a| include_expired || !a.is_expired(&env.block))
                .collect();
            to_json_binary(&OwnerOfResponse {
                owner: token.owner.into_string(),
                approvals,
            })
        }
        Cw721QueryMsg::Operator {
            owner,
            operator,
            include_expired,
        } => {
            let owner = deps.api.addr_validate(&owner)?;
            let operator = deps.api.addr_validate(&operator)?;
            let expires = OPERATORS
                .may_load(deps.storage, (owner.clone(), operator.clone()))?
                .ok_or_else(|| StdError::msg("operator not found"))?;
            let include_expired = include_expired.unwrap_or(false);
            if !include_expired && expires.is_expired(&env.block) {
                return Err(StdError::msg("operator expired"));
            }
            to_json_binary(&OperatorResponse {
                approval: Approval {
                    spender: operator,
                    expires,
                },
            })
        }
        _ => Err(StdError::msg("cw721 query is not implemented in the test double")),
    }
}

fn load_token(deps: Deps, token_id: &str) -> StdResult<TokenState> {
    TOKENS
        .may_load(deps.storage, token_id.to_string())?
        .ok_or_else(|| StdError::msg("token not found"))
}

fn can_send(deps: Deps, env: &Env, sender: &Addr, token: &TokenState) -> StdResult<bool> {
    if token.owner == sender {
        return Ok(true);
    }
    if token
        .approvals
        .iter()
        .any(|a| a.spender == sender && !a.is_expired(&env.block))
    {
        return Ok(true);
    }
    match OPERATORS.may_load(deps.storage, (token.owner.clone(), sender.clone()))? {
        Some(expires) if !expires.is_expired(&env.block) => Ok(true),
        _ => Ok(false),
    }
}

/// Convenience constructor used by tests that build [`crate::AssetInfo`].
pub fn token(address: &Addr, token_id: &str) -> Cw721Token {
    Cw721Token {
        address: address.to_string(),
        token_id: token_id.to_string(),
    }
}
