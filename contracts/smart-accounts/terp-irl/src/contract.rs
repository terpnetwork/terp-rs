use crate::claims::Proof;
use crate::error::ContractError;
use crate::msg::{
    CreateFantokenMsg, ExecuteMsg, FantokenInfo, GetAllEpochResponse, GetEpochResponse,
    InstantiateMsg, MintFantokenMsg, MintTicketObject, QueryMsg, SetFantokenUriMsg,
};
use crate::state::{
    Config, Epoch, Witness, CONFIG, EPOCHS, FANTOKEN_INFO, MINTED_AMOUNTS, WAVS_SMART_ACCOUNT,
    ZKTLS_ENABLED,
};
use crate::TerpAccountIrl;
use terp_account::traits::default::TerpAccountTrait;
use terp_auth::{
    AuthenticationRequest, ConfirmExecutionRequest, OnAuthenticatorAddedRequest,
    OnAuthenticatorRemovedRequest, TrackRequest,
};
use cosmwasm_std::{
    coin, from_json, to_json_binary, Addr, AnyMsg, Binary, Deps, DepsMut, Env, MessageInfo, Reply,
    Response, StdError, StdResult, SubMsg, Uint128,
};
use cosmwasm_std::{entry_point, Event, Timestamp};
use cw2::set_contract_version;
use sha2::{Digest, Sha256};

// Constants
const CONTRACT_NAME: &str = "crates.io:terp-irl";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
const FANTOKEN_CREATE_REPLY_ID: u64 = 1;

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    InstantiateMsg {
        enable_zktls,
        minter_params,
    }: InstantiateMsg,
) -> Result<Response, ContractError> {
    let mut submsg = vec![];
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    let config = Config {
        owner: info.sender.to_string(),
        current_epoch: Uint128::zero(),
    };
    if let Some(a) = minter_params {
        // Store fantoken info (denom will be updated in reply)
        let fantoken_info = FantokenInfo {
            symbol: a.symbol.clone(),
            name: a.name.clone(),
            max_supply: a.max_supply,
            authority: env.contract.address.to_string(),
            uri: a.uri.clone(),
            minter: env.contract.address.to_string(),
            denom: String::new(), // Will be set in reply
        };
        // Create the fantoken creation message
        let create_msg = AnyMsg {
            type_url: "rs".into(),
            value: to_json_binary(&CreateFantokenMsg {
                symbol: a.symbol.clone(),
                name: a.name.clone(),
                max_supply: a.max_supply.to_string(),
                authority: env.contract.address.to_string(),
                uri: a.uri.clone(),
                minter: env.contract.address.to_string(),
            })?,
        };
        FANTOKEN_INFO.save(deps.storage, &fantoken_info)?;
        submsg.push(SubMsg::reply_on_success(
            create_msg,
            FANTOKEN_CREATE_REPLY_ID,
        ))
    }

    CONFIG.save(deps.storage, &config)?;
    WAVS_SMART_ACCOUNT.save(deps.storage, &info.sender.to_string())?;
    ZKTLS_ENABLED.save(deps.storage, &enable_zktls)?;

    Ok(Response::new()
        .add_submessages(submsg)
        .add_attribute("method", "instantiate")
        .add_attribute("smart_account", info.sender))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(deps: DepsMut, _env: Env, msg: Reply) -> Result<Response, ContractError> {
    match msg.id {
        FANTOKEN_CREATE_REPLY_ID => {
            let mut fantoken_info = FANTOKEN_INFO.load(deps.storage)?;
            fantoken_info.denom = format!("ft{}", fantoken_info.symbol.to_lowercase());

            FANTOKEN_INFO.save(deps.storage, &fantoken_info)?;

            Ok(Response::new()
                .add_attribute("method", "fantoken_created")
                .add_attribute("denom", &fantoken_info.denom))
        }
        _ => Err(ContractError::UnknownReplyId {}),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::MintFantokens { data } => handle_mint_fantokens(deps, env, info, data),
        ExecuteMsg::SetUri { uri } => handle_set_uri(deps, env, info, uri),
        ExecuteMsg::AddEpoch {
            witness,
            minimum_witness,
        } => add_epoch(deps, env, witness, minimum_witness, info.sender.clone()),
    }
}

// @dev - add epoch
pub fn add_epoch(
    deps: DepsMut,
    env: Env,
    witness: Vec<Witness>,
    minimum_witness: Uint128,
    sender: Addr,
) -> Result<Response, ContractError> {
    // load configs
    let mut config = CONFIG.load(deps.storage)?;

    if config.owner != sender.to_string() {
        return Err(ContractError::Unauthorized {});
    }

    //Increment Epoch number
    let new_epoch = config.current_epoch + Uint128::one();
    // Create the new epoch
    let epoch = Epoch {
        id: new_epoch,
        witness,
        timestamp_start: env.block.time.nanos(),
        timestamp_end: env.block.time.plus_seconds(86400).nanos(),
        minimum_witness_for_claim_creation: minimum_witness,
    };

    // Upsert the new epoch into memory
    EPOCHS.save(deps.storage, new_epoch.into(), &epoch)?;

    // Save the new epoch
    config.current_epoch = new_epoch;
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::default())
}

