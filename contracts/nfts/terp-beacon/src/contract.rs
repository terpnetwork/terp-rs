use crate::attestor::verify_attestor_signature;
use crate::drand::{ensure_pubkey, round_after, round_publish_time, verify_round};
use crate::error::ContractError;
use crate::jobs::{
    self, SAFETY_MARGIN_NANOS, deliver_queued, enqueue_job, normalize_prices, validate_job_id,
    validate_payment,
};
use crate::msg::{
    AttestationInfo, AttestationsResponse, AttestorInfo, AttestorsResponse, BeaconResponse,
    ConfigResponse, ExecuteMsg, InstantiateMsg, JobInfo, JobResponse, MigrateMsg, QueryMsg,
    SourceIds,
};
use crate::source::{BeaconSource, RESERVED_IBC_SOURCE_ID};
use crate::state::{
    ATTESTATIONS, ATTESTORS, BEACONS, CONFIG, Config, DEFAULT_CALLBACK_GAS_LIMIT, JOBS,
    StoredAttestation, StoredAttestor, StoredBeacon,
};
use cosmwasm_std::{
    Addr, BankMsg, Binary, Coin, Deps, DepsMut, Env, HexBinary, MessageInfo, Order, Reply,
    Response, StdResult, Storage, SubMsgResult, Timestamp, Uint256, to_json_binary,
};
use cw2::set_contract_version;
use serde::{Deserialize, Serialize};

pub const CONTRACT_NAME: &str = "terp-beacon";
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    let admin = deps.api.addr_validate(&msg.admin)?;
    if msg.chain_hash.len() != 32 {
        return Err(ContractError::InvalidChainHash);
    }
    if msg.period_seconds == 0 {
        return Err(ContractError::InvalidPeriod);
    }
    let callback_gas_limit = msg.callback_gas_limit.unwrap_or(DEFAULT_CALLBACK_GAS_LIMIT);
    if callback_gas_limit == 0 {
        return Err(ContractError::InvalidGasLimit);
    }
    let prices = normalize_prices(msg.prices.unwrap_or_default())?;
    let config = Config {
        admin,
        chain_hash: msg.chain_hash,
        pubkey: msg.pubkey,
        genesis_time: msg.genesis_time,
        period_seconds: msg.period_seconds,
        scheme: msg.scheme,
        required_attestations: msg.required_attestations.unwrap_or(0),
        prices,
        callback_gas_limit,
    };
    ensure_pubkey(&config)?;

    CONFIG.save(deps.storage, &config)?;
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("admin", config.admin))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::AddRound {
            round,
            signature,
            previous_signature,
        } => execute_add_round(deps, env, info, round, signature, previous_signature),
        ExecuteMsg::Attest {
            round,
            pubkey,
            signature,
        } => execute_attest(deps, info, round, pubkey, signature),
        ExecuteMsg::AddAttestor { pubkey } => execute_add_attestor(deps, env, info, pubkey),
        ExecuteMsg::RemoveAttestor { pubkey } => execute_remove_attestor(deps, info, pubkey),
        ExecuteMsg::TransferAdmin { new_admin } => execute_transfer_admin(deps, info, new_admin),
        ExecuteMsg::UpdateConfig {
            required_attestations,
            prices,
            callback_gas_limit,
        } => execute_update_config(
            deps,
            info,
            required_attestations,
            prices,
            callback_gas_limit,
        ),
        ExecuteMsg::Withdraw {
            recipient,
            denom,
            amount,
        } => execute_withdraw(deps, env, info, recipient, denom, amount),
        ExecuteMsg::ProcessQueue {} => {
            reject_funds(&info)?;
            let config = CONFIG.load(deps.storage)?;
            let msgs = deliver_queued(deps.storage, &config, None, None)?;
            Ok(response_with_callbacks("process_queue", msgs))
        }
        ExecuteMsg::Provide { raffle_id, owner } => execute_provide(deps, info, raffle_id, owner),
        ExecuteMsg::GetNextRandomness { job_id } => {
            let after = env.block.time.plus_nanos(SAFETY_MARGIN_NANOS);
            execute_request_randomness(deps, info, job_id, after)
        }
        ExecuteMsg::GetRandomnessAfter { after, job_id } => {
            execute_request_randomness(deps, info, job_id, after)
        }
    }
}

fn reject_funds(info: &MessageInfo) -> Result<(), ContractError> {
    if info.funds.is_empty() {
        Ok(())
    } else {
        Err(ContractError::UnexpectedFunds)
    }
}

fn require_admin(config: &Config, sender: &Addr) -> Result<(), ContractError> {
    if *sender == config.admin {
        Ok(())
    } else {
        Err(ContractError::Unauthorized)
    }
}

