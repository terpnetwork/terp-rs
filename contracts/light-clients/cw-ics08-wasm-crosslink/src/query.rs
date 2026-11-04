//! Query entry point handlers for the Crosslink light client.

use cosmwasm_std::{Binary, Deps, Env, to_json_binary};
use crosslink_light_client::header::CrosslinkHeader;
use crosslink_light_client::{ZcashDeserialize, verify};

use crate::ContractError;
use crate::custom_query::CrosslinkCustomQuery;
use crate::msg::{
    CheckForMisbehaviourMsg, CheckForMisbehaviourResult, StatusResult, TimestampAtHeightMsg,
    TimestampAtHeightResult, VerifyClientMessageMsg,
};
use crate::state;

/// Verify a client message.
///
/// # Errors
///
/// Returns [`ContractError`] if verification fails.
pub fn verify_client_message(
    deps: Deps<CrosslinkCustomQuery>,
    _env: Env,
    msg: VerifyClientMessageMsg,
) -> Result<Binary, ContractError> {
    let header: CrosslinkHeader =
        CrosslinkHeader::zcash_deserialize(std::io::Cursor::new(msg.client_message))?;

    let client_state = state::get_crosslink_client_state(deps.storage)?;
    let consensus_state =
        state::get_crosslink_consensus_state(deps.storage, client_state.latest_bft_height)?;

    verify::verify_header(deps.api, &client_state, &consensus_state, &header)
        .map_err(ContractError::VerifyClientMessageFailed)?;

    Ok(Binary::default())
}

/// Check for misbehaviour.
///
/// # Errors
///
/// Returns [`ContractError`] if the check fails.
pub fn check_for_misbehaviour(
    deps: Deps<CrosslinkCustomQuery>,
    _env: Env,
    msg: CheckForMisbehaviourMsg,
) -> Result<Binary, ContractError> {
    let header: CrosslinkHeader =
        CrosslinkHeader::zcash_deserialize(std::io::Cursor::new(msg.client_message))?;

    let client_state = state::get_crosslink_client_state(deps.storage)?;
    let consensus_state =
        state::get_crosslink_consensus_state(deps.storage, client_state.latest_bft_height)?;

    let found = verify::check_for_misbehaviour(deps.api, &client_state, &consensus_state, &header)
        .map_err(ContractError::VerifyClientMessageFailed)?;

    to_json_binary(&CheckForMisbehaviourResult {
        found_misbehaviour: found,
    })
    .map_err(ContractError::Std)
}

/// Get the timestamp at a given height.
///
/// # Errors
///
/// Returns [`ContractError`] if the consensus state is not found.
pub fn timestamp_at_height(
    deps: Deps<CrosslinkCustomQuery>,
    msg: &TimestampAtHeightMsg,
) -> Result<Binary, ContractError> {
    let consensus_state =
        state::get_crosslink_consensus_state(deps.storage, msg.height.revision_height)?;

    to_json_binary(&TimestampAtHeightResult {
        timestamp: consensus_state.timestamp,
    })
    .map_err(ContractError::Std)
}

/// Get the status of the client.
///
/// # Errors
///
/// Returns [`ContractError`] if the client state is not found.
pub fn status(deps: Deps<CrosslinkCustomQuery>) -> Result<Binary, ContractError> {
    let client_state = state::get_crosslink_client_state(deps.storage)?;

    let status = if client_state.is_frozen {
        "Frozen".to_string()
    } else {
        "Active".to_string()
    };

    to_json_binary(&StatusResult { status }).map_err(ContractError::Std)
}
