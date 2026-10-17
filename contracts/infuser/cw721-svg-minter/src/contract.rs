#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    instantiate2_address, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Order, Response,
    StdResult, WasmMsg,
};
use cw2::set_contract_version;
use cw_storage_plus::Bound;
use sha2::{Digest, Sha256};

use crate::error::ContractError;
use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
use crate::msg::MigrateMsg;
use crate::state::{
    svg_collections, SvgCollection, COLLECTION_NONCE, DREGG_CHANNEL, DREGG_IBC_ONLY, DREGG_PORT,
    RUNTIME_MINTERS, SVG_CODE_ID,
};

pub(crate) const CONTRACT_NAME: &str = "cw-svg-minter";
pub(crate) const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const DEFAULT_LIMIT: u32 = 10;
pub const MAX_LIMIT: u32 = 50;

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    cw_ownable::initialize_owner(deps.storage, deps.api, msg.owner.as_deref())?;
    SVG_CODE_ID.save(deps.storage, &msg.svg_code_id)?;
    COLLECTION_NONCE.save(deps.storage, &0u64)?;
    DREGG_IBC_ONLY.save(deps.storage, &msg.dregg_ibc_only)?;
    for a in msg.runtime_minters {
        let addr = deps.api.addr_validate(&a)?;
        RUNTIME_MINTERS.save(deps.storage, &addr, &())?;
    }
    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("creator", info.sender)
        .add_attribute("dregg_ibc_only", msg.dregg_ibc_only.to_string()))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, _env: Env, msg: MigrateMsg) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    if let Some(ch) = msg.dregg_channel {
        DREGG_CHANNEL.save(deps.storage, &ch)?;
    }
    if let Some(p) = msg.dregg_port {
        DREGG_PORT.save(deps.storage, &p)?;
    }
    if let Some(only) = msg.dregg_ibc_only {
        DREGG_IBC_ONLY.save(deps.storage, &only)?;
    }
    if let Some(addrs) = msg.runtime_minters {
        for a in addrs {
            let addr = deps.api.addr_validate(&a)?;
            RUNTIME_MINTERS.save(deps.storage, &addr, &())?;
        }
    }
    Ok(Response::new()
        .add_attribute("action", "migrate")
        .add_attribute("to_version", CONTRACT_VERSION))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::CreateSvgCollection {
            instantiate_msg,
            label,
        } => execute_create_svg_collection(deps, env, info, instantiate_msg, label),
        ExecuteMsg::UpdateOwnership(action) => {
            let ownership = cw_ownable::update_ownership(deps, &env.block, &info.sender, action)?;
            Ok(Response::new().add_attributes(ownership.into_attributes()))
        }
        ExecuteMsg::UpdateCodeId { code_id } => execute_update_code_id(deps, info, code_id),
        ExecuteMsg::RecvDreggPacket { packet } => {
            if !crate::state::RUNTIME_MINTERS.has(deps.storage, &info.sender) {
                return Err(ContractError::Unauthorized {});
            }
            crate::ibc::accept_execute(deps, packet.clone())?;
            let mut res = Response::new().add_attribute("action", "recv_dregg_packet");
            if !packet.collection.is_empty() {
                let mint = br#"{"update_extension":{"msg":{"mint":{"alloc":0,"amnt":1,"proof_hashes":[],"ext":null}}}}"#;
                res = res.add_message(WasmMsg::Execute {
                    contract_addr: packet.collection,
                    msg: mint.to_vec().into(),
                    funds: vec![],
                });
            }
            Ok(res)
        }
        ExecuteMsg::BindDreggChannel { channel, port } => {
            if !crate::state::RUNTIME_MINTERS.has(deps.storage, &info.sender) {
                return Err(ContractError::Unauthorized {});
            }
            crate::state::DREGG_CHANNEL.save(deps.storage, &channel)?;
            crate::state::DREGG_PORT.save(deps.storage, &port)?;
            Ok(Response::new().add_attribute("action", "bind_dregg_channel"))
        }
    }
}

