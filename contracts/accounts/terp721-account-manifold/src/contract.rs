use cosmwasm_std::{
    instantiate2_address, to_json_binary, Binary, Decimal, Deps, DepsMut, Env, MessageInfo,
    Response, StdError, StdResult, Uint128, WasmMsg,
};
use cw2::set_contract_version;
use cw721::msg::{CollectionExtensionMsg, RoyaltyInfoResponse};
use terp_account::manifold::{MigrateMsg, SudoParams};

use terp721_account::msg::Terp721InstantiateMsg;
use terp_account::manifold::{Config, ExecuteMsg, InstantiateMsg, QueryMsg, SudoMsg};
// use cw2::set_contract_version;

use crate::commands::*;
use crate::error::ContractError;

use crate::state::{
    ask_key, ACCOUNT_COLLECTION, ASK_HOOKS, BID_HOOKS, CONFIG, COOLDOWN_BID, MAX_FEE_BPS, PAUSED,
    SALE_HOOKS, SUDO_PARAMS,
};

// version info for migration info
pub const ACCOUNT_MANIFOLD_CONTRACT: &str = "crates.io:terp721-account-manifold";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, ACCOUNT_MANIFOLD_CONTRACT, CONTRACT_VERSION)?;

    cw_ownable::initialize_owner(
        deps.storage,
        deps.api,
        Some(msg.admin.unwrap_or(info.sender.to_string()).as_str()),
    )?;

    PAUSED.save(deps.storage, &false)?;

    SUDO_PARAMS.save(
        deps.storage,
        &SudoParams {
            min_account_length: msg.min_account_length,
            max_account_length: msg.max_account_length,
            base_price: msg.base_price,
            base_delegation: msg.base_delegation,
            trading_fee_percent: Decimal::percent(msg.trading_fee_bps) / Uint128::from(100u128),
            min_price: msg.min_price,
            ask_interval: msg.ask_interval,
            valid_bid_query_limit: msg.valid_bid_query_limit,
            cooldown_duration: msg.cooldown_timeframe,
            cooldown_fee: msg.cooldown_cancel_fee,
            hooks_admin: msg.hooks_admin.unwrap_or(info.sender.to_string()),
        },
    )?;

    CONFIG.save(
        deps.storage,
        &Config {
            public_mint_start_time: env
                .block
                .time
                .plus_seconds(msg.mint_start_delay.unwrap_or(1)),
        },
    )?;

    let metadata = CollectionExtensionMsg::<RoyaltyInfoResponse> {
        description: Some("Terp Networks account token billboards".into()),
        image: Some("ipfs://QmbAXZjPUPw8gisR8VkEm5iG4ca2qT9kXQHoTg1nadFn9z".into()),
        external_link: Some("https://terp.network".into()),
        explicit_content: None,
        start_trading_time: None,
        royalty_info: None,
        banner_url: None,
    };

    let account_collection_init_msg = terp721_account::msg::InstantiateMsg {
        verifier: msg.verifier,
        base_init_msg: Terp721InstantiateMsg {
            name: "Terp Account Tokens".to_string(),
            symbol: "ACCOUNTS".to_string(),
            minter: Some(env.contract.address.to_string()),
            collection_info_extension: Some(metadata),
            creator: Some(info.sender.to_string()),
            withdraw_address: Some(info.sender.to_string()),
        },
    };
    let salt = &env.block.height.to_be_bytes();
    let code_info = deps.querier.query_wasm_code_info(msg.collection_code_id)?;

    let addr = instantiate2_address(
        code_info.checksum.as_slice(),
        &deps.api.addr_canonicalize(env.contract.address.as_str())?,
        salt,
    )?;

    let wasm_msg = WasmMsg::Instantiate2 {
        code_id: msg.collection_code_id,
        msg: to_json_binary(&account_collection_init_msg)?,
        funds: info.funds,
        admin: Some(info.sender.to_string()),
        label: "Account Collection".to_string(),
        salt: salt.into(),
    };
    ACCOUNT_COLLECTION.save(deps.storage, &deps.api.addr_humanize(&addr)?)?;

    if msg.trading_fee_bps > MAX_FEE_BPS {
        return Err(ContractError::InvalidTradingFeeBps(msg.trading_fee_bps));
    }

    Ok(Response::new()
        .add_message(wasm_msg)
        .add_attribute("action", "instantiate")
        .add_attribute("account_minter_addr", env.contract.address.to_string()))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    let api = deps.api;
    match msg {
        ExecuteMsg::MintAndList { account } => {
            execute_mint_and_list(deps, info, env, account.trim())
        }
        ExecuteMsg::UpdateOwnership(action) => execute_update_owner(deps, info, env, action),
        ExecuteMsg::Pause { pause } => execute_pause(deps, info, pause),
        ExecuteMsg::UpdateConfig { config } => execute_update_config(deps, info, env, config),
        ExecuteMsg::RemoveAsk { token_id } => execute_remove_ask(deps, info, &token_id),
        ExecuteMsg::UpdateAsk { token_id, seller } => {
            execute_update_ask(deps, info, &token_id, api.addr_validate(&seller)?)
        }
        ExecuteMsg::SetBid { token_id } => execute_set_bid(deps, env, info, &token_id),
        ExecuteMsg::RemoveBid { token_id } => execute_remove_bid(deps, env, info, &token_id),
        ExecuteMsg::AcceptBid { token_id, bidder } => {
            execute_accept_bid(deps, env, info, &token_id, api.addr_validate(&bidder)?)
        }
        ExecuteMsg::FinalizeBid { token_id } => execute_finalize_bid(deps, env, &token_id),
        ExecuteMsg::CancelCooldown { token_id } => {
            execute_cancel_cooldown(deps, env, info, &token_id)
        }
        ExecuteMsg::RemoveBids { token_id } => execute_remove_bids(deps, env, info, &token_id),
        ExecuteMsg::CheckedRemoveBids { token_id } => {
            execute_removed_overflow_bids(deps, &token_id)
        }
        ExecuteMsg::ManageHooks(action) => manage_hooks(deps, info.sender, action),
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    let api = deps.api;
    match msg {
        QueryMsg::Ownership {} => to_json_binary(&cw_ownable::get_ownership(deps.storage)?),
        QueryMsg::Collection {} => to_json_binary(&query_collection(deps)?),
        QueryMsg::Params {} => to_json_binary(&query_params(deps)?),
        QueryMsg::Config {} => to_json_binary(&query_config(deps)?),
        QueryMsg::Ask { token_id } => to_json_binary(&query_ask(deps, token_id)?),
        QueryMsg::Asks { start_after, limit } => {
            to_json_binary(&query_asks(deps, start_after, limit)?)
        }
        QueryMsg::AsksBySeller {
            seller,
            start_after,
            limit,
        } => to_json_binary(&query_asks_by_seller(
            deps,
            api.addr_validate(&seller)?,
            start_after,
            limit,
        )?),
        QueryMsg::AskCount {} => to_json_binary(&query_ask_count(deps)?),
        QueryMsg::Bid { token_id, bidder } => {
            to_json_binary(&query_bid(deps, token_id, api.addr_validate(&bidder)?)?)
        }
        QueryMsg::Bids {
            token_id,
            start_after,
            limit,
        } => to_json_binary(&query_bids(deps, token_id, start_after, limit)?),
        QueryMsg::BidsByBidder {
            bidder,
            start_after,
            limit,
        } => to_json_binary(&query_bids_by_bidder(
            deps,
            api.addr_validate(&bidder)?,
            start_after,
            limit,
        )?),
        QueryMsg::BidsSortedByPrice { start_after, limit } => {
            to_json_binary(&query_bids_sorted_by_price(deps, start_after, limit)?)
        }
        QueryMsg::ReverseBidsSortedByPrice {
            start_before,
            limit,
        } => to_json_binary(&reverse_query_bids_sorted_by_price(
            deps,
            start_before,
            limit,
        )?),
        QueryMsg::BidsForSeller {
            seller,
            start_after,
            limit,
        } => to_json_binary(&query_bids_for_seller(
            deps,
            api.addr_validate(&seller)?,
            start_after,
            limit,
        )?),
        QueryMsg::HighestBid { token_id } => to_json_binary(&query_highest_bid(deps, token_id)?),
        QueryMsg::Params {} => to_json_binary(&query_params(deps)?),
        QueryMsg::AskHooks {} => to_json_binary(&ASK_HOOKS.query_hooks(deps)?),
        QueryMsg::BidHooks {} => to_json_binary(&BID_HOOKS.query_hooks(deps)?),
        QueryMsg::SaleHooks {} => to_json_binary(&SALE_HOOKS.query_hooks(deps)?),
        QueryMsg::Cooldown { token_id } => {
            to_json_binary(&COOLDOWN_BID.may_load(deps.storage, &ask_key(&token_id))?)
        }
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn sudo(deps: DepsMut, _env: Env, msg: SudoMsg) -> Result<Response, ContractError> {
    let api = deps.api;

    match msg {
        SudoMsg::UpdateParams {
            min_account_length,
            max_account_length,
            base_price,
            base_delegation,
            trading_fee_bps,
            min_price,
            ask_interval,
            cooldown_duration,
            cooldown_cancel_fee,
            // fair_burn_bps,
        } => sudo_update_params(
            deps,
            min_account_length,
            max_account_length,
            base_price,
            base_delegation,
            trading_fee_bps,
            min_price,
            ask_interval,
            cooldown_duration,
            cooldown_cancel_fee,
        ),
        SudoMsg::UpdateAccountCollection { collection } => {
            sudo_update_account_collection(deps, api.addr_validate(&collection)?)
        }
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn migrate(deps: DepsMut, _env: Env, _msg: MigrateMsg) -> Result<Response, ContractError> {
    let then = cw2::get_contract_version(deps.storage)?;
    if then.version >= CONTRACT_VERSION.to_owned()
        || then.contract != ACCOUNT_MANIFOLD_CONTRACT.to_owned()
    {
        return Err(ContractError::Std(StdError::msg(
            "unable to migrate terp721-account minter.",
        )));
    }
    cw2::set_contract_version(deps.storage, ACCOUNT_MANIFOLD_CONTRACT, CONTRACT_VERSION)?;
    Ok(Response::default())
}

#[cfg(test)]
mod tests {
    use cosmwasm_std::{coin, Addr, MessageInfo, Uint256};
    use terp_account::CURRENT_BASE_PRICE;

    use crate::commands::{validate_account, validate_payment};

    #[test]
    fn check_validate_account() {
        let min = 3;
        let max = 63;
        assert!(validate_account("bobo", min, max).is_ok());
        assert!(validate_account("-bobo", min, max).is_err());
        assert!(validate_account("bobo-", min, max).is_err());
        assert!(validate_account("bo-bo", min, max).is_ok());
        assert!(validate_account("bo--bo", min, max).is_err());
        assert!(validate_account("bob--o", min, max).is_ok());
        assert!(validate_account("bo", min, max).is_err());
        assert!(validate_account("b", min, max).is_err());
        assert!(validate_account("bob", min, max).is_ok());
        assert!(validate_account(
            "bobobobobobobobobobobobobobobobobobobobobobobobobobobobobobobo",
            min,
            max
        )
        .is_ok());
        assert!(validate_account(
            "bobobobobobobobobobobobobobobobobobobobobobobobobobobobobobobob",
            min,
            max
        )
        .is_err());
        assert!(validate_account("0123456789", min, max).is_ok());
        assert!(validate_account("😬", min, max).is_err());
        assert!(validate_account("BOBO", min, max).is_err());
        assert!(validate_account("b-o----b", min, max).is_ok());
        assert!(validate_account("bobo.stars", min, max).is_err());
    }

    #[test]
    fn check_validate_payment() {
        let base_price = CURRENT_BASE_PRICE as u128;

        let info = MessageInfo {
            sender: Addr::unchecked("sender"),
            funds: vec![coin(base_price, "uthiol")],
        };
        assert_eq!(
            validate_payment(5, &info, base_price)
                .unwrap()
                .unwrap()
                .amount,
            Uint256::new(base_price)
        );

        let info = MessageInfo {
            sender: Addr::unchecked("sender"),
            funds: vec![coin(base_price * 10, "uthiol")],
        };
        assert_eq!(
            validate_payment(4, &info, base_price)
                .unwrap()
                .unwrap()
                .amount,
            Uint256::new(base_price * 10)
        );

        let info = MessageInfo {
            sender: Addr::unchecked("sender"),
            funds: vec![coin(base_price * 100, "uthiol")],
        };
        assert_eq!(
            validate_payment(3, &info, base_price)
                .unwrap()
                .unwrap()
                .amount,
            Uint256::new(base_price * 100)
        );
    }
}
