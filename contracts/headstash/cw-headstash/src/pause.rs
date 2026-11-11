//! Pauser: fast halt/resume of claims and registration (spec Appendix P).
//! The pauser can only set or clear `PAUSED`. It has no funds or config power.
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Api, DepsMut, Env, MessageInfo, Response, StdError, StdResult, Storage};
use cw_storage_plus::Item;

#[cw_serde]
pub struct PauseRecord {
    pub height: u64,
    pub by: Addr,
}

#[cw_serde]
pub struct PauseStatusResponse {
    pub pauser: Option<Addr>,
    pub paused: bool,
    pub paused_at_height: Option<u64>,
    pub paused_by: Option<Addr>,
}

pub const PAUSER: Item<Addr> = Item::new("pauser");
pub const PAUSED: Item<PauseRecord> = Item::new("paused");

pub const PAUSED_ERR: &str = "headstash claims are paused";

fn validate(api: &dyn Api, p: Option<String>) -> StdResult<Option<Addr>> {
    match p {
        None => Ok(None),
        Some(s) if s.trim().is_empty() => Err(StdError::msg("pauser must not be empty")),
        Some(s) => Ok(Some(api.addr_validate(&s)?)),
    }
}

pub fn save_pauser(
    storage: &mut dyn Storage,
    api: &dyn Api,
    p: Option<String>,
) -> StdResult<Option<Addr>> {
    let a = validate(api, p)?;
    match &a {
        Some(a) => PAUSER.save(storage, a)?,
        None => PAUSER.remove(storage),
    }
    Ok(a)
}

pub fn ensure_not_paused(storage: &dyn Storage) -> StdResult<()> {
    if PAUSED.may_load(storage)?.is_some() {
        return Err(StdError::msg(PAUSED_ERR));
    }
    Ok(())
}

fn assert_pauser_or_owner(storage: &dyn Storage, sender: &Addr) -> StdResult<()> {
    if PAUSER.may_load(storage)?.as_ref() == Some(sender)
        || cw_ownable::assert_owner(storage, sender).is_ok()
    {
        return Ok(());
    }
    Err(StdError::msg(
        "unauthorized: only the pauser or the owner can pause/resume",
    ))
}

pub fn pause(deps: DepsMut, env: Env, info: MessageInfo) -> Result<Response, StdError> {
    assert_pauser_or_owner(deps.storage, &info.sender)?;
    if PAUSED.may_load(deps.storage)?.is_some() {
        return Err(StdError::msg("already paused"));
    }
    PAUSED.save(
        deps.storage,
        &PauseRecord {
            height: env.block.height,
            by: info.sender.clone(),
        },
    )?;
    Ok(Response::new()
        .add_attribute("action", "pause")
        .add_attribute("by", info.sender)
        .add_attribute("height", env.block.height.to_string()))
}

pub fn resume(deps: DepsMut, env: Env, info: MessageInfo) -> Result<Response, StdError> {
    assert_pauser_or_owner(deps.storage, &info.sender)?;
    if PAUSED.may_load(deps.storage)?.is_none() {
        return Err(StdError::msg("not paused"));
    }
    PAUSED.remove(deps.storage);
    Ok(Response::new()
        .add_attribute("action", "resume")
        .add_attribute("by", info.sender)
        .add_attribute("height", env.block.height.to_string()))
}

pub fn update_pauser(
    deps: DepsMut,
    info: MessageInfo,
    pauser: Option<String>,
) -> Result<Response, StdError> {
    cw_ownable::assert_owner(deps.storage, &info.sender)?;
    let a = save_pauser(deps.storage, deps.api, pauser)?;
    Ok(Response::new()
        .add_attribute("action", "update_pauser")
        .add_attribute(
            "pauser",
            a.map(|a| a.to_string()).unwrap_or_else(|| "none".into()),
        ))
}

pub fn query_status(storage: &dyn Storage) -> StdResult<PauseStatusResponse> {
    let p = PAUSED.may_load(storage)?;
    Ok(PauseStatusResponse {
        pauser: PAUSER.may_load(storage)?,
        paused: p.is_some(),
        paused_at_height: p.as_ref().map(|r| r.height),
        paused_by: p.map(|r| r.by),
    })
}