fn response_with_callbacks(action: &str, msgs: Vec<cosmwasm_std::SubMsg>) -> Response {
    let delivered = msgs.len();
    Response::new()
        .add_submessages(msgs)
        .add_attribute("action", action)
        .add_attribute("jobs_delivered", delivered.to_string())
}

fn resolve_previous_signature(
    storage: &dyn Storage,
    config: &Config,
    round: u64,
    previous_signature: Option<HexBinary>,
) -> Result<Vec<u8>, ContractError> {
    match config.scheme {
        crate::msg::DrandScheme::BlsUnchainedG1Rfc9380 => {
            if previous_signature.as_ref().is_some_and(|s| !s.is_empty()) {
                return Err(ContractError::UnexpectedPreviousSignature);
            }
            Ok(Vec::new())
        }
        crate::msg::DrandScheme::PedersenBlsChained => {
            let stored_prev = if round > 1 {
                BEACONS
                    .may_load(storage, (BeaconSource::Drand.source_id(), round - 1))?
                    .map(|b| b.signature)
            } else {
                None
            };
            match (previous_signature, stored_prev) {
                (Some(given), Some(stored)) if given != stored => {
                    Err(ContractError::PreviousSignatureMismatch)
                }
                (Some(given), _) if !given.is_empty() => Ok(given.to_vec()),
                (None | Some(_), Some(stored)) => Ok(stored.to_vec()),
                (None | Some(_), None) => Err(ContractError::MissingPreviousSignature),
            }
        }
    }
}

fn execute_add_round(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    round: u64,
    signature: HexBinary,
    previous_signature: Option<HexBinary>,
) -> Result<Response, ContractError> {
    reject_funds(&info)?;
    let config = CONFIG.load(deps.storage)?;
    let previous = resolve_previous_signature(deps.storage, &config, round, previous_signature)?;
    let randomness = verify_round(&config, round, signature.as_slice(), &previous)?;
    let published = round_publish_time(config.genesis_time, config.period_seconds, round)?;
    let key = (BeaconSource::Drand.source_id(), round);
    if let Some(existing) = BEACONS.may_load(deps.storage, key)? {
        if existing.randomness != randomness {
            return Err(ContractError::BeaconConflict);
        }
    } else {
        BEACONS.save(
            deps.storage,
            key,
            &StoredBeacon {
                round,
                randomness: randomness.clone(),
                signature,
                published,
                verified_at: env.block.time,
                source: BeaconSource::Drand,
            },
        )?;
    }
    let msgs = deliver_queued(deps.storage, &config, Some(round), None)?;
    Ok(response_with_callbacks("add_round", msgs)
        .add_attribute("round", round.to_string())
        .add_attribute("randomness", randomness.to_hex())
        .add_attribute("published", published.to_string()))
}

fn execute_attest(
    deps: DepsMut,
    info: MessageInfo,
    round: u64,
    pubkey: HexBinary,
    signature: HexBinary,
) -> Result<Response, ContractError> {
    reject_funds(&info)?;
    let config = CONFIG.load(deps.storage)?;
    let beacon = BEACONS
        .may_load(deps.storage, (BeaconSource::Drand.source_id(), round))?
        .ok_or(ContractError::RoundUnknown { round })?;
    if !ATTESTORS.has(deps.storage, pubkey.as_slice()) {
        return Err(ContractError::AttestorNotRegistered);
    }
    verify_attestor_signature(
        deps.api,
        round,
        beacon.randomness.as_slice(),
        pubkey.as_slice(),
        signature.as_slice(),
    )?;
    let key = (round, pubkey.as_slice());
    if ATTESTATIONS.has(deps.storage, key) {
        return Err(ContractError::AttestationExists);
    }
    ATTESTATIONS.save(
        deps.storage,
        key,
        &StoredAttestation {
            signature,
            source: BeaconSource::Attestor,
        },
    )?;
    let msgs = deliver_queued(deps.storage, &config, Some(round), None)?;
    Ok(response_with_callbacks("attest", msgs).add_attribute("round", round.to_string()))
}

fn execute_add_attestor(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    pubkey: HexBinary,
) -> Result<Response, ContractError> {
    reject_funds(&info)?;
    let config = CONFIG.load(deps.storage)?;
    require_admin(&config, &info.sender)?;
    if pubkey.len() != crate::attestor::SECP256K1_PUBKEY_LEN
        || (pubkey[0] != 0x02 && pubkey[0] != 0x03)
    {
        return Err(ContractError::InvalidAttestorPubkey);
    }
    if ATTESTORS.has(deps.storage, pubkey.as_slice()) {
        return Err(ContractError::AttestorExists);
    }
    ATTESTORS.save(
        deps.storage,
        pubkey.as_slice(),
        &StoredAttestor {
            key_type: crate::msg::AttestorKeyType::Secp256k1,
        },
    )?;
    Ok(Response::new()
        .add_attribute("action", "add_attestor")
        .add_attribute("pubkey", pubkey.to_hex()))
}

