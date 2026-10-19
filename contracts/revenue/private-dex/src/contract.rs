//! Entry points.
//!
//! Settle flow mirrors `crates/dex/contracts/pair` `swap`:
//! 1. load pool / reject if paused
//! 2. orient offer/ask reserves
//! 3. host curve recompute (`quote_exact_in` ≈ `compute_swap`)
//! 4. assert min_out / oracle bounds (≈ `assert_max_spread`)
//! 5. update reserves + emit events
//!
//! Differences: proof gate via `proof_instance_verify` (or mock), pool-spend
//! nullifiers, private cm_out attributes instead of bank/CW20 transfer.

use cosmwasm_std::{
    Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult, Uint128, to_json_binary,
};
use cw_ownable::{assert_owner, initialize_owner};

use crate::error::ContractError;
use crate::msg::{
    ConfigResponse, ExecuteMsg, InstantiateMsg, Pool, PoolStatus, QueryMsg, QuoteResponse,
    SwapStatementPublic,
};
use crate::seams::{
    OracleBoundParamsView, OracleMidView, apply_reserves, check_oracle_bound, quote_exact_in,
};
use crate::state::{CONFIG, Config, NULLIFIERS, POOLS, TREE_LEAVES, pool_active};
use crate::verify::verify_swap_proof;

pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    initialize_owner(deps.storage, deps.api, Some(info.sender.as_str()))?;
    CONFIG.save(
        deps.storage,
        &Config::default_new(msg.mock_verify, msg.zkid, msg.allowed_root),
    )?;
    TREE_LEAVES.save(deps.storage, &0u64)?;
    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("mock_verify", msg.mock_verify.to_string()))
}

pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::CreatePool {
            asset_a,
            asset_b,
            r_a,
            r_b,
            gamma,
            gamma_den,
        } => execute_create_pool(deps, info, asset_a, asset_b, r_a, r_b, gamma, gamma_den),
        ExecuteMsg::SetPoolStatus { pool_id, status } => {
            execute_set_pool_status(deps, info, pool_id, status)
        }
        ExecuteMsg::SetZkCfg { mock_verify, zkid } => {
            execute_set_zk_cfg(deps, info, mock_verify, zkid)
        }
        ExecuteMsg::SetAllowedRoot { root } => execute_set_allowed_root(deps, info, root),
        ExecuteMsg::SettleSwap { statement, proof } => {
            execute_settle_swap(deps, env, info, statement, proof)
        }
    }
}

fn execute_create_pool(
    deps: DepsMut,
    info: MessageInfo,
    asset_a: Binary,
    asset_b: Binary,
    r_a: Uint128,
    r_b: Uint128,
    gamma: u64,
    gamma_den: u64,
) -> Result<Response, ContractError> {
    assert_owner(deps.storage, &info.sender)?;
    if r_a.is_zero() || r_b.is_zero() || gamma_den == 0 || asset_a == asset_b {
        return Err(ContractError::BadAmount {});
    }
    let mut cfg = CONFIG.load(deps.storage)?;
    let pool_id = cfg.next_pool_id;
    let pool = Pool {
        pool_id,
        asset_a,
        asset_b,
        r_a,
        r_b,
        gamma,
        gamma_den,
        status: PoolStatus::Active,
    };
    POOLS.save(deps.storage, pool_id, &pool)?;
    cfg.next_pool_id = pool_id.checked_add(1).ok_or(ContractError::BadAmount {})?;
    CONFIG.save(deps.storage, &cfg)?;
    Ok(Response::new()
        .add_attribute("action", "create_pool")
        .add_attribute("pool_id", pool_id.to_string()))
}

fn execute_set_pool_status(
    deps: DepsMut,
    info: MessageInfo,
    pool_id: u64,
    status: PoolStatus,
) -> Result<Response, ContractError> {
    assert_owner(deps.storage, &info.sender)?;
    let mut pool = POOLS
        .may_load(deps.storage, pool_id)?
        .ok_or(ContractError::PoolPaused {})?;
    pool.status = status;
    POOLS.save(deps.storage, pool_id, &pool)?;
    Ok(Response::new()
        .add_attribute("action", "set_pool_status")
        .add_attribute("pool_id", pool_id.to_string()))
}

