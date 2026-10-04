use crate::{
    admin::{
        can_execute, execute_freeze, execute_update_admins, query_admin_list, query_can_execute,
    },
    error::ContractError,
    msg::{
        ExecuteMsg, HasMemberResponse, InstantiateMsg, MerkleRootResponse, MerkleTreeURIResponse,
        QueryMsg,
    },
    state::{AdminList, ADMIN_LIST, MERKLE_ROOT, MERKLE_TREE_URI},
};

#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    to_json_binary, Binary, Deps, DepsMut, Empty, Env, Event, MessageInfo, Response, StdError,
    StdResult,
};
use cw2::set_contract_version;
use mtree_tooling::helpers::{
    map_validate, string_to_hash, valid_hash_string, verify_merkle_root, verify_tree_uri,
};

use semver::Version;

// version info for migration info
pub const WLIST_MERKLETREE: &str = "whitelist-mtree";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

// contract governance params
pub const CREATION_FEE: u128 = 1_000_000_000;
pub const MIN_MINT_PRICE: u128 = 0;

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    verify_merkle_root(&msg.merkle_root)?;
    verify_tree_uri(&msg.merkle_tree_uri)?;
    set_contract_version(deps.storage, WLIST_MERKLETREE, CONTRACT_VERSION)?;

    let admin_config = AdminList {
        admins: map_validate(deps.api, &msg.admins)?,
        mutable: msg.admins_mutable,
    };

    MERKLE_ROOT.save(deps.storage, &msg.merkle_root)?;
    ADMIN_LIST.save(deps.storage, &admin_config)?;

    let tree_url = msg.merkle_tree_uri.unwrap_or_default();

    let mut attrs = Vec::with_capacity(6);
    attrs.push(("action", "update_merkle_tree"));
    attrs.push(("merkle_root", &msg.merkle_root));
    attrs.push(("contract_name", WLIST_MERKLETREE));
    attrs.push(("contract_version", CONTRACT_VERSION));
    if !tree_url.is_empty() {
        attrs.push(("merkle_tree_uri", &tree_url));
    }
    attrs.push(("sender", info.sender.as_str()));

    Ok(Response::new().add_attributes(attrs))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::UpdateAdmins { admins } => execute_update_admins(deps, env, info, admins),
        ExecuteMsg::Freeze {} => execute_freeze(deps, env, info),
    }
}

pub fn execute_update_merkle_tree(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    merkle_root: String,
    merkle_tree_uri: Option<String>,
) -> Result<Response, ContractError> {
    can_execute(&deps, info.sender.clone())?;
    verify_merkle_root(&merkle_root)?;
    verify_tree_uri(&merkle_tree_uri)?;

    MERKLE_ROOT.save(deps.storage, &merkle_root)?;

    let mut attrs = Vec::with_capacity(4);

    attrs.push(("action", String::from("update_merkle_tree")));
    attrs.push(("merkle_root", merkle_root));
    if let Some(uri) = merkle_tree_uri {
        attrs.push(("merkle_tree_uri", uri));
    }
    attrs.push(("sender", info.sender.to_string()));

    Ok(Response::new().add_attributes(attrs))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::HasMember {
            member,
            proof_hashes,
        } => to_json_binary(&query_has_member(deps, member, proof_hashes)?),
        QueryMsg::AdminList {} => to_json_binary(&query_admin_list(deps)?),
        QueryMsg::CanExecute { sender, .. } => to_json_binary(&query_can_execute(deps, &sender)?),
        QueryMsg::MerkleRoot {} => to_json_binary(&query_merkle_root(deps)?),
        QueryMsg::MerkleTreeURI {} => to_json_binary(&query_merkle_tree_uri(deps)?),
    }
}

pub fn query_has_member(
    deps: Deps,
    composite_key: String,
    proof_hashes: Vec<String>,
) -> StdResult<HasMemberResponse> {
    let merkle_root = MERKLE_ROOT.load(deps.storage)?;
    let member_init_hash_slice = blake3::hash(composite_key.as_bytes());
    let final_hash = proof_hashes.into_iter().try_fold(
        member_init_hash_slice,
        |accum_hash_slice, new_proof_hashstring| {
            valid_hash_string(&new_proof_hashstring)?;

            let mut hash_bytes = [
                *accum_hash_slice.as_bytes(),
                *string_to_hash(&new_proof_hashstring)?.as_bytes(),
            ];
            hash_bytes.sort_unstable();
            Result::<blake3::Hash, StdError>::Ok(blake3::hash(&hash_bytes.concat()))
        },
    );

    if final_hash.is_err() {
        return Err(cosmwasm_std::StdError::msg(
            "Invalid Merkle Proof".to_string(),
        ));
    }

    Ok(HasMemberResponse {
        has_member: merkle_root == hex::encode(final_hash.unwrap().as_bytes()),
    })
}

pub fn query_merkle_root(deps: Deps) -> StdResult<MerkleRootResponse> {
    Ok(MerkleRootResponse {
        merkle_root: MERKLE_ROOT.load(deps.storage)?,
    })
}

pub fn query_merkle_tree_uri(deps: Deps) -> StdResult<MerkleTreeURIResponse> {
    Ok(MerkleTreeURIResponse {
        merkle_tree_uri: MERKLE_TREE_URI.may_load(deps.storage)?,
    })
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, _env: Env, _msg: Empty) -> Result<Response, ContractError> {
    let current_version = cw2::get_contract_version(deps.storage)?;
    if current_version.contract != WLIST_MERKLETREE {
        return Err(StdError::msg("Cannot upgrade to a different contract").into());
    }
    let version: Version = current_version
        .version
        .parse()
        .map_err(|_| StdError::msg("Invalid contract version"))?;
    let new_version: Version = CONTRACT_VERSION
        .parse()
        .map_err(|_| StdError::msg("Invalid contract version"))?;

    if version > new_version {
        return Err(StdError::msg("Cannot upgrade to a previous contract version").into());
    }
    // if same version return
    if version == new_version {
        return Ok(Response::new());
    }

    // set new contract version
    set_contract_version(deps.storage, WLIST_MERKLETREE, CONTRACT_VERSION)?;
    let event = Event::new("migrate")
        .add_attribute("from_name", current_version.contract)
        .add_attribute("from_version", current_version.version)
        .add_attribute("to_name", WLIST_MERKLETREE)
        .add_attribute("to_version", CONTRACT_VERSION);
    Ok(Response::new().add_event(event))
}