fn execute_remove_attestor(
    deps: DepsMut,
    info: MessageInfo,
    pubkey: HexBinary,
) -> Result<Response, ContractError> {
    reject_funds(&info)?;
    let config = CONFIG.load(deps.storage)?;
    require_admin(&config, &info.sender)?;
    if !ATTESTORS.has(deps.storage, pubkey.as_slice()) {
        return Err(ContractError::AttestorNotRegistered);
    }
    ATTESTORS.remove(deps.storage, pubkey.as_slice());
    Ok(Response::new()
        .add_attribute("action", "remove_attestor")
        .add_attribute("pubkey", pubkey.to_hex()))
}

fn execute_transfer_admin(
    deps: DepsMut,
    info: MessageInfo,
    new_admin: String,
) -> Result<Response, ContractError> {
    reject_funds(&info)?;
    let mut config = CONFIG.load(deps.storage)?;
    require_admin(&config, &info.sender)?;
    let new_admin = deps.api.addr_validate(&new_admin)?;
    config.admin = new_admin.clone();
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new()
        .add_attribute("action", "transfer_admin")
        .add_attribute("admin", new_admin))
}

fn execute_update_config(
    deps: DepsMut,
    info: MessageInfo,
    required_attestations: Option<u32>,
    prices: Option<Vec<Coin>>,
    callback_gas_limit: Option<u64>,
) -> Result<Response, ContractError> {
    reject_funds(&info)?;
    let mut config = CONFIG.load(deps.storage)?;
    require_admin(&config, &info.sender)?;
    if let Some(required) = required_attestations {
        config.required_attestations = required;
    }
    if let Some(prices) = prices {
        config.prices = normalize_prices(prices)?;
    }
    if let Some(limit) = callback_gas_limit {
        if limit == 0 {
            return Err(ContractError::InvalidGasLimit);
        }
        config.callback_gas_limit = limit;
    }
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new().add_attribute("action", "update_config"))
}

fn execute_withdraw(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    recipient: Option<String>,
    denom: String,
    amount: Option<Uint256>,
) -> Result<Response, ContractError> {
    reject_funds(&info)?;
    let config = CONFIG.load(deps.storage)?;
    require_admin(&config, &info.sender)?;
    let recipient = match recipient {
        Some(addr) => deps.api.addr_validate(&addr)?,
        None => config.admin,
    };
    let balance = deps
        .querier
        .query_balance(env.contract.address, &denom)?
        .amount;
    let amount = amount.unwrap_or(balance);
    if amount.is_zero() {
        return Ok(Response::new().add_attribute("action", "withdraw"));
    }
    Ok(Response::new()
        .add_message(BankMsg::Send {
            to_address: recipient.to_string(),
            amount: vec![Coin { denom, amount }],
        })
        .add_attribute("action", "withdraw")
        .add_attribute("recipient", recipient)
        .add_attribute("amount", amount))
}

/// Query shape of `nftfi-raffle` `QueryMsg::Raffle`. Plain serde so the
/// beacon does not depend on the raffle crate.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum RaffleQuery {
    Raffle { raffle_id: u64 },
}

#[derive(Deserialize)]
struct RaffleView {
    info: RaffleInfoView,
}

#[derive(Deserialize)]
struct RaffleInfoView {
    options: RaffleOptionsView,
}

#[derive(Deserialize)]
struct RaffleOptionsView {
    start: Timestamp,
    duration: u64,
}

fn execute_provide(
    deps: DepsMut,
    info: MessageInfo,
    raffle_id: u64,
    owner: String,
) -> Result<Response, ContractError> {
    reject_funds(&info)?;
    let owner = deps.api.addr_validate(&owner)?;
    let config = CONFIG.load(deps.storage)?;
    let raffle: RaffleView = deps
        .querier
        .query_wasm_smart(&info.sender, &RaffleQuery::Raffle { raffle_id })?;
    let options = raffle.info.options;
    let end = options.start.plus_seconds(options.duration);
    let round = round_after(config.genesis_time, config.period_seconds, end)?;
    let beacon = BEACONS
        .may_load(deps.storage, (BeaconSource::Drand.source_id(), round))?
        .ok_or(ContractError::RoundUnknown { round })?;
    if !jobs::round_is_deliverable(deps.storage, &config, round)? {
        return Err(ContractError::RoundNotDeliverable { round });
    }
    Ok(Response::new()
        .add_attribute("action", "provide")
        .add_attribute("raffle_id", raffle_id.to_string())
        .add_attribute("round", round.to_string())
        .add_attribute(
            "randomness",
            Binary::from(beacon.randomness.as_slice()).to_base64(),
        )
        .add_attribute("owner", owner))
}