fn execute_set_zk_cfg(
    deps: DepsMut,
    info: MessageInfo,
    mock_verify: bool,
    zkid: Option<u64>,
) -> Result<Response, ContractError> {
    assert_owner(deps.storage, &info.sender)?;
    let mut cfg = CONFIG.load(deps.storage)?;
    cfg.mock_verify = mock_verify;
    cfg.zkid = zkid;
    CONFIG.save(deps.storage, &cfg)?;
    Ok(Response::new().add_attribute("action", "set_zk_cfg"))
}

fn execute_set_allowed_root(
    deps: DepsMut,
    info: MessageInfo,
    root: Option<Binary>,
) -> Result<Response, ContractError> {
    assert_owner(deps.storage, &info.sender)?;
    let mut cfg = CONFIG.load(deps.storage)?;
    cfg.allowed_root = root;
    CONFIG.save(deps.storage, &cfg)?;
    Ok(Response::new().add_attribute("action", "set_allowed_root"))
}

/// Product settle path — private analog of astroport pair `swap`.
fn execute_settle_swap(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    statement: SwapStatementPublic,
    proof: Binary,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    let mut pool = POOLS
        .may_load(deps.storage, statement.pool_id)?
        .ok_or(ContractError::PoolPaused {})?;

    if !pool_active(&pool.status) {
        return Err(ContractError::PoolPaused {});
    }

    // Root window (stub) — pure apply_swap_action allowed_root check.
    if let Some(ref allowed) = cfg.allowed_root {
        if &statement.root != allowed {
            return Err(ContractError::BadRoot {});
        }
    }

    // Orient reserves (astroport: offer_pool / ask_pool from pair assets).
    let (r_in, r_out, asset_in_is_a) =
        orient_reserves(&pool, &statement.asset_in, &statement.asset_out)?;

    if r_in != statement.r_in_before.u128() || r_out != statement.r_out_before.u128() {
        return Err(ContractError::CurveMismatch {});
    }
    if pool.gamma != statement.gamma || pool.gamma_den != statement.gamma_den {
        return Err(ContractError::BadAmount {});
    }
    if statement.delta_r_in.is_zero() || statement.gamma_den == 0 {
        return Err(ContractError::BadAmount {});
    }
    if statement.nullifiers.is_empty() {
        return Err(ContractError::BadAmount {});
    }

    // Proof gate (mock or proof_instance_verify).
    verify_swap_proof(deps.api, &cfg, &statement, &proof)?;

    // Nullifier double-spend (before mutate).
    for nf in &statement.nullifiers {
        if nf.is_empty() {
            return Err(ContractError::BadAmount {});
        }
        if NULLIFIERS
            .may_load(deps.storage, nf.as_slice())?
            .unwrap_or(false)
        {
            return Err(ContractError::NullifierExists {});
        }
    }

    // Host curve recompute — must match statement.delta_r_out.
    let delta_out = quote_exact_in(
        r_in,
        r_out,
        statement.delta_r_in.u128(),
        statement.gamma,
        statement.gamma_den,
    )?;
    if delta_out != statement.delta_r_out.u128() {
        return Err(ContractError::CurveMismatch {});
    }
    if delta_out < statement.min_out.u128() {
        return Err(ContractError::MinOut {});
    }

    // Oracle bounds only (SPEC §3).
    if let Some(ref params) = statement.oracle_params {
        let mid_view = statement.oracle_mid.as_ref().map(|m| OracleMidView {
            mid: m.mid.u128(),
            observed_height: m.observed_height,
        });
        let params_view = OracleBoundParamsView {
            max_age_blocks: params.max_age_blocks,
            max_slippage_bps: params.max_slippage_bps,
            require_oracle: params.require_oracle,
        };
        check_oracle_bound(
            mid_view.as_ref(),
            &params_view,
            env.block.height,
            statement.delta_r_in.u128(),
            delta_out,
        )?;
    }

    let (r_in2, r_out2) = apply_reserves(r_in, r_out, statement.delta_r_in.u128(), delta_out)?;
    if asset_in_is_a {
        pool.r_a = Uint128::new(r_in2);
        pool.r_b = Uint128::new(r_out2);
    } else {
        pool.r_b = Uint128::new(r_in2);
        pool.r_a = Uint128::new(r_out2);
    }
    POOLS.save(deps.storage, pool.pool_id, &pool)?;

    for nf in &statement.nullifiers {
        NULLIFIERS.save(deps.storage, nf.as_slice(), &true)?;
    }

    let leaves = TREE_LEAVES.load(deps.storage)?;
    let added = statement.cm_out.len() as u64;
    TREE_LEAVES.save(deps.storage, &(leaves.saturating_add(added)))?;

    let mut res = Response::new()
        .add_attribute("action", "settle_swap")
        .add_attribute("pool_id", statement.pool_id.to_string())
        .add_attribute("delta_r_in", statement.delta_r_in.to_string())
        .add_attribute("delta_r_out", statement.delta_r_out.to_string())
        .add_attribute("nullifier_count", statement.nullifiers.len().to_string())
        .add_attribute("cm_out_count", statement.cm_out.len().to_string());

    for (i, cm) in statement.cm_out.iter().enumerate() {
        res = res.add_attribute(format!("cm_out_{i}"), cm.to_string());
    }

    Ok(res)
}