fn handle_mint_fantokens(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    mint_requests: Vec<MintTicketObject>,
) -> Result<Response, ContractError> {
    // Ensure only the smart account can mint fantokens

    if info.sender.to_string() != WAVS_SMART_ACCOUNT.load(deps.storage)? {
        return Err(ContractError::Unauthorized {});
    }
    let fantoken_info = FANTOKEN_INFO.load(deps.storage)?;
    if fantoken_info.denom.is_empty() {
        return Err(ContractError::FantokenNotCreated {});
    }

    let mut mint_msgs = Vec::new();
    let mut total_to_mint = Uint128::zero();

    for mint_request in mint_requests {
        let amount = Uint128::from(mint_request.amount);
        total_to_mint += amount;

        let mint_msg = AnyMsg {
            type_url: "/bitsong.fantoken.v1beta1.MsgMint".into(),
            value: to_json_binary(&MintFantokenMsg {
                recipient: mint_request.ticket, // recipient address
                coin: coin(amount.u128(), fantoken_info.denom.clone()),
                minter: env.contract.address.to_string(),
            })?,
        };

        mint_msgs.push(mint_msg);
    }

    // Check if minting would exceed max supply
    let current_minted = MINTED_AMOUNTS
        .may_load(deps.storage, &fantoken_info.denom)?
        .unwrap_or_default();

    if current_minted + total_to_mint > fantoken_info.max_supply {
        return Err(ContractError::ExceedsMaxSupply {});
    }

    // Update minted amount
    MINTED_AMOUNTS.save(
        deps.storage,
        &fantoken_info.denom,
        &(current_minted + total_to_mint),
    )?;

    Ok(Response::new().add_messages(mint_msgs))
}

fn handle_set_uri(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    new_uri: String,
) -> Result<Response, ContractError> {
    // Ensure only the smart account can update URI
    if info.sender.to_string() != WAVS_SMART_ACCOUNT.load(deps.storage)? {
        return Err(ContractError::Unauthorized {});
    }

    let mut fantoken_info = FANTOKEN_INFO.load(deps.storage)?;

    if fantoken_info.denom.is_empty() {
        return Err(ContractError::FantokenNotCreated {});
    }

    // Update stored URI
    fantoken_info.uri = new_uri.clone();
    FANTOKEN_INFO.save(deps.storage, &fantoken_info)?;

    let set_uri_msg = AnyMsg {
        type_url: "/bitsong.fantoken.v1beta1.MsgSetUri".into(),
        value: to_json_binary(&SetFantokenUriMsg {
            denom: fantoken_info.denom,
            uri: new_uri,
            authority: env.contract.address.to_string(),
        })?,
    };

    Ok(Response::new().add_message(set_uri_msg))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::GetFantokenInfo {} => {
            let info = FANTOKEN_INFO.may_load(deps.storage)?;
            to_json_binary(&info)
        }
        QueryMsg::GetMintedAmount {} => {
            let fantoken_info = FANTOKEN_INFO.load(deps.storage)?;
            let minted = MINTED_AMOUNTS
                .may_load(deps.storage, &fantoken_info.denom)?
                .unwrap_or_default();
            to_json_binary(&minted)
        }
        QueryMsg::GetEpoch { id } => to_json_binary(&query_epoch_id(deps, id)?),
        QueryMsg::GetAllEpoch {} => to_json_binary(&query_all_epoch_ids(deps)?),
    }
}

//NOTE: Unimplemented as secret doesn't allow to iterate via keys
fn query_all_epoch_ids(_deps: Deps) -> StdResult<GetAllEpochResponse> {
    Ok(GetAllEpochResponse { ids: vec![] })
}

fn query_epoch_id(deps: Deps, id: u128) -> StdResult<GetEpochResponse> {
    match EPOCHS.may_load(deps.storage, id)? {
        Some(epoch) => Ok(GetEpochResponse { epoch }),
        None => Err(StdError::msg("No such epoch")),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn sudo(deps: DepsMut, env: Env, msg: crate::SudoMsg) -> Result<Response, ContractError> {
    match ZKTLS_ENABLED.load(deps.storage)? {
        true => TerpAccountIrl::process_sudo_auth(deps, env, &msg),
        false => Ok(Response::new()),
    }
}
