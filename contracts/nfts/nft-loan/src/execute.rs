use cosmwasm_std::{
    Addr, BankMsg, Coin, Coins, Decimal, DepsMut, Env, MessageInfo, Response, StdError,
};
use nftfi_utils::{
    all_unique, assert_can_pull, is_valid_comment, is_valid_name, take_one_of, transfer_nft_msg,
    AssetInfo,
};

use crate::error::ContractError;
use crate::msg::{ExecuteMsg, InstantiateMsg, SudoMsg};
use crate::state::{
    active_offer, is_defaulted, load_collateral, load_offer, CollateralInfo, Config, LoanState,
    LoanTerms, OfferInfo, OfferState, BORROWER, COLLATERAL, CONFIG, OFFERS,
};

pub fn instantiate(
    deps: DepsMut,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    if !is_valid_name(&msg.name) {
        return Err(ContractError::InvalidName);
    }
    if msg.fee_rate >= Decimal::one() {
        return Err(ContractError::InvalidFeeRate);
    }
    let owner = match msg.owner {
        Some(owner) => deps.api.addr_validate(&owner)?,
        None => info.sender,
    };
    CONFIG.save(
        deps.storage,
        &Config {
            name: msg.name,
            owner,
            treasury_addr: deps.api.addr_validate(&msg.treasury_addr)?,
            fee_rate: msg.fee_rate,
            listing_fee_coins: msg.listing_fee_coins.unwrap_or_default(),
            global_offer_index: 0,
            locks: nftfi_utils::Locks::default(),
        },
    )?;
    Ok(Response::new().add_attribute("action", "instantiate"))
}

pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::ListCollaterals {
            tokens,
            terms,
            comment,
        } => list(deps, env, info, tokens, terms, comment),
        ExecuteMsg::WithdrawCollaterals { loan_id } => withdraw_listing(deps, info, loan_id),
        ExecuteMsg::MakeOffer {
            borrower,
            loan_id,
            terms,
            comment,
            on_behalf_of,
        } => make_offer(deps, env, info, borrower, loan_id, terms, comment, on_behalf_of),
        ExecuteMsg::CancelOffer { global_offer_id } => cancel_offer(deps, info, global_offer_id),
        ExecuteMsg::RefuseOffer { global_offer_id } => refuse_offer(deps, info, global_offer_id),
        ExecuteMsg::WithdrawRefusedOffer { global_offer_id } => {
            withdraw_refused(deps, info, global_offer_id)
        }
        ExecuteMsg::AcceptOffer { global_offer_id } => accept_offer(deps, env, info, global_offer_id),
        ExecuteMsg::AcceptLoan {
            borrower,
            loan_id,
            comment,
        } => accept_loan(deps, env, info, borrower, loan_id, comment),
        ExecuteMsg::Repay { loan_id } => repay(deps, env, info, loan_id),
        ExecuteMsg::WithdrawDefaulted { borrower, loan_id } => {
            withdraw_defaulted(deps, env, info, borrower, loan_id)
        }
        ExecuteMsg::ToggleLock { lock } => {
            let mut cfg = admin(&deps, &info)?;
            cfg.locks.lock = lock;
            CONFIG.save(deps.storage, &cfg)?;
            Ok(Response::new()
                .add_attribute("action", "toggle_lock")
                .add_attribute("lock", lock.to_string()))
        }
        ExecuteMsg::SetOwner { owner } => {
            let mut cfg = admin(&deps, &info)?;
            cfg.owner = deps.api.addr_validate(&owner)?;
            CONFIG.save(deps.storage, &cfg)?;
            Ok(Response::new().add_attribute("action", "set_owner"))
        }
        ExecuteMsg::SetTreasury { treasury_addr } => {
            let mut cfg = admin(&deps, &info)?;
            cfg.treasury_addr = deps.api.addr_validate(&treasury_addr)?;
            CONFIG.save(deps.storage, &cfg)?;
            Ok(Response::new().add_attribute("action", "set_treasury"))
        }
        ExecuteMsg::SetFeeRate { fee_rate } => {
            if fee_rate >= Decimal::one() {
                return Err(ContractError::InvalidFeeRate);
            }
            let mut cfg = admin(&deps, &info)?;
            cfg.fee_rate = fee_rate;
            CONFIG.save(deps.storage, &cfg)?;
            Ok(Response::new().add_attribute("action", "set_fee_rate"))
        }
        ExecuteMsg::SetListingFee { listing_fee_coins } => {
            let mut cfg = admin(&deps, &info)?;
            cfg.listing_fee_coins = listing_fee_coins;
            CONFIG.save(deps.storage, &cfg)?;
            Ok(Response::new().add_attribute("action", "set_listing_fee"))
        }
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

