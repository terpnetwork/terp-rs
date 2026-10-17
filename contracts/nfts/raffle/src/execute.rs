use std::collections::BTreeMap;

use cosmwasm_std::{
    ensure, wasm_execute, Addr, BankMsg, Binary, Coin, Coins, Decimal, DepsMut, Env, Event,
    MessageInfo, Response, StdError, StdResult, SubMsg, Uint256,
};
use nftfi_randomness::{select_tickets, RandomnessSource, VerifierExecuteMsg};
use nftfi_utils::{
    all_unique, assert_can_pull, is_valid_comment, is_valid_name, take_one_of, transfer_nft_msg,
    AssetInfo,
};

use crate::error::ContractError;
use crate::msg::{ExecuteMsg, InstantiateMsg, RaffleOptionsMsg, SudoMsg};
use crate::state::{
    load_raffle, raffle_state, seed_bytes, ticket_pot, Config, DrawnRandomness, RaffleInfo,
    RaffleOptions, RaffleState, CONFIG, RAFFLES, TICKETS, USER_TICKETS,
};

pub const DRAW_REPLY_ID: u64 = 1;

pub fn instantiate(
    deps: DepsMut,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    if !is_valid_name(&msg.name) {
        return Err(ContractError::InvalidName);
    }
    if msg.raffle_fee >= Decimal::one() {
        return Err(ContractError::InvalidFeeRate);
    }
    let owner = match msg.owner {
        Some(owner) => deps.api.addr_validate(&owner)?,
        None => info.sender,
    };
    let cfg = Config {
        name: msg.name,
        owner,
        fee_addr: deps.api.addr_validate(&msg.fee_addr)?,
        last_raffle_id: None,
        minimum_raffle_duration: msg.minimum_raffle_duration.unwrap_or(1),
        max_tickets_per_raffle: msg.max_tickets_per_raffle,
        raffle_fee: msg.raffle_fee,
        locks: nftfi_utils::Locks::default(),
        creation_coins: msg.creation_coins.unwrap_or_default(),
        randomness: msg.randomness.validate(deps.api)?,
    };
    CONFIG.save(deps.storage, &cfg)?;
    Ok(Response::new().add_attribute("action", "instantiate"))
}

pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::CreateRaffle {
            owner,
            assets,
            raffle_options,
            raffle_ticket_price,
        } => create_raffle(deps, env, info, owner, assets, raffle_options, raffle_ticket_price),
        ExecuteMsg::CancelRaffle { raffle_id } => cancel_raffle(deps, env, info, raffle_id),
        ExecuteMsg::BuyTicket {
            raffle_id,
            ticket_count,
            on_behalf_of,
        } => buy_ticket(deps, env, info, raffle_id, ticket_count, on_behalf_of),
        ExecuteMsg::Draw { raffle_id, seed } => draw(deps, env, info, raffle_id, seed),
        ExecuteMsg::RequestDraw { raffle_id } => request_draw(deps, env, info, raffle_id),
        ExecuteMsg::ClaimRaffle { raffle_id } => claim(deps, env, raffle_id),
        ExecuteMsg::UpdateConfig {
            owner,
            fee_addr,
            minimum_raffle_duration,
            max_tickets_per_raffle,
            raffle_fee,
            creation_coins,
            randomness,
        } => update_config(
            deps,
            info,
            owner,
            fee_addr,
            minimum_raffle_duration,
            max_tickets_per_raffle,
            raffle_fee,
            creation_coins,
            randomness,
        ),
        ExecuteMsg::ToggleLock { lock } => toggle_lock(deps, info, lock),
    }
}

pub fn sudo(deps: DepsMut, msg: SudoMsg) -> Result<Response, ContractError> {
    let SudoMsg::ToggleLock { lock } = msg;
    let mut cfg = CONFIG.load(deps.storage)?;
    cfg.locks.sudo_lock = lock;
    CONFIG.save(deps.storage, &cfg)?;
    Ok(Response::new()
        .add_attribute("action", "sudo_lock")
        .add_attribute("lock", lock.to_string()))
}

