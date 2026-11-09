//! State management for the Crosslink light client.

use cosmwasm_std::Storage;
use crosslink_light_client::ZcashSerialize;
use crosslink_light_client::consensus_state::ConsensusState as CrosslinkConsensusState;
use crosslink_light_client::{ZcashDeserialize, client_state::ClientState as CrosslinkClientState};
use ibc_proto::{
    google::protobuf::Any,
    ibc::lightclients::wasm::v1::{
        ClientState as WasmClientState, ConsensusState as WasmConsensusState,
    },
};
use prost::Message;
use prost::Name;

use crate::ContractError;

/// The store key used by `ibc-go` to store the client state.
pub const HOST_CLIENT_STATE_KEY: &str = "clientState";
/// The store key used by `ibc-go` to store the consensus states.
pub const HOST_CONSENSUS_STATES_KEY: &str = "consensusStates";

/// The key used to store the consensus states by BFT height.
#[must_use]
pub fn consensus_db_key(height: u32) -> String {
    format!("{}/{}-{}", HOST_CONSENSUS_STATES_KEY, 0, height)
}

/// Get the Wasm client state.
///
/// # Errors
///
/// Returns [`ContractError`] if the client state is missing or fails to decode.
pub fn get_wasm_client_state(storage: &dyn Storage) -> Result<WasmClientState, ContractError> {
    let wasm_client_state_any_bz = storage
        .get(HOST_CLIENT_STATE_KEY.as_bytes())
        .ok_or(ContractError::ClientStateNotFound)?;
    let wasm_client_state_any = Any::decode(wasm_client_state_any_bz.as_slice())?;
    Ok(WasmClientState::decode(
        wasm_client_state_any.value.as_slice(),
    )?)
}

/// Get the Crosslink client state.
///
/// # Errors
///
/// Returns [`ContractError`] if the client state is missing or fails to decode.
pub fn get_crosslink_client_state(
    storage: &dyn Storage,
) -> Result<CrosslinkClientState, ContractError> {
    let wasm_client_state = get_wasm_client_state(storage)?;
    Ok(CrosslinkClientState::zcash_deserialize(
        std::io::Cursor::new(wasm_client_state.data),
    )?)
}

/// Get the Crosslink consensus state at a given BFT height.
///
/// # Errors
///
/// Returns [`ContractError`] if the consensus state at `height` is missing or fails to decode.
pub fn get_crosslink_consensus_state(
    storage: &dyn Storage,
    height: u32,
) -> Result<CrosslinkConsensusState, ContractError> {
    let wasm_consensus_state_any_bz = storage
        .get(consensus_db_key(height).as_bytes())
        .ok_or(ContractError::ConsensusStateNotFound)?;
    let wasm_consensus_state_any = Any::decode(wasm_consensus_state_any_bz.as_slice())?;
    let wasm_consensus_state =
        WasmConsensusState::decode(wasm_consensus_state_any.value.as_slice())?;
    Ok(CrosslinkConsensusState::zcash_deserialize(
        std::io::Cursor::new(wasm_consensus_state.data),
    )?)
}

/// Store the Wasm client state.
///
/// # Errors
///
/// Returns [`ContractError`] if storing fails.
pub fn store_client_state(
    storage: &mut dyn Storage,
    wasm_client_state: &WasmClientState,
) -> Result<(), ContractError> {
    let any = Any {
        type_url: WasmClientState::type_url(),
        value: wasm_client_state.encode_to_vec(),
    };
    storage.set(HOST_CLIENT_STATE_KEY.as_bytes(), &any.encode_to_vec());
    Ok(())
}

/// Store a Crosslink consensus state at a given BFT height.
///
/// # Errors
///
/// Returns [`ContractError`] if the consensus state fails to serialize.
pub fn store_consensus_state(
    storage: &mut dyn Storage,
    height: u32,
    consensus_state: &CrosslinkConsensusState,
) -> Result<(), ContractError> {
    let data = consensus_state.zcash_serialize_to_vec()?;
    let wasm_consensus_state = WasmConsensusState { data };

    let any = Any {
        type_url: WasmConsensusState::type_url(),
        value: wasm_consensus_state.encode_to_vec(),
    };
    storage.set(consensus_db_key(height).as_bytes(), &any.encode_to_vec());
    Ok(())
}