fn admin(deps: &DepsMut, info: &MessageInfo) -> Result<Config, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    if info.sender != cfg.owner {
        return Err(ContractError::Unauthorized);
    }
    Ok(cfg)
}

fn list(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    tokens: Vec<AssetInfo>,
    terms: Option<LoanTerms>,
    comment: Option<String>,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    if cfg.locks.is_locked() {
        return Err(ContractError::Locked);
    }
    if let Some(comment) = &comment {
        if !is_valid_comment(comment) {
            return Err(ContractError::CommentTooLong);
        }
    }
    if let Some(terms) = &terms {
        terms.check()?;
    }
    if tokens.is_empty() {
        return Err(ContractError::NoAssets);
    }
    let tokens = tokens
        .into_iter()
        .map(|token| token.canonicalize(deps.api))
        .collect::<Result<Vec<_>, _>>()?;
    if !all_unique(&tokens) {
        return Err(ContractError::DuplicateAssets);
    }
    for token in &tokens {
        let AssetInfo::Cw721(nft) = token else {
            return Err(ContractError::NotNft);
        };
        assert_can_pull(
            deps.as_ref(),
            &env,
            info.sender.as_str(),
            nft,
            env.contract.address.as_str(),
        )?;
    }

    let mut sent: Coins = info.funds.clone().try_into().map_err(|e| {
        ContractError::Std(StdError::msg(format!("funds: {e}")))
    })?;
    let mut messages = Vec::new();
    if !cfg.listing_fee_coins.is_empty() {
        let fee = take_one_of(&mut sent, &cfg.listing_fee_coins)
            .map_err(|_| ContractError::InvalidFee)?;
        messages.push(bank(cfg.treasury_addr.to_string(), fee));
    }
    if !sent.is_empty() {
        return Err(ContractError::ExtraFunds);
    }

    let mut borrower = BORROWER
        .may_load(deps.storage, info.sender.clone())?
        .unwrap_or_default();
    let loan_id = if borrower.listed {
        borrower.last_collateral_id + 1
    } else {
        0
    };
    borrower.last_collateral_id = loan_id;
    borrower.listed = true;
    BORROWER.save(deps.storage, info.sender.clone(), &borrower)?;
    COLLATERAL.save(
        deps.storage,
        (info.sender.clone(), loan_id),
        &CollateralInfo {
            terms,
            associated_assets: tokens,
            list_date: env.block.time,
            state: LoanState::Published,
            offer_amount: 0,
            active_offer: None,
            start_block: None,
            comment,
        },
    )?;
    Ok(Response::new()
        .add_messages(messages)
        .add_attribute("action", "list_collateral")
        .add_attribute("borrower", info.sender)
        .add_attribute("loan_id", loan_id.to_string()))
}

fn withdraw_listing(
    deps: DepsMut,
    info: MessageInfo,
    loan_id: u64,
) -> Result<Response, ContractError> {
    let mut collateral = load_collateral(deps.storage, &info.sender, loan_id)?;
    if collateral.state != LoanState::Published {
        return Err(ContractError::WrongLoanState {
            state: collateral.state,
        });
    }
    collateral.state = LoanState::Inactive;
    COLLATERAL.save(deps.storage, (info.sender.clone(), loan_id), &collateral)?;
    Ok(Response::new()
        .add_attribute("action", "withdraw_collateral")
        .add_attribute("borrower", info.sender)
        .add_attribute("loan_id", loan_id.to_string()))
}