fn create_raffle(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    owner: Option<String>,
    assets: Vec<AssetInfo>,
    options: RaffleOptionsMsg,
    ticket_price: AssetInfo,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    if cfg.locks.is_locked() {
        return Err(ContractError::Locked);
    }
    let ticket_price = match ticket_price.canonicalize(deps.api)? {
        AssetInfo::Coin(coin) => coin,
        AssetInfo::Cw721(_) => return Err(ContractError::InvalidTicketCost),
    };
    if ticket_price.amount < Uint256::one() {
        return Err(ContractError::InvalidTicketCost);
    }
    if let Some(comment) = &options.comment {
        if !is_valid_comment(comment) {
            return Err(ContractError::CommentTooLong);
        }
    }
    if options.duration < cfg.minimum_raffle_duration {
        return Err(ContractError::DurationTooShort);
    }
    if let (Some(asked), Some(cap)) = (options.max_ticket_number, cfg.max_tickets_per_raffle) {
        if asked > cap {
            return Err(ContractError::TicketCap);
        }
    }
    if assets.is_empty() {
        return Err(ContractError::NoAssets);
    }
    let assets = assets
        .into_iter()
        .map(|asset| asset.canonicalize(deps.api))
        .collect::<StdResult<Vec<_>>>()?;
    if !all_unique(&assets) {
        return Err(ContractError::DuplicateAssets);
    }

    let mut sent: Coins = info.funds.clone().try_into().map_err(|e| {
        ContractError::Std(StdError::msg(format!("funds: {e}")))
    })?;
    let mut messages = Vec::new();
    if !cfg.creation_coins.is_empty() {
        let fee = take_one_of(&mut sent, &cfg.creation_coins)
            .map_err(|_| ContractError::InvalidFee)?;
        messages.push(bank(cfg.fee_addr.to_string(), fee));
    }
    for asset in &assets {
        match asset {
            AssetInfo::Coin(coin) => {
                sent.sub(coin.clone())?;
            }
            AssetInfo::Cw721(token) => {
                assert_can_pull(
                    deps.as_ref(),
                    &env,
                    info.sender.as_str(),
                    token,
                    env.contract.address.as_str(),
                )?;
                messages.push(transfer_nft_msg(token, env.contract.address.to_string())?);
            }
        }
    }
    if !sent.is_empty() {
        return Err(ContractError::ExtraFunds);
    }

    let owner = match owner {
        Some(owner) => deps.api.addr_validate(&owner)?,
        None => info.sender.clone(),
    };
    let raffle_id = next_id(deps.storage)?;
    let info_row = RaffleInfo {
        owner: owner.clone(),
        assets,
        ticket_price,
        ticket_count: 0,
        options: RaffleOptions {
            start: options.start.unwrap_or(env.block.time),
            duration: options.duration,
            comment: options.comment,
            max_ticket_number: options.max_ticket_number.or(cfg.max_tickets_per_raffle),
            max_ticket_per_address: options.max_ticket_per_address,
            min_ticket_number: options.min_ticket_number,
            one_winner_per_asset: options.one_winner_per_asset,
        },
        randomness: None,
        winners: vec![],
        is_cancelled: false,
    };
    RAFFLES.save(deps.storage, raffle_id, &info_row)?;
    Ok(Response::new()
        .add_messages(messages)
        .add_attribute("action", "create_raffle")
        .add_attribute("raffle_id", raffle_id.to_string())
        .add_attribute("owner", owner))
}

fn next_id(storage: &mut dyn cosmwasm_std::Storage) -> Result<u64, ContractError> {
    let mut cfg = CONFIG.load(storage)?;
    let id = cfg.last_raffle_id.map(|id| id + 1).unwrap_or(0);
    cfg.last_raffle_id = Some(id);
    CONFIG.save(storage, &cfg)?;
    Ok(id)
}

fn cancel_raffle(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    raffle_id: u64,
) -> Result<Response, ContractError> {
    let mut raffle = load_raffle(deps.storage, raffle_id)?;
    if info.sender != raffle.owner {
        return Err(ContractError::Unauthorized);
    }
    let state = raffle_state(&env, &raffle);
    if !matches!(
        state,
        RaffleState::Created | RaffleState::Started | RaffleState::Closed
    ) {
        return Err(ContractError::WrongState { state });
    }
    if raffle.ticket_count != 0 {
        return Err(ContractError::TicketsSold);
    }
    raffle.is_cancelled = true;
    let messages = return_assets(&env, &raffle, &raffle.owner)?;
    RAFFLES.save(deps.storage, raffle_id, &raffle)?;
    Ok(Response::new()
        .add_messages(messages)
        .add_attribute("action", "cancel_raffle")
        .add_attribute("raffle_id", raffle_id.to_string()))
}

