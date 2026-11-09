//! Loan harness. Offer escrows principal. Accept escrows the NFT and pays the
//! borrower. Repay returns the NFT. Default after expiry sends the NFT to the lender.

use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::{
    Addr, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdError, StdResult, Timestamp,
    Uint128, to_json_binary,
};
use cw_storage_plus::Map;

use crate::pay::{self, token_key};

#[cw_serde]
pub struct InstantiateMsg {}

#[cw_serde]
pub enum ExecuteMsg {
    Offer {
        collection: String,
        token_id: String,
        principal: Uint128,
        duration_secs: u64,
    },
    Accept {
        collection: String,
        token_id: String,
    },
    Repay {
        collection: String,
        token_id: String,
    },
    Default {
        collection: String,
        token_id: String,
    },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(Count)]
    Count {},
}

#[cw_serde]
pub struct Count {
    pub offers: u64,
}

#[cw_serde]
struct Offer {
    lender: Addr,
    principal: Uint128,
    duration_secs: u64,
}

#[cw_serde]
struct Loan {
    lender: Addr,
    borrower: Addr,
    principal: Uint128,
    expiry: Timestamp,
}

const OFFERS: Map<&str, Offer> = Map::new("offers");
const LOANS: Map<&str, Loan> = Map::new("loans");

pub fn instantiate(
    _deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    _msg: InstantiateMsg,
) -> StdResult<Response> {
    Ok(Response::new().add_attribute("action", "instantiate"))
}

pub fn execute(deps: DepsMut, env: Env, info: MessageInfo, msg: ExecuteMsg) -> StdResult<Response> {
    match msg {
        ExecuteMsg::Offer {
            collection,
            token_id,
            principal,
            duration_secs,
        } => offer(deps, info, collection, token_id, principal, duration_secs),
        ExecuteMsg::Accept {
            collection,
            token_id,
        } => accept(deps, env, info, collection, token_id),
        ExecuteMsg::Repay {
            collection,
            token_id,
        } => repay(deps, env, info, collection, token_id),
        ExecuteMsg::Default {
            collection,
            token_id,
        } => default_loan(deps, env, collection, token_id),
    }
}

pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Count {} => {
            let offers = OFFERS
                .keys(deps.storage, None, None, cosmwasm_std::Order::Ascending)
                .count() as u64;
            to_json_binary(&Count { offers })
        }
    }
}

fn offer(
    deps: DepsMut,
    info: MessageInfo,
    collection: String,
    token_id: String,
    principal: Uint128,
    duration_secs: u64,
) -> StdResult<Response> {
    if principal.is_zero() || duration_secs == 0 {
        return Err(StdError::msg("principal and duration must be set"));
    }
    pay::must_pay_exact(&info, principal)?;
    let key = token_key(&collection, &token_id);
    if OFFERS.may_load(deps.storage, &key)?.is_some() || LOANS.has(deps.storage, &key) {
        return Err(StdError::msg("offer or loan exists"));
    }
    OFFERS.save(
        deps.storage,
        &key,
        &Offer {
            lender: info.sender,
            principal,
            duration_secs,
        },
    )?;
    Ok(Response::new().add_attribute("action", "offer"))
}

fn accept(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    collection: String,
    token_id: String,
) -> StdResult<Response> {
    let owner = pay::owner_of(deps.as_ref(), &collection, &token_id)?;
    if owner != info.sender {
        return Err(StdError::msg("not the owner"));
    }
    let key = token_key(&collection, &token_id);
    let offer = OFFERS.load(deps.storage, &key)?;
    let transfer = pay::transfer(&collection, &token_id, &env.contract.address)?;
    let pay_borrower = pay::send(&info.sender, offer.principal)
        .ok_or_else(|| StdError::msg("principal is zero"))?;
    LOANS.save(
        deps.storage,
        &key,
        &Loan {
            lender: offer.lender,
            borrower: info.sender,
            principal: offer.principal,
            expiry: env.block.time.plus_seconds(offer.duration_secs),
        },
    )?;
    OFFERS.remove(deps.storage, &key);
    Ok(Response::new()
        .add_message(transfer)
        .add_message(pay_borrower)
        .add_attribute("action", "accept"))
}

fn repay(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    collection: String,
    token_id: String,
) -> StdResult<Response> {
    let key = token_key(&collection, &token_id);
    let loan = LOANS.load(deps.storage, &key)?;
    if info.sender != loan.borrower {
        return Err(StdError::msg("not the borrower"));
    }
    if env.block.time >= loan.expiry {
        return Err(StdError::msg("loan expired"));
    }
    pay::must_pay_exact(&info, loan.principal)?;
    let transfer = pay::transfer(&collection, &token_id, &loan.borrower)?;
    let to_lender = pay::send(&loan.lender, loan.principal)
        .ok_or_else(|| StdError::msg("principal is zero"))?;
    LOANS.remove(deps.storage, &key);
    Ok(Response::new()
        .add_message(transfer)
        .add_message(to_lender)
        .add_attribute("action", "repay"))
}

fn default_loan(
    deps: DepsMut,
    env: Env,
    collection: String,
    token_id: String,
) -> StdResult<Response> {
    let key = token_key(&collection, &token_id);
    let loan = LOANS.load(deps.storage, &key)?;
    if env.block.time < loan.expiry {
        return Err(StdError::msg("loan still current"));
    }
    let transfer = pay::transfer(&collection, &token_id, &loan.lender)?;
    LOANS.remove(deps.storage, &key);
    Ok(Response::new()
        .add_message(transfer)
        .add_attribute("action", "default"))
}

#[cw_orch::interface(
    InstantiateMsg,
    ExecuteMsg,
    QueryMsg,
    cosmwasm_std::Empty,
    id = "nftfi_bench_loan"
)]
pub struct BenchLoan;

impl<Chain: cw_orch::prelude::CwEnv> cw_orch::prelude::Uploadable for BenchLoan<Chain> {
    fn wasm(_chain: &cw_orch::prelude::ChainInfoOwned) -> cw_orch::prelude::WasmPath {
        panic!("nftfi-bench loan harness has no wasm");
    }

    fn wrapper() -> Box<dyn cw_orch::prelude::MockContract<cosmwasm_std::Empty>> {
        Box::new(cw_orch::prelude::ContractWrapper::new_with_empty(
            execute,
            instantiate,
            query,
        ))
    }
}
