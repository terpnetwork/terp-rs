//! Emergency reclaim (spec Appendix R).
//!
//! Before the cutoff, x/gov (which is both the cw-ownable owner and the wasm admin)
//! can return every remaining escrowed coin to the community pool. The contract then
//! enters a terminal `reclaimed` state: every execute and sudo is refused, and the
//! claim/registration dry-run queries report the halt. After the cutoff only the burn
//! sweep (`SweepUnclaimed`) exists; reclaim is refused.
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Coin, CosmosMsg, DistributionMsg, DepsMut, Env, MessageInfo, Response, StdError, StdResult, Storage};
use cw_storage_plus::Item;

use crate::epoch::EPOCH_CAMPAIGN;
use crate::HEADSTASH_CFG;

#[cw_serde]
pub struct ReclaimRecord {
    pub height: u64,
    /// Coins sent to the community pool (empty when the escrow was already empty).
    pub amounts: Vec<Coin>,
}

pub const RECLAIMED: Item<ReclaimRecord> = Item::new("emergency_reclaimed");

pub const HALTED: &str = "headstash reclaimed by governance: claims and registration are halted";

pub fn ensure_live(storage: &dyn Storage) -> StdResult<()> {
    if RECLAIMED.may_load(storage)?.is_some() {
        return Err(StdError::msg(HALTED));
    }
    Ok(())
}

pub fn query_status(storage: &dyn Storage) -> StdResult<Option<ReclaimRecord>> {
    RECLAIMED.may_load(storage)
}

pub fn emergency_reclaim(deps: DepsMut, env: Env, info: MessageInfo) -> Result<Response, StdError> {
    ensure_live(deps.storage)?;
    // Owner (cw-ownable) AND wasm admin: both are x/gov on mainnet, so only a
    // passed proposal can satisfy this. Neither alone is enough.
    cw_ownable::assert_owner(deps.storage, &info.sender)?;
    let ci = deps.querier.query_wasm_contract_info(env.contract.address.as_str())?;
    if ci.admin.as_ref() != Some(&info.sender) {
        return Err(StdError::msg("emergency reclaim requires the contract admin (x/gov)"));
    }
    if let Some(c) = EPOCH_CAMPAIGN.may_load(deps.storage)? {
        // Same boundary as claims: open while height <= cutoff. After it, only the burn sweep.
        if env.block.height > c.cutoff_height {
            return Err(StdError::msg("campaign has ended: only the burn sweep (SweepUnclaimed) is allowed"));
        }
    }
    let cfg = HEADSTASH_CFG.load(deps.storage)?;
    let mut amounts: Vec<Coin> = vec![];
    for ts in cfg.ts.iter() {
        let denom = ts.denom(&env.contract.address);
        if amounts.iter().any(|c| c.denom == denom) {
            continue;
        }
        let bal = deps.querier.query_balance(env.contract.address.as_str(), denom)?;
        if !bal.amount.is_zero() {
            amounts.push(bal);
        }
    }
    RECLAIMED.save(deps.storage, &ReclaimRecord { height: env.block.height, amounts: amounts.clone() })?;
    let total: Vec<String> = amounts.iter().map(|c| c.to_string()).collect();
    let mut res = Response::new()
        .add_attribute("action", "emergency_reclaim")
        .add_attribute("destination", "community_pool")
        .add_attribute("amount", total.join(","));
    if !amounts.is_empty() {
        res = res.add_message(CosmosMsg::Distribution(DistributionMsg::FundCommunityPool { amount: amounts }));
    }
    Ok(res)
}