fn orient_reserves(
    pool: &Pool,
    asset_in: &Binary,
    asset_out: &Binary,
) -> Result<(u128, u128, bool), ContractError> {
    if asset_in == &pool.asset_a && asset_out == &pool.asset_b {
        Ok((pool.r_a.u128(), pool.r_b.u128(), true))
    } else if asset_in == &pool.asset_b && asset_out == &pool.asset_a {
        Ok((pool.r_b.u128(), pool.r_a.u128(), false))
    } else {
        Err(ContractError::WrongAsset {})
    }
}

pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&query_config(deps)?),
        QueryMsg::Pool { pool_id } => to_json_binary(&query_pool(deps, pool_id)?),
        QueryMsg::IsNullifierSpent { nullifier } => {
            to_json_binary(&query_nullifier(deps, nullifier)?)
        }
        QueryMsg::QuoteExactIn {
            pool_id,
            asset_in,
            delta_in,
        } => to_json_binary(&query_quote(deps, pool_id, asset_in, delta_in)?),
    }
}

fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let cfg = CONFIG.load(deps.storage)?;
    let ownership = cw_ownable::get_ownership(deps.storage)?;
    Ok(ConfigResponse {
        owner: ownership.owner.map(|a| a.to_string()),
        mock_verify: cfg.mock_verify,
        zkid: cfg.zkid,
        allowed_root: cfg.allowed_root,
        next_pool_id: cfg.next_pool_id,
        zk_api_compiled: cfg!(feature = "zk-api"),
    })
}

fn query_pool(deps: Deps, pool_id: u64) -> StdResult<Pool> {
    POOLS.load(deps.storage, pool_id)
}

fn query_nullifier(deps: Deps, nullifier: Binary) -> StdResult<bool> {
    Ok(NULLIFIERS
        .may_load(deps.storage, nullifier.as_slice())?
        .unwrap_or(false))
}

fn query_quote(
    deps: Deps,
    pool_id: u64,
    asset_in: Binary,
    delta_in: Uint128,
) -> StdResult<QuoteResponse> {
    let pool = POOLS.load(deps.storage, pool_id)?;
    let (r_in, r_out, asset_out) = if asset_in == pool.asset_a {
        (pool.r_a.u128(), pool.r_b.u128(), pool.asset_b.clone())
    } else if asset_in == pool.asset_b {
        (pool.r_b.u128(), pool.r_a.u128(), pool.asset_a.clone())
    } else {
        return Err(cosmwasm_std::StdError::msg("wrong asset"));
    };
    let delta_out = quote_exact_in(r_in, r_out, delta_in.u128(), pool.gamma, pool.gamma_den)
        .map_err(|e| cosmwasm_std::StdError::msg(e.to_string()))?;
    Ok(QuoteResponse {
        pool_id,
        asset_in,
        asset_out,
        delta_in,
        delta_out: Uint128::new(delta_out),
        r_in_before: Uint128::new(r_in),
        r_out_before: Uint128::new(r_out),
    })
}
