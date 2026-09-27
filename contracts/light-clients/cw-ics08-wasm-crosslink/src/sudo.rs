//! Sudo entry point handlers for the Crosslink light client.

use crate::ContractError;
use crate::{
    msg::{MisbehaviourMsg, UpdateStateMsg, VerifyMembershipMsg, VerifyNonMembershipMsg},
    state,
};
use cosmwasm_std::{Deps, DepsMut};
use crosslink_light_client::ZcashSerialize;
use crosslink_light_client::{
    ZcashDeserialize, header::CrosslinkHeader, membership, update, verify,
};
use ibc_proto::ibc::core::client::v1::Height;

/// Verify membership proof.
///
/// # Errors
///
/// Returns [`ContractError`] if the proof is invalid.
pub fn verify_membership(
    deps: Deps<CrosslinkCustomQuery>,
    msg: VerifyMembershipMsg,
) -> Result<Vec<u8>, ContractError> {
    let consensus_state =
        state::get_crosslink_consensus_state(deps.storage, msg.height.revision_height)?;
    let client_state = state::get_crosslink_client_state(deps.storage)?;

    membership::verify_membership(
        consensus_state,
        client_state,
        msg.proof.to_vec(),
        msg.merkle_path
            .key_path
            .into_iter()
            .map(|b| b.to_vec())
            .collect(),
        msg.value.to_vec(),
    )
    .map_err(ContractError::VerifyMembershipFailed)?;

    Ok(vec![])
}

/// Verify non-membership proof.
///
/// # Errors
///
/// Returns [`ContractError`] if the proof is invalid.
pub fn verify_non_membership(
    deps: Deps<CrosslinkCustomQuery>,
    msg: VerifyNonMembershipMsg,
) -> Result<Vec<u8>, ContractError> {
    let consensus_state =
        state::get_crosslink_consensus_state(deps.storage, msg.height.revision_height)?;
    let client_state = state::get_crosslink_client_state(deps.storage)?;

    membership::verify_non_membership(
        consensus_state,
        client_state,
        msg.proof.to_vec(),
        msg.merkle_path
            .key_path
            .into_iter()
            .map(|b| b.to_vec())
            .collect(),
    )
    .map_err(ContractError::VerifyNonMembershipFailed)?;

    Ok(vec![])
}

/// Update the client state from a verified header.
///
/// # Errors
///
/// Returns [`ContractError`] if the update fails.
pub fn update_state(
    deps: DepsMut<CrosslinkCustomQuery>,
    msg: UpdateStateMsg,
) -> Result<Vec<u8>, ContractError> {
    let header: CrosslinkHeader =
        CrosslinkHeader::zcash_deserialize(std::io::Cursor::new(&msg.client_message))?;
    let client_state = state::get_crosslink_client_state(deps.as_ref().storage)?;
    let consensus_state = state::get_crosslink_consensus_state(
        deps.as_ref().storage,
        client_state.latest_bft_height,
    )?;

    // Verify the header
    verify::verify_header(deps.api, &client_state, &consensus_state, &header)
        .map_err(ContractError::VerifyClientMessageFailed)?;

    // Update consensus state and client state
    let (new_height, new_consensus_state, updated_client_state) =
        update::update_consensus_state(consensus_state, client_state, &header)
            .map_err(ContractError::UpdateClientStateFailed)?;

    // Store the new consensus state
    state::store_consensus_state(deps.storage, new_height, &new_consensus_state)?;

    // Store the updated client state
    if let Some(updated_client) = updated_client_state {
        let client_state_bz = updated_client.zcash_serialize_to_vec()?;
        let mut wasm_client_state = state::get_wasm_client_state(deps.as_ref().storage)?;
        wasm_client_state.data = client_state_bz;
        wasm_client_state.latest_height = Some(Height {
            revision_number: 0,
            revision_height: new_height.into(),
        });
        state::store_client_state(deps.storage, &wasm_client_state)?;
    }

    let result = crate::msg::UpdateStateResult {
        heights: vec![crate::msg::Height {
            revision_number: 0,
            revision_height: new_height,
        }],
    };

    Ok(serde_json::to_vec(&result).map_err(ContractError::SerializeConsensusStateFailed)?)
}

/// Handle misbehaviour.
///
/// Freezes the client only when the submitted header conflicts with stored
/// state at the same BFT height (or the client is already frozen). Incomplete
/// evidence fails closed.
///
/// # Errors
///
/// Returns [`ContractError`] if misbehaviour handling fails.
pub fn misbehaviour(
    deps: DepsMut<CrosslinkCustomQuery>,
    msg: MisbehaviourMsg,
) -> Result<Vec<u8>, ContractError> {
    let header: CrosslinkHeader =
        CrosslinkHeader::zcash_deserialize(std::io::Cursor::new(&msg.client_message))?;

    let client_state = state::get_crosslink_client_state(deps.as_ref().storage)?;
    let consensus_state = state::get_crosslink_consensus_state(
        deps.as_ref().storage,
        client_state.latest_bft_height,
    )?;
    let found = verify::check_for_misbehaviour(&client_state, &consensus_state, &header)
        .map_err(ContractError::VerifyClientMessageFailed)?;
    if !found {
        return Err(ContractError::InvalidClientMessage);
    }

    // Freeze the client
    let mut frozen_client = client_state;
    frozen_client.is_frozen = true;
    let client_state_bz = frozen_client.zcash_serialize_to_vec()?;
    let mut wasm_client_state = state::get_wasm_client_state(deps.as_ref().storage)?;
    wasm_client_state.data = client_state_bz;
    state::store_client_state(deps.storage, &wasm_client_state)?;

    Ok(vec![])
}

use crate::custom_query::CrosslinkCustomQuery;
