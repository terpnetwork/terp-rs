//! E2E-only registration hook for cw-headstash (tools/headstash/e2e). Records every
//! `headstash_registered` call from the configured headstash contract; `set_fail` makes it
//! return an error so the e2e can prove a failing hook never blocks registration.
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{entry_point, to_json_binary, Addr, Binary, Deps, DepsMut, Env, MessageInfo, Order, Response, StdError, StdResult};
use cw_storage_plus::{Item, Map};

#[cw_serde]
pub struct InstantiateMsg {
    /// Only this contract may call `headstash_registered` (optional for the e2e).
    pub source: Option<String>,
}

#[cw_serde]
pub enum ExecuteMsg {
    HeadstashRegistered { campaign: String, id_tag: u8, identity: String, note_count: u32, reward_addr: Option<String> },
    SetFail { fail: bool },
}

#[cw_serde]
pub enum QueryMsg {
    Registrants {},
}

#[cw_serde]
pub struct Registrant {
    pub caller: Addr,
    pub campaign: String,
    pub id_tag: u8,
    pub identity: String,
    pub note_count: u32,
    pub reward_addr: Option<String>,
    pub height: u64,
    pub funds: usize,
}

const ADMIN: Item<Addr> = Item::new("admin");
const SOURCE: Item<Option<Addr>> = Item::new("source");
const FAIL: Item<bool> = Item::new("fail");
const SEQ: Item<u32> = Item::new("seq");
const REGS: Map<u32, Registrant> = Map::new("regs");

#[entry_point]
pub fn instantiate(deps: DepsMut, _env: Env, info: MessageInfo, msg: InstantiateMsg) -> StdResult<Response> {
    ADMIN.save(deps.storage, &info.sender)?;
    let src = msg.source.map(|s| deps.api.addr_validate(&s)).transpose()?;
    SOURCE.save(deps.storage, &src)?;
    FAIL.save(deps.storage, &false)?;
    SEQ.save(deps.storage, &0)?;
    Ok(Response::new())
}

#[entry_point]
pub fn execute(deps: DepsMut, env: Env, info: MessageInfo, msg: ExecuteMsg) -> StdResult<Response> {
    match msg {
        ExecuteMsg::SetFail { fail } => {
            if info.sender != ADMIN.load(deps.storage)? {
                return Err(StdError::msg("unauthorized"));
            }
            FAIL.save(deps.storage, &fail)?;
            Ok(Response::new().add_attribute("fail", fail.to_string()))
        }
        ExecuteMsg::HeadstashRegistered { campaign, id_tag, identity, note_count, reward_addr } => {
            if FAIL.load(deps.storage)? {
                return Err(StdError::msg("hook-recorder: told to fail"));
            }
            if let Some(src) = SOURCE.load(deps.storage)? {
                if info.sender != src {
                    return Err(StdError::msg("unknown source"));
                }
            }
            let n = SEQ.load(deps.storage)?;
            REGS.save(deps.storage, n, &Registrant {
                caller: info.sender, campaign, id_tag, identity, note_count, reward_addr,
                height: env.block.height, funds: info.funds.len(),
            })?;
            SEQ.save(deps.storage, &(n + 1))?;
            Ok(Response::new().add_attribute("recorded", n.to_string()))
        }
    }
}

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Registrants {} => {
            let v: Vec<Registrant> = REGS.range(deps.storage, None, None, Order::Ascending).map(|r| r.map(|x| x.1)).collect::<StdResult<_>>()?;
            to_json_binary(&v)
        }
    }
}