fn make_offer(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    borrower: String,
    loan_id: u64,
    terms: LoanTerms,
    comment: Option<String>,
    on_behalf_of: Option<String>,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    if cfg.locks.is_locked() {
        return Err(ContractError::Locked);
    }
    terms.check()?;
    if let Some(comment) = &comment {
        if !is_valid_comment(comment) {
            return Err(ContractError::CommentTooLong);
        }
    }
    let borrower = deps.api.addr_validate(&borrower)?;
    let lender = match on_behalf_of {
        Some(lender) => deps.api.addr_validate(&lender)?,
        None => info.sender,
    };
    let collateral = load_collateral(deps.storage, &borrower, loan_id)?;
    if collateral.state != LoanState::Published {
        return Err(ContractError::WrongLoanState {
            state: collateral.state,
        });
    }
    require_exact_funds(&info.funds, &terms.principle)?;
    let global_offer_id = save_offer(
        deps.storage,
        env,
        lender.clone(),
        borrower.clone(),
        loan_id,
        terms,
        comment,
    )?;
    Ok(Response::new()
        .add_attribute("action", "make_offer")
        .add_attribute("borrower", borrower)
        .add_attribute("lender", lender)
        .add_attribute("loan_id", loan_id.to_string())
        .add_attribute("global_offer_id", global_offer_id.to_string()))
}

fn save_offer(
    storage: &mut dyn cosmwasm_std::Storage,
    env: Env,
    lender: Addr,
    borrower: Addr,
    loan_id: u64,
    terms: LoanTerms,
    comment: Option<String>,
) -> Result<u64, ContractError> {
    let mut cfg = CONFIG.load(storage)?;
    let mut collateral = load_collateral(storage, &borrower, loan_id)?;
    collateral.offer_amount += 1;
    let offer_id = collateral.offer_amount;
    COLLATERAL.save(storage, (borrower.clone(), loan_id), &collateral)?;
    cfg.global_offer_index += 1;
    let global = cfg.global_offer_index;
    CONFIG.save(storage, &cfg)?;
    OFFERS.save(
        storage,
        global,
        &OfferInfo {
            lender,
            borrower,
            loan_id,
            offer_id,
            terms: terms.clone(),
            state: OfferState::Published,
            list_date: env.block.time,
            deposited_funds: Some(terms.principle),
            comment,
        },
    )?;
    Ok(global)
}

fn accept_loan(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    borrower: String,
    loan_id: u64,
    comment: Option<String>,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    if cfg.locks.is_locked() {
        return Err(ContractError::Locked);
    }
    if let Some(comment) = &comment {
        if !is_valid_comment(comment) {
            return Err(ContractError::CommentTooLong);
        }
    }
    let borrower = deps.api.addr_validate(&borrower)?;
    let collateral = load_collateral(deps.storage, &borrower, loan_id)?;
    let terms = collateral.terms.clone().ok_or(ContractError::NoTerms)?;
    terms.check()?;
    require_exact_funds(&info.funds, &terms.principle)?;
    let global = save_offer(
        deps.storage,
        env.clone(),
        info.sender,
        borrower,
        loan_id,
        terms,
        comment,
    )?;
    start_loan(deps, env, global).map(|res| res.add_attribute("action_type", "accept_loan"))
}

fn accept_offer(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    global_offer_id: String,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    if cfg.locks.is_locked() {
        return Err(ContractError::Locked);
    }
    let id = parse_id(&global_offer_id)?;
    let offer = load_offer(deps.storage, id)?;
    if info.sender != offer.borrower {
        return Err(ContractError::Unauthorized);
    }
    start_loan(deps, env, id).map(|res| res.add_attribute("action_type", "accept_offer"))
}

