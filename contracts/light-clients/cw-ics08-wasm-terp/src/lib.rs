//! 08-wasm client for Terp IAVL (SHA-256, hybrid, or full BLAKE3).
//!
//! Inner client/consensus bytes are ICS-07 Tendermint protobufs (Hermes
//! `--wasm-checksum` wrapping) or lab JSON `{hasher_mode, root_hex}`.
//! VerifyMembership picks `Blake3IavlSpec` vs `IavlSpec` from the proof's
//! ICS-23 `HashOp` when `hasher_mode` is `auto` (TM wrap). Header signatures
//! are not checked (lab); UpdateState extracts app hash + height.

mod header;
pub mod msg;
mod state;

use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdError,
};
use terp_iavl_lc::{
    verify_chained_membership, verify_chained_nonmembership, verify_ics23_existence,
    verify_ics23_nonexistence, HashAlg,
};

use header::parse_client_message;
use msg::*;
use state::{
    encode_inner_client, encode_inner_consensus, load_checksum, load_inner_client,
    load_inner_consensus, parse_inner_client, parse_inner_consensus, store_wasm_client,
    store_wasm_consensus,
};

const CONTRACT_NAME: &str = "crates.io:cw-ics08-wasm-terp";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(thiserror::Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),
    #[error("proof: {0}")]
    Proof(String),
    #[error("hex: {0}")]
    Hex(String),
    #[error("json: {0}")]
    Json(String),
    #[error("prost: {0}")]
    Prost(String),
    #[error("client state not found")]
    ClientStateNotFound,
    #[error("consensus state not found")]
    ConsensusStateNotFound,
    #[error("invalid client message")]
    InvalidClientMessage,
    #[error("frozen")]
    Frozen,
}

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    let mut inner = parse_inner_client(&msg.client_state)?;
    if inner.hasher_mode.is_empty() {
        inner.hasher_mode = "hybrid".into();
    }
    let cons = parse_inner_consensus(&msg.consensus_state)?;
    let data = encode_inner_client(&inner)?;
    store_wasm_client(
        deps.storage,
        data,
        msg.checksum.to_vec(),
        inner.latest_height.clone(),
    )?;
    store_wasm_consensus(
        deps.storage,
        &inner.latest_height,
        encode_inner_consensus(&cons)?,
    )?;
    Ok(Response::default())
}

#[entry_point]
pub fn sudo(deps: DepsMut, _env: Env, msg: SudoMsg) -> Result<Response, ContractError> {
    match msg {
        SudoMsg::VerifyMembership(m) => {
            verify_membership(deps.storage, m)?;
            Ok(Response::default().set_data(Binary::default()))
        }
        SudoMsg::VerifyNonMembership(m) => {
            verify_non_membership(deps.storage, m)?;
            Ok(Response::default().set_data(Binary::default()))
        }
        SudoMsg::UpdateState(m) => {
            let data = update_state(deps.storage, m)?;
            Ok(Response::default().set_data(data))
        }
        SudoMsg::UpdateStateOnMisbehaviour(_) => {
            let mut inner = load_inner_client(deps.storage)?;
            inner.frozen = true;
            let checksum = load_checksum(deps.storage)?;
            let height = inner.latest_height.clone();
            store_wasm_client(
                deps.storage,
                encode_inner_client(&inner)?,
                checksum,
                height,
            )?;
            Ok(Response::default().set_data(Binary::default()))
        }
        SudoMsg::VerifyUpgradeAndUpdateState(_) | SudoMsg::MigrateClientStore(_) => {
            Err(ContractError::InvalidClientMessage)
        }
    }
}

fn verify_membership(
    storage: &dyn cosmwasm_std::Storage,
    msg: VerifyMembershipMsg,
) -> Result<(), ContractError> {
    let inner = load_inner_client(storage)?;
    if inner.frozen {
        return Err(ContractError::Frozen);
    }
    let cons = load_inner_consensus(storage, &msg.height)?;
    let root = cons.root_bytes()?;
    let path: Vec<Vec<u8>> = msg.merkle_path.key_path.iter().map(|b| b.to_vec()).collect();
    verify_chained_membership(
        &inner.hasher_mode,
        &path,
        msg.proof.as_slice(),
        msg.value.as_slice(),
        &root,
    )
    .map_err(|e| ContractError::Proof(e.to_string()))
}

