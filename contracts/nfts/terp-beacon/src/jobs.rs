use crate::drand::round_after;
use crate::error::ContractError;
use crate::nois::{MAX_JOB_ID_LEN, NoisCallback, ReceiverExecuteMsg};
use crate::source::BeaconSource;
use crate::state::{
    ATTESTATIONS, ATTESTORS, BEACONS, Config, JOBS, QUEUE, QUEUE_MARKER, StoredBeacon, StoredJob,
};
use cosmwasm_std::{
    Addr, Coin, Order, StdResult, Storage, SubMsg, Timestamp, WasmMsg, to_json_binary,
};
use cw_storage_plus::Bound;

/// Callbacks included in one transaction. Further jobs wait for `ProcessQueue` or the next round.
pub const MAX_DELIVERIES_PER_TX: usize = 10;

/// Request time for `GetNextRandomness` is the block time plus this margin,
/// matching nois-proxy so the next drand round is not already public.
pub const SAFETY_MARGIN_NANOS: u64 = 5_000_000_000;

pub const REPLY_CALLBACK: u64 = 1;

pub fn validate_job_id(job_id: &str) -> Result<(), ContractError> {
    if job_id.len() > MAX_JOB_ID_LEN {
        Err(ContractError::JobIdTooLong)
    } else {
        Ok(())
    }
}

/// Empty `prices` means the request is free. Otherwise one listed denom must be covered.
/// Extra funds are kept by the contract.
pub fn validate_payment(prices: &[Coin], funds: &[Coin]) -> Result<(), ContractError> {
    if prices.is_empty() {
        return Ok(());
    }
    for price in prices {
        if funds
            .iter()
            .any(|fund| fund.denom == price.denom && fund.amount >= price.amount)
        {
            return Ok(());
        }
    }
    Err(ContractError::InsufficientPayment)
}

pub fn normalize_prices(prices: Vec<Coin>) -> Result<Vec<Coin>, ContractError> {
    let mut denoms: Vec<&str> = Vec::with_capacity(prices.len());
    for price in &prices {
        if price.amount.is_zero() {
            return Err(ContractError::ZeroPrice);
        }
        if denoms.iter().any(|d| *d == price.denom) {
            return Err(ContractError::DuplicatePriceDenom);
        }
        denoms.push(price.denom.as_str());
    }
    Ok(prices)
}

pub fn enqueue_job(
    storage: &mut dyn Storage,
    config: &Config,
    requester: &Addr,
    job_id: &str,
    after: Timestamp,
) -> Result<u64, ContractError> {
    if JOBS.has(storage, (requester, job_id)) {
        return Err(ContractError::JobExists);
    }
    let round = round_after(config.genesis_time, config.period_seconds, after)?;
    JOBS.save(storage, (requester, job_id), &StoredJob { after, round })?;
    QUEUE.save(storage, (round, requester, job_id), &QUEUE_MARKER)?;
    Ok(round)
}

fn attestation_count(storage: &dyn Storage, round: u64) -> StdResult<u32> {
    let mut count = 0u32;
    for item in ATTESTATIONS
        .prefix(round)
        .range(storage, None, None, Order::Ascending)
    {
        let (pubkey, _) = item?;
        if ATTESTORS.has(storage, pubkey.as_slice()) {
            count = count.saturating_add(1);
        }
    }
    Ok(count)
}

pub fn round_is_deliverable(storage: &dyn Storage, config: &Config, round: u64) -> StdResult<bool> {
    if config.required_attestations == 0 {
        return Ok(true);
    }
    Ok(attestation_count(storage, round)? >= config.required_attestations)
}

fn beacon_for_delivery(
    storage: &dyn Storage,
    config: &Config,
    round: u64,
) -> Result<Option<StoredBeacon>, ContractError> {
    let Some(beacon) = BEACONS.may_load(storage, (BeaconSource::Drand.source_id(), round))? else {
        return Ok(None);
    };
    if round_is_deliverable(storage, config, round)? {
        Ok(Some(beacon))
    } else {
        Ok(None)
    }
}