fn execute_request_randomness(
    deps: DepsMut,
    info: MessageInfo,
    job_id: String,
    after: Timestamp,
) -> Result<Response, ContractError> {
    validate_job_id(&job_id)?;
    let config = CONFIG.load(deps.storage)?;
    validate_payment(&config.prices, &info.funds)?;
    let round = enqueue_job(deps.storage, &config, &info.sender, &job_id, after)?;
    let msgs = deliver_queued(
        deps.storage,
        &config,
        None,
        Some((&info.sender, job_id.as_str())),
    )?;
    Ok(response_with_callbacks("request_randomness", msgs)
        .add_attribute("job_id", job_id)
        .add_attribute("requester", info.sender)
        .add_attribute("after", after.to_string())
        .add_attribute("round", round.to_string()))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> Result<Binary, ContractError> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&query_config(deps)?),
        QueryMsg::Beacon { round } => to_json_binary(&query_beacon(deps, round)?),
        QueryMsg::Attestors {} => to_json_binary(&query_attestors(deps)?),
        QueryMsg::Attestations { round } => to_json_binary(&query_attestations(deps, round)?),
        QueryMsg::Job { requester, job_id } => to_json_binary(&query_job(deps, requester, job_id)?),
    }
    .map_err(ContractError::from)
}

fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let config = CONFIG.load(deps.storage)?;
    Ok(ConfigResponse {
        admin: config.admin.into_string(),
        chain_hash: config.chain_hash,
        pubkey: config.pubkey,
        genesis_time: config.genesis_time,
        period_seconds: config.period_seconds,
        scheme: config.scheme,
        required_attestations: config.required_attestations,
        prices: config.prices,
        callback_gas_limit: config.callback_gas_limit,
        source_ids: SourceIds {
            drand: BeaconSource::Drand.source_id(),
            attestor: BeaconSource::Attestor.source_id(),
            reserved_ibc: RESERVED_IBC_SOURCE_ID,
        },
    })
}

fn query_beacon(deps: Deps, round: u64) -> Result<BeaconResponse, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let beacon = BEACONS
        .may_load(deps.storage, (BeaconSource::Drand.source_id(), round))?
        .ok_or(ContractError::RoundUnknown { round })?;
    let attestation_count = jobs::count_registered_attestations(deps.storage, round)?;
    let deliverable = jobs::round_is_deliverable(deps.storage, &config, round)?;
    Ok(BeaconResponse {
        round: beacon.round,
        randomness: beacon.randomness,
        signature: beacon.signature,
        published: beacon.published,
        verified_at: beacon.verified_at,
        source: beacon.source,
        attestation_count,
        deliverable,
    })
}

fn query_attestors(deps: Deps) -> StdResult<AttestorsResponse> {
    let attestors = ATTESTORS
        .range(deps.storage, None, None, Order::Ascending)
        .map(|item| {
            let (pubkey, stored) = item?;
            Ok(AttestorInfo {
                pubkey: HexBinary::from(pubkey),
                key_type: stored.key_type,
            })
        })
        .collect::<StdResult<Vec<_>>>()?;
    Ok(AttestorsResponse { attestors })
}

fn query_attestations(deps: Deps, round: u64) -> StdResult<AttestationsResponse> {
    let attestations = ATTESTATIONS
        .prefix(round)
        .range(deps.storage, None, None, Order::Ascending)
        .map(|item| {
            let (pubkey, stored) = item?;
            Ok(AttestationInfo {
                pubkey: HexBinary::from(pubkey),
                signature: stored.signature,
                source: stored.source,
            })
        })
        .collect::<StdResult<Vec<_>>>()?;
    Ok(AttestationsResponse {
        round,
        attestations,
    })
}

fn query_job(deps: Deps, requester: String, job_id: String) -> Result<JobResponse, ContractError> {
    let requester = deps.api.addr_validate(&requester)?;
    let job = JOBS
        .may_load(deps.storage, (&requester, job_id.as_str()))?
        .map(|stored| JobInfo {
            requester: requester.into_string(),
            job_id,
            after: stored.after,
            round: stored.round,
        });
    Ok(JobResponse { job })
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn migrate(
    deps: DepsMut,
    _env: Env,
    _msg: MigrateMsg,
    _info: cosmwasm_std::MigrateInfo,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    Ok(Response::new().add_attribute("action", "migrate"))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn reply(_deps: DepsMut, _env: Env, msg: Reply) -> Result<Response, ContractError> {
    if msg.id != jobs::REPLY_CALLBACK {
        return Err(ContractError::UnknownReplyId { id: msg.id });
    }
    match msg.result {
        SubMsgResult::Ok(_) => Ok(Response::new().add_attribute("callback", "success")),
        SubMsgResult::Err(err) => Ok(Response::new()
            .add_attribute("callback", "error")
            .add_attribute("callback_error", err)),
    }
}