fn buy_ticket(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    raffle_id: u64,
    ticket_count: u32,
    on_behalf_of: Option<String>,
) -> Result<Response, ContractError> {
    if ticket_count == 0 {
        return Err(ContractError::NoTickets);
    }
    let mut raffle = load_raffle(deps.storage, raffle_id)?;
    let state = raffle_state(&env, &raffle);
    if state != RaffleState::Started {
        return Err(ContractError::WrongState { state });
    }
    let buyer = match on_behalf_of {
        Some(buyer) => deps.api.addr_validate(&buyer)?,
        None => info.sender.clone(),
    };
    let cost = raffle
        .ticket_price
        .amount
        .checked_mul(Uint256::from(ticket_count as u128))
        .map_err(|e| ContractError::Std(StdError::msg(e.to_string())))?;
    let paid = info
        .funds
        .iter()
        .find(|c| c.denom == raffle.ticket_price.denom)
        .map(|c| c.amount)
        .unwrap_or_default();
    if info.funds.len() != 1 || paid != cost {
        return Err(ContractError::Payment);
    }
    if let Some(max) = raffle.options.max_ticket_per_address {
        let held = USER_TICKETS
            .may_load(deps.storage, (buyer.clone(), raffle_id))?
            .unwrap_or(0);
        if held.saturating_add(ticket_count) > max {
            return Err(ContractError::UserLimit);
        }
    }
    if let Some(max) = raffle.options.max_ticket_number {
        if raffle.ticket_count.saturating_add(ticket_count) > max {
            return Err(ContractError::RaffleLimit);
        }
    }
    for n in 0..ticket_count {
        TICKETS.save(
            deps.storage,
            (raffle_id, raffle.ticket_count + n),
            &buyer,
        )?;
    }
    USER_TICKETS.update(
        deps.storage,
        (buyer.clone(), raffle_id),
        |held| -> StdResult<_> { Ok(held.unwrap_or(0) + ticket_count) },
    )?;
    raffle.ticket_count += ticket_count;
    if let Some(max) = raffle.options.max_ticket_number {
        if raffle.ticket_count >= max && env.block.time >= raffle.options.start {
            let elapsed = env.block.time.seconds() - raffle.options.start.seconds();
            raffle.options.duration = elapsed;
        }
    }
    RAFFLES.save(deps.storage, raffle_id, &raffle)?;
    Ok(Response::new()
        .add_attribute("action", "buy_ticket")
        .add_attribute("raffle_id", raffle_id.to_string())
        .add_attribute("buyer", buyer)
        .add_attribute("ticket_count", ticket_count.to_string()))
}

fn draw(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    raffle_id: u64,
    seed: Binary,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    let RandomnessSource::Seed { provider } = &cfg.randomness else {
        return Err(ContractError::WrongRandomnessSource);
    };
    if let Some(provider) = provider {
        if info.sender != provider {
            return Err(ContractError::Unauthorized);
        }
    }
    if seed.len() != 32 {
        return Err(ContractError::InvalidSeed);
    }
    save_draw(
        deps,
        &env,
        raffle_id,
        DrawnRandomness {
            seed,
            round: 0,
            provider: info.sender,
        },
    )
}

fn request_draw(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    raffle_id: u64,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    let RandomnessSource::Callback { verifier } = &cfg.randomness else {
        return Err(ContractError::WrongRandomnessSource);
    };
    let raffle = load_raffle(deps.storage, raffle_id)?;
    ensure_closed(&env, &raffle)?;
    let msg = wasm_execute(
        verifier.to_string(),
        &VerifierExecuteMsg::Provide {
            raffle_id,
            owner: info.sender.to_string(),
        },
        vec![],
    )?;
    Ok(Response::new()
        .add_submessage(SubMsg::reply_on_success(msg, DRAW_REPLY_ID))
        .add_attribute("action", "request_draw")
        .add_attribute("raffle_id", raffle_id.to_string()))
}

pub fn reply_draw(deps: DepsMut, env: Env, events: &[Event]) -> Result<Response, ContractError> {
    let raffle_id = attr(events, "raffle_id")
        .ok_or(ContractError::BadDrawReply)?
        .parse::<u64>()
        .map_err(|_| ContractError::BadDrawReply)?;
    let round = attr(events, "round")
        .ok_or(ContractError::BadDrawReply)?
        .parse::<u64>()
        .map_err(|_| ContractError::BadDrawReply)?;
    let randomness = Binary::from_base64(attr(events, "randomness").ok_or(ContractError::BadDrawReply)?)?;
    if randomness.len() != 32 {
        return Err(ContractError::InvalidSeed);
    }
    let provider = deps.api.addr_validate(attr(events, "owner").ok_or(ContractError::BadDrawReply)?)?;
    save_draw(
        deps,
        &env,
        raffle_id,
        DrawnRandomness {
            seed: randomness,
            round,
            provider,
        },
    )
}