pub fn execute_create_svg_collection(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    instantiate_msg: cw721_svg::InstantiateMsg,
    label: String,
) -> Result<Response, ContractError> {
    // If owner is set, only owner may create collections.
    let ownership = cw_ownable::get_ownership(deps.storage)?;
    if ownership
        .owner
        .as_ref()
        .is_some_and(|owner| *owner != info.sender)
    {
        return Err(ContractError::Unauthorized {});
    }

    // Fail early (before instantiate2) on bad templates, for both input paths.
    let (template, _) = cw_svg_pack::unpack(&instantiate_msg.collection_info_extension.packed)
        .map_err(|e| ContractError::Std(cosmwasm_std::StdError::msg(e.to_string())))?;
    cw_svg::validate_animated_paint(&template)?;

    let code_id = SVG_CODE_ID.load(deps.storage)?;

    // Fetch code checksum required for instantiate2 address derivation.
    let code_info = deps.querier.query_wasm_code_info(code_id)?;

    // The "creator" in the instantiate2 formula is the factory contract itself.
    let sender = info.sender.clone();
    let creator = deps.api.addr_canonicalize(env.contract.address.as_str())?;
    let tru_creator = match &instantiate_msg.creator {
        Some(c) => c,
        None => &sender.to_string(),
    };

    // Build a unique, deterministic salt: sha256(sender_bytes || nonce_le_bytes).
    // Using the sender ensures different creators can't collide even at the same nonce.
    let nonce = COLLECTION_NONCE.load(deps.storage)?;
    let mut hasher = Sha256::new();
    hasher.update(sender.as_bytes());
    hasher.update(nonce.to_le_bytes());
    let salt_bytes: [u8; 32] = hasher.finalize().into();
    let salt = Binary::from(salt_bytes.as_slice());

    // Derive the address before dispatching — no reply handler needed.
    let predicted_canonical =
        instantiate2_address(code_info.checksum.as_slice(), &creator, salt.as_slice())
            .map_err(|e| ContractError::Instantiate2AddressError { msg: e.to_string() })?;
    let predicted_addr = deps.api.addr_humanize(&predicted_canonical)?;
    // Record the collection before sending the sub-message.
    svg_collections().save(
        deps.storage,
        predicted_addr.as_ref(),
        &SvgCollection {
            contract: predicted_addr.to_string(),
            creator: tru_creator.to_string(),
            name: instantiate_msg.name.clone(),
            symbol: instantiate_msg.symbol.clone(),
        },
    )?;

    // Advance nonce so the next collection gets a different salt.
    COLLECTION_NONCE.save(deps.storage, &(nonce + 1))?;

    let wasm_msg = WasmMsg::Instantiate2 {
        admin: Some(tru_creator.to_string()),
        code_id,
        label,
        msg: to_json_binary(&instantiate_msg)?,
        funds: info.funds,
        salt,
    };

    Ok(Response::new()
        .add_attribute("action", "create_svg_collection")
        .add_attribute("contract", predicted_addr.to_string())
        .add_attribute("creator", info.sender.to_string())
        .add_message(wasm_msg))
}

pub fn execute_update_code_id(
    deps: DepsMut,
    info: MessageInfo,
    code_id: u64,
) -> Result<Response, ContractError> {
    cw_ownable::assert_owner(deps.storage, &info.sender)?;
    SVG_CODE_ID.save(deps.storage, &code_id)?;
    Ok(Response::new()
        .add_attribute("action", "update_code_id")
        .add_attribute("code_id", code_id.to_string()))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::ListSvgCollections { start_after, limit } => {
            let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
            let start = start_after.as_deref().map(Bound::exclusive);

            let res: Vec<SvgCollection> = svg_collections()
                .range(deps.storage, start, None, Order::Ascending)
                .take(limit)
                .flat_map(|item| Ok::<SvgCollection, ContractError>(item?.1))
                .collect();

            to_json_binary(&res)
        }
        QueryMsg::ListSvgCollectionsReverse {
            start_before,
            limit,
        } => {
            let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
            let start = start_before.as_deref().map(Bound::exclusive);

            let res: Vec<SvgCollection> = svg_collections()
                .range(deps.storage, None, start, Order::Descending)
                .take(limit)
                .flat_map(|item| Ok::<SvgCollection, ContractError>(item?.1))
                .collect();

            to_json_binary(&res)
        }
        QueryMsg::ListSvgCollectionsByCreator {
            creator,
            start_after,
            limit,
        } => {
            let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
            let start = start_after.map(Bound::<String>::exclusive);

            deps.api.addr_validate(&creator)?;

            let res: Vec<SvgCollection> = svg_collections()
                .idx
                .creator
                .prefix(creator)
                .range(deps.storage, start, None, Order::Ascending)
                .take(limit)
                .flat_map(|item| Ok::<SvgCollection, ContractError>(item?.1))
                .collect();

            to_json_binary(&res)
        }
        QueryMsg::ListSvgCollectionsByCreatorReverse {
            creator,
            start_before,
            limit,
        } => {
            let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
            let start = start_before.map(Bound::<String>::exclusive);

            deps.api.addr_validate(&creator)?;

            let res: Vec<SvgCollection> = svg_collections()
                .idx
                .creator
                .prefix(creator)
                .range(deps.storage, None, start, Order::Descending)
                .take(limit)
                .flat_map(|item| Ok::<SvgCollection, ContractError>(item?.1))
                .collect();

            to_json_binary(&res)
        }
        QueryMsg::Ownership {} => to_json_binary(&cw_ownable::get_ownership(deps.storage)?),
        QueryMsg::CodeId {} => to_json_binary(&SVG_CODE_ID.load(deps.storage)?),
        QueryMsg::DreggIbc {} => to_json_binary(&crate::msg::DreggIbcStatus {
            channel: DREGG_CHANNEL.may_load(deps.storage)?,
            port: DREGG_PORT.may_load(deps.storage)?,
            ibc_only: DREGG_IBC_ONLY.may_load(deps.storage)?.unwrap_or(false),
        }),
    }
}