fn verify_non_membership(
    storage: &dyn cosmwasm_std::Storage,
    msg: VerifyNonMembershipMsg,
) -> Result<(), ContractError> {
    let inner = load_inner_client(storage)?;
    if inner.frozen {
        return Err(ContractError::Frozen);
    }
    let cons = load_inner_consensus(storage, &msg.height)?;
    let root = cons.root_bytes()?;
    let path: Vec<Vec<u8>> = msg.merkle_path.key_path.iter().map(|b| b.to_vec()).collect();
    verify_chained_nonmembership(&inner.hasher_mode, &path, msg.proof.as_slice(), &root)
        .map_err(|e| ContractError::Proof(e.to_string()))
}

fn update_state(
    storage: &mut dyn cosmwasm_std::Storage,
    msg: UpdateStateMsg,
) -> Result<Binary, ContractError> {
    let parsed = parse_client_message(msg.client_message.as_slice())?;
    let mut inner = load_inner_client(storage)?;
    if inner.frozen {
        return Err(ContractError::Frozen);
    }
    if parsed.height.revision_height == 0 {
        return Err(ContractError::InvalidClientMessage);
    }
    let checksum = load_checksum(storage)?;
    if parsed.height.revision_height > inner.latest_height.revision_height
        || parsed.height.revision_number > inner.latest_height.revision_number
    {
        inner.latest_height = parsed.height.clone();
        if !parsed.chain_id.is_empty() {
            inner.chain_id = parsed.chain_id.clone();
        }
    }
    store_wasm_consensus(
        storage,
        &parsed.height,
        encode_inner_consensus(&parsed.consensus)?,
    )?;
    store_wasm_client(
        storage,
        encode_inner_client(&inner)?,
        checksum,
        inner.latest_height.clone(),
    )?;
    to_json_binary(&UpdateStateResult {
        heights: vec![parsed.height],
    })
    .map_err(ContractError::Std)
}

#[entry_point]
pub fn execute(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::ConfirmIcs23 {
            store_name,
            store_root_hex,
            proof_hex,
            key_hex,
            value_hex,
        } => {
            let mode = load_inner_client(deps.storage)
                .map(|c| c.hasher_mode)
                .unwrap_or_else(|_| "hybrid".into());
            let alg = HashAlg::for_store_mode(&store_name, &mode);
            let proof = hex::decode(&proof_hex).map_err(|e| ContractError::Hex(e.to_string()))?;
            let key = hex::decode(&key_hex).map_err(|e| ContractError::Hex(e.to_string()))?;
            let root =
                hex::decode(&store_root_hex).map_err(|e| ContractError::Hex(e.to_string()))?;
            if value_hex.is_empty() {
                verify_ics23_nonexistence(alg, &root, &proof, &key)
                    .map_err(|e| ContractError::Proof(e.to_string()))?;
            } else {
                let value =
                    hex::decode(&value_hex).map_err(|e| ContractError::Hex(e.to_string()))?;
                verify_ics23_existence(alg, &root, &proof, &key, &value)
                    .map_err(|e| ContractError::Proof(e.to_string()))?;
            }
            Ok(Response::new().add_attribute("hasher", format!("{alg:?}")))
        }
    }
}

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> Result<Binary, ContractError> {
    match msg {
        QueryMsg::Status(_) => {
            let inner = load_inner_client(deps.storage)?;
            let status = if inner.frozen {
                "Frozen".to_string()
            } else {
                "Active".to_string()
            };
            to_json_binary(&StatusResult { status }).map_err(ContractError::Std)
        }
        QueryMsg::TimestampAtHeight(m) => {
            let cons = load_inner_consensus(deps.storage, &m.height)?;
            to_json_binary(&TimestampAtHeightResult {
                timestamp: cons.timestamp_ns,
            })
            .map_err(ContractError::Std)
        }
        QueryMsg::VerifyClientMessage(m) => {
            let _ = parse_client_message(m.client_message.as_slice())?;
            let inner = load_inner_client(deps.storage)?;
            if inner.frozen {
                return Err(ContractError::Frozen);
            }
            Ok(Binary::default())
        }
        QueryMsg::CheckForMisbehaviour(_) => to_json_binary(&CheckForMisbehaviourResult {
            found_misbehaviour: false,
        })
        .map_err(ContractError::Std),
    }
}