fn save_draw(
    deps: DepsMut,
    env: &Env,
    raffle_id: u64,
    drawn: DrawnRandomness,
) -> Result<Response, ContractError> {
    let mut raffle = load_raffle(deps.storage, raffle_id)?;
    ensure_closed(env, &raffle)?;
    raffle.randomness = Some(drawn);
    RAFFLES.save(deps.storage, raffle_id, &raffle)?;
    Ok(Response::new()
        .add_attribute("action", "draw")
        .add_attribute("raffle_id", raffle_id.to_string()))
}

fn ensure_closed(env: &Env, raffle: &RaffleInfo) -> Result<(), ContractError> {
    let state = raffle_state(env, raffle);
    if state != RaffleState::Closed {
        return Err(ContractError::WrongState { state });
    }
    if raffle.randomness.is_some() {
        return Err(ContractError::RandomnessSet);
    }
    Ok(())
}

fn claim(deps: DepsMut, env: Env, raffle_id: u64) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    let mut raffle = load_raffle(deps.storage, raffle_id)?;
    let state = raffle_state(&env, &raffle);
    if state != RaffleState::Finished {
        return Err(ContractError::WrongState { state });
    }
    let minimum = raffle.options.min_ticket_number.unwrap_or(0);
    let messages = if raffle.ticket_count == 0 || raffle.ticket_count < minimum {
        raffle.winners = vec![raffle.owner.clone()];
        let mut msgs = return_assets(&env, &raffle, &raffle.owner)?;
        msgs.extend(refund_tickets(deps.storage, raffle_id, &raffle)?);
        msgs
    } else {
        let drawn = raffle
            .randomness
            .as_ref()
            .ok_or_else(|| ContractError::Std(StdError::msg("missing draw")))?;
        let seed = seed_bytes(drawn)?;
        let picks = if raffle.options.one_winner_per_asset {
            raffle.assets.len() as u32
        } else {
            1
        };
        let indexes = select_tickets(&seed, raffle_id, raffle.ticket_count, picks);
        let mut winners = Vec::with_capacity(indexes.len());
        for index in indexes {
            winners.push(TICKETS.load(deps.storage, (raffle_id, index))?);
        }
        raffle.winners = winners;
        let mut msgs = pay_pot(&cfg, &raffle)?;
        msgs.extend(pay_prizes(&env, &raffle)?);
        msgs
    };
    RAFFLES.save(deps.storage, raffle_id, &raffle)?;
    let winner_list = raffle
        .winners
        .iter()
        .map(|a| a.to_string())
        .collect::<Vec<_>>()
        .join(",");
    Ok(Response::new()
        .add_messages(messages)
        .add_attribute("action", "claim")
        .add_attribute("raffle_id", raffle_id.to_string())
        .add_attribute("winners", winner_list))
}

fn pay_pot(cfg: &Config, raffle: &RaffleInfo) -> Result<Vec<cosmwasm_std::CosmosMsg>, ContractError> {
    let pot = ticket_pot(raffle)?;
    let fee = pot
        .checked_mul_floor(cfg.raffle_fee)
        .map_err(|e| ContractError::Std(StdError::msg(e.to_string())))?;
    let owner_amt = pot.checked_sub(fee).map_err(math_err)?;
    let mut msgs = Vec::new();
    if !owner_amt.is_zero() {
        msgs.push(bank(
            raffle.owner.to_string(),
            Coin::new(owner_amt, raffle.ticket_price.denom.clone()),
        ));
    }
    if !fee.is_zero() {
        msgs.push(bank(
            cfg.fee_addr.to_string(),
            Coin::new(fee, raffle.ticket_price.denom.clone()),
        ));
    }
    Ok(msgs)
}

fn pay_prizes(env: &Env, raffle: &RaffleInfo) -> Result<Vec<cosmwasm_std::CosmosMsg>, ContractError> {
    let winners = &raffle.winners;
    ensure!(!winners.is_empty(), ContractError::NoTickets);
    let mut msgs = Vec::new();
    for (i, asset) in raffle.assets.iter().enumerate() {
        let winner = if winners.len() == 1 {
            &winners[0]
        } else {
            &winners[i.min(winners.len() - 1)]
        };
        msgs.push(send_asset(env, asset, winner)?);
    }
    Ok(msgs)
}