fn start_loan(deps: DepsMut, env: Env, global: u64) -> Result<Response, ContractError> {
    let mut offer = load_offer(deps.storage, global)?;
    if offer.state != OfferState::Published {
        return Err(ContractError::WrongOfferState { state: offer.state });
    }
    let mut collateral = load_collateral(deps.storage, &offer.borrower, offer.loan_id)?;
    if collateral.state != LoanState::Published {
        return Err(ContractError::WrongLoanState {
            state: collateral.state,
        });
    }
    let mut messages = Vec::new();
    for asset in &collateral.associated_assets {
        let AssetInfo::Cw721(nft) = asset else {
            return Err(ContractError::NotNft);
        };
        assert_can_pull(
            deps.as_ref(),
            &env,
            offer.borrower.as_str(),
            nft,
            env.contract.address.as_str(),
        )?;
        messages.push(transfer_nft_msg(nft, env.contract.address.to_string())?);
    }
    let funds = offer
        .deposited_funds
        .clone()
        .ok_or(ContractError::FundsMismatch)?;
    messages.insert(0, bank(offer.borrower.to_string(), funds));

    collateral.state = LoanState::Started;
    collateral.start_block = Some(env.block.height);
    collateral.active_offer = Some(global);
    offer.state = OfferState::Accepted;
    COLLATERAL.save(
        deps.storage,
        (offer.borrower.clone(), offer.loan_id),
        &collateral,
    )?;
    OFFERS.save(deps.storage, global, &offer)?;
    Ok(Response::new()
        .add_messages(messages)
        .add_attribute("action", "start_loan")
        .add_attribute("borrower", offer.borrower)
        .add_attribute("lender", offer.lender)
        .add_attribute("loan_id", offer.loan_id.to_string())
        .add_attribute("global_offer_id", global.to_string()))
}

fn repay(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    loan_id: u64,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    let mut collateral = load_collateral(deps.storage, &info.sender, loan_id)?;
    let offer = active_offer(deps.storage, &collateral)?;
    if collateral.state != LoanState::Started || is_defaulted(&env, &collateral, &offer) {
        return Err(ContractError::WrongLoanState {
            state: if is_defaulted(&env, &collateral, &offer) {
                LoanState::Defaulted
            } else {
                collateral.state
            },
        });
    }
    let due = offer
        .terms
        .principle
        .amount
        .checked_add(offer.terms.interest)
        .map_err(|e| ContractError::Std(StdError::msg(e.to_string())))?;
    let paid = single_coin(&info.funds)?;
    if paid.denom != offer.terms.principle.denom || paid.amount < due {
        return Err(ContractError::FundsMismatch);
    }
    let fee = offer
        .terms
        .interest
        .checked_mul_floor(cfg.fee_rate)
        .map_err(|e| ContractError::Std(StdError::msg(e.to_string())))?;
    let lender_interest = offer.terms.interest.checked_sub(fee).map_err(math_err)?;
    let lender_pay = offer
        .terms
        .principle
        .amount
        .checked_add(lender_interest)
        .map_err(math_err)?;
    let treasury_pay = paid.amount.checked_sub(lender_pay).map_err(math_err)?;

    collateral.state = LoanState::Ended;
    COLLATERAL.save(deps.storage, (info.sender.clone(), loan_id), &collateral)?;

    let mut res = Response::new();
    if !lender_pay.is_zero() {
        res = res.add_message(bank(
            offer.lender.to_string(),
            Coin::new(lender_pay, paid.denom.clone()),
        ));
    }
    res = res.add_messages(move_nfts(&collateral, &info.sender)?);
    if !treasury_pay.is_zero() {
        res = res.add_message(bank(
            cfg.treasury_addr.to_string(),
            Coin::new(treasury_pay, paid.denom.clone()),
        ));
    }
    Ok(res
        .add_attribute("action", "repay")
        .add_attribute("borrower", info.sender)
        .add_attribute("lender", offer.lender)
        .add_attribute("loan_id", loan_id.to_string()))
}

fn withdraw_defaulted(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    borrower: String,
    loan_id: u64,
) -> Result<Response, ContractError> {
    let borrower = deps.api.addr_validate(&borrower)?;
    let mut collateral = load_collateral(deps.storage, &borrower, loan_id)?;
    if collateral.state == LoanState::Defaulted {
        return Err(ContractError::AlreadyDefaulted);
    }
    let offer = active_offer(deps.storage, &collateral)?;
    if info.sender != offer.lender {
        return Err(ContractError::Unauthorized);
    }
    if !is_defaulted(&env, &collateral, &offer) {
        return Err(ContractError::WrongLoanState {
            state: collateral.state,
        });
    }
    collateral.state = LoanState::Defaulted;
    COLLATERAL.save(deps.storage, (borrower.clone(), loan_id), &collateral)?;
    Ok(Response::new()
        .add_messages(move_nfts(&collateral, &offer.lender)?)
        .add_attribute("action", "default")
        .add_attribute("borrower", borrower)
        .add_attribute("lender", offer.lender)
        .add_attribute("loan_id", loan_id.to_string()))
}