fn callback_submsg(
    config: &Config,
    requester: &Addr,
    job_id: String,
    beacon: &StoredBeacon,
) -> StdResult<SubMsg> {
    let msg = WasmMsg::Execute {
        contract_addr: requester.to_string(),
        msg: to_json_binary(&ReceiverExecuteMsg::NoisReceive {
            callback: NoisCallback {
                job_id,
                published: beacon.published,
                randomness: beacon.randomness.clone(),
            },
        })?,
        funds: vec![],
    };
    Ok(SubMsg::reply_on_error(msg, REPLY_CALLBACK).with_gas_limit(config.callback_gas_limit))
}

fn take_job(
    storage: &mut dyn Storage,
    config: &Config,
    requester: &Addr,
    job_id: &str,
) -> Result<Option<SubMsg>, ContractError> {
    let Some(job) = JOBS.may_load(storage, (requester, job_id))? else {
        return Ok(None);
    };
    let Some(beacon) = beacon_for_delivery(storage, config, job.round)? else {
        return Ok(None);
    };
    JOBS.remove(storage, (requester, job_id));
    QUEUE.remove(storage, (job.round, requester, job_id));
    Ok(Some(callback_submsg(
        config,
        requester,
        job_id.to_string(),
        &beacon,
    )?))
}

/// Deliver up to `limit` jobs committed to `round`. One beacon load, then a
/// bounded prefix read. A different stored round is never substituted.
fn deliver_round(
    storage: &mut dyn Storage,
    config: &Config,
    round: u64,
    out: &mut Vec<SubMsg>,
) -> Result<(), ContractError> {
    if out.len() >= MAX_DELIVERIES_PER_TX {
        return Ok(());
    }
    let Some(beacon) = beacon_for_delivery(storage, config, round)? else {
        return Ok(());
    };
    let room = MAX_DELIVERIES_PER_TX - out.len();
    let queued: Vec<(Addr, String)> = QUEUE
        .sub_prefix(round)
        .keys(storage, None, None, Order::Ascending)
        .take(room)
        .collect::<StdResult<Vec<_>>>()?;
    for (requester, job_id) in queued {
        JOBS.remove(storage, (&requester, job_id.as_str()));
        QUEUE.remove(storage, (round, &requester, job_id.as_str()));
        out.push(callback_submsg(config, &requester, job_id, &beacon)?);
    }
    Ok(())
}

/// `focus_round` drains only that round (AddRound / Attest). `prefer` tries the
/// job that was just enqueued, then fills the batch from its committed round.
/// With neither, walk at most [`MAX_DELIVERIES_PER_TX`] distinct queued rounds
/// and load each beacon by key.
pub fn deliver_queued(
    storage: &mut dyn Storage,
    config: &Config,
    focus_round: Option<u64>,
    prefer: Option<(&Addr, &str)>,
) -> Result<Vec<SubMsg>, ContractError> {
    let mut out = Vec::new();
    let mut prefer_round = None;
    if let Some((requester, job_id)) = prefer {
        prefer_round = JOBS
            .may_load(storage, (requester, job_id))?
            .map(|job| job.round);
        if let Some(msg) = take_job(storage, config, requester, job_id)? {
            out.push(msg);
        }
    }
    if let Some(round) = focus_round.or(prefer_round) {
        deliver_round(storage, config, round, &mut out)?;
        return Ok(out);
    }

    let mut next_round = None;
    for _ in 0..MAX_DELIVERIES_PER_TX {
        if out.len() >= MAX_DELIVERIES_PER_TX {
            break;
        }
        let lower = Addr::unchecked("");
        let min = next_round.map(|round| Bound::inclusive((round, &lower, "")));
        let next = QUEUE
            .range(storage, min, None, Order::Ascending)
            .next()
            .transpose()?;
        let Some((round, _, _)) = next.map(|(key, _)| key) else {
            break;
        };
        deliver_round(storage, config, round, &mut out)?;
        next_round = Some(
            round
                .checked_add(1)
                .ok_or(ContractError::RoundTimeOverflow)?,
        );
    }
    Ok(out)
}

pub fn count_registered_attestations(storage: &dyn Storage, round: u64) -> StdResult<u32> {
    attestation_count(storage, round)
}