fn return_assets(
    env: &Env,
    raffle: &RaffleInfo,
    to: &Addr,
) -> Result<Vec<cosmwasm_std::CosmosMsg>, ContractError> {
    raffle
        .assets
        .iter()
        .map(|asset| send_asset(env, asset, to))
        .collect()
}

fn send_asset(
    _env: &Env,
    asset: &AssetInfo,
    to: &Addr,
) -> Result<cosmwasm_std::CosmosMsg, ContractError> {
    match asset {
        AssetInfo::Coin(coin) => Ok(bank(to.to_string(), coin.clone())),
        AssetInfo::Cw721(token) => Ok(transfer_nft_msg(token, to.to_string())?),
    }
}

fn refund_tickets(
    storage: &dyn cosmwasm_std::Storage,
    raffle_id: u64,
    raffle: &RaffleInfo,
) -> Result<Vec<cosmwasm_std::CosmosMsg>, ContractError> {
    let mut totals: BTreeMap<Addr, Uint256> = BTreeMap::new();
    for index in 0..raffle.ticket_count {
        let buyer = TICKETS.load(storage, (raffle_id, index))?;
        let entry = totals.entry(buyer).or_insert_with(Uint256::zero);
        *entry = entry
            .checked_add(raffle.ticket_price.amount)
            .map_err(math_err)?;
    }
    Ok(totals
        .into_iter()
        .filter(|(_, amount)| !amount.is_zero())
        .map(|(buyer, amount)| bank(buyer.to_string(), Coin::new(amount, raffle.ticket_price.denom.clone())))
        .collect())
}

fn update_config(
    deps: DepsMut,
    info: MessageInfo,
    owner: Option<String>,
    fee_addr: Option<String>,
    minimum_raffle_duration: Option<u64>,
    max_tickets_per_raffle: Option<u32>,
    raffle_fee: Option<Decimal>,
    creation_coins: Option<Vec<Coin>>,
    randomness: Option<nftfi_randomness::RandomnessSourceMsg>,
) -> Result<Response, ContractError> {
    let mut cfg = CONFIG.load(deps.storage)?;
    if info.sender != cfg.owner {
        return Err(ContractError::Unauthorized);
    }
    if let Some(owner) = owner {
        cfg.owner = deps.api.addr_validate(&owner)?;
    }
    if let Some(fee_addr) = fee_addr {
        cfg.fee_addr = deps.api.addr_validate(&fee_addr)?;
    }
    if let Some(min) = minimum_raffle_duration {
        cfg.minimum_raffle_duration = min.max(1);
    }
    if let Some(max) = max_tickets_per_raffle {
        cfg.max_tickets_per_raffle = Some(max);
    }
    if let Some(fee) = raffle_fee {
        if fee >= Decimal::one() {
            return Err(ContractError::InvalidFeeRate);
        }
        cfg.raffle_fee = fee;
    }
    if let Some(coins) = creation_coins {
        cfg.creation_coins = coins;
    }
    if let Some(source) = randomness {
        cfg.randomness = source.validate(deps.api)?;
    }
    CONFIG.save(deps.storage, &cfg)?;
    Ok(Response::new().add_attribute("action", "update_config"))
}

fn toggle_lock(deps: DepsMut, info: MessageInfo, lock: bool) -> Result<Response, ContractError> {
    let mut cfg = CONFIG.load(deps.storage)?;
    if info.sender != cfg.owner {
        return Err(ContractError::Unauthorized);
    }
    cfg.locks.lock = lock;
    CONFIG.save(deps.storage, &cfg)?;
    Ok(Response::new()
        .add_attribute("action", "toggle_lock")
        .add_attribute("lock", lock.to_string()))
}

fn math_err(err: impl ToString) -> ContractError {
    ContractError::Std(StdError::msg(err.to_string()))
}

fn bank(to: String, coin: Coin) -> cosmwasm_std::CosmosMsg {
    BankMsg::Send {
        to_address: to,
        amount: vec![coin],
    }
    .into()
}

fn attr<'a>(events: &'a [Event], key: &str) -> Option<&'a str> {
    events.iter().find_map(|event| {
        event
            .attributes
            .iter()
            .find(|attr| attr.key == key)
            .map(|attr| attr.value.as_str())
    })
}