fn cancel_offer(
    deps: DepsMut,
    info: MessageInfo,
    global_offer_id: String,
) -> Result<Response, ContractError> {
    let id = parse_id(&global_offer_id)?;
    let mut offer = load_offer(deps.storage, id)?;
    if info.sender != offer.lender {
        return Err(ContractError::Unauthorized);
    }
    if offer.state != OfferState::Published {
        return Err(ContractError::WrongOfferState { state: offer.state });
    }
    let collateral = load_collateral(deps.storage, &offer.borrower, offer.loan_id)?;
    // Inactive listings (borrower withdrew) still let the lender take the deposit back.
    if !matches!(collateral.state, LoanState::Published | LoanState::Inactive) {
        return Err(ContractError::WrongLoanState {
            state: collateral.state,
        });
    }
    let funds = offer
        .deposited_funds
        .clone()
        .ok_or(ContractError::FundsMismatch)?;
    offer.state = OfferState::Cancelled;
    offer.deposited_funds = None;
    OFFERS.save(deps.storage, id, &offer)?;
    Ok(Response::new()
        .add_message(bank(offer.lender.to_string(), funds))
        .add_attribute("action", "cancel_offer")
        .add_attribute("global_offer_id", id.to_string()))
}

fn refuse_offer(
    deps: DepsMut,
    info: MessageInfo,
    global_offer_id: String,
) -> Result<Response, ContractError> {
    let id = parse_id(&global_offer_id)?;
    let mut offer = load_offer(deps.storage, id)?;
    if info.sender != offer.borrower {
        return Err(ContractError::Unauthorized);
    }
    if offer.state != OfferState::Published {
        return Err(ContractError::WrongOfferState { state: offer.state });
    }
    let collateral = load_collateral(deps.storage, &offer.borrower, offer.loan_id)?;
    if collateral.state != LoanState::Published {
        return Err(ContractError::WrongLoanState {
            state: collateral.state,
        });
    }
    offer.state = OfferState::Refused;
    OFFERS.save(deps.storage, id, &offer)?;
    Ok(Response::new()
        .add_attribute("action", "refuse_offer")
        .add_attribute("global_offer_id", id.to_string()))
}

fn withdraw_refused(
    deps: DepsMut,
    info: MessageInfo,
    global_offer_id: String,
) -> Result<Response, ContractError> {
    let id = parse_id(&global_offer_id)?;
    let mut offer = load_offer(deps.storage, id)?;
    if info.sender != offer.lender {
        return Err(ContractError::Unauthorized);
    }
    if offer.state != OfferState::Refused {
        return Err(ContractError::WrongOfferState { state: offer.state });
    }
    let funds = offer
        .deposited_funds
        .clone()
        .ok_or(ContractError::FundsMismatch)?;
    offer.deposited_funds = None;
    OFFERS.save(deps.storage, id, &offer)?;
    Ok(Response::new()
        .add_message(bank(info.sender.to_string(), funds))
        .add_attribute("action", "withdraw_refused_offer")
        .add_attribute("global_offer_id", id.to_string()))
}

fn move_nfts(
    collateral: &CollateralInfo,
    to: &Addr,
) -> Result<Vec<cosmwasm_std::CosmosMsg>, ContractError> {
    collateral
        .associated_assets
        .iter()
        .map(|asset| {
            let AssetInfo::Cw721(nft) = asset else {
                return Err(ContractError::NotNft);
            };
            Ok(transfer_nft_msg(nft, to.to_string())?)
        })
        .collect()
}

fn require_exact_funds(funds: &[Coin], want: &Coin) -> Result<(), ContractError> {
    let paid = single_coin(funds)?;
    if paid.denom != want.denom || paid.amount != want.amount {
        return Err(ContractError::FundsMismatch);
    }
    Ok(())
}

fn single_coin(funds: &[Coin]) -> Result<&Coin, ContractError> {
    if funds.len() != 1 {
        return Err(ContractError::MultipleCoins);
    }
    Ok(&funds[0])
}

fn parse_id(id: &str) -> Result<u64, ContractError> {
    id.parse().map_err(|_| ContractError::OfferNotFound)
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
