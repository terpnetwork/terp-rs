//! ibc-go v11 08-wasm contract API (types/contract_api.go).

use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::Binary;

#[cw_serde]
pub struct InstantiateMsg {
    pub client_state: Binary,
    pub consensus_state: Binary,
    pub checksum: Binary,
}

#[cw_serde]
pub enum ExecuteMsg {
    /// Lab path (x/wasm instantiate): ICS-23 IAVL proof. Empty `value_hex` = non-membership.
    ConfirmIcs23 {
        store_name: String,
        store_root_hex: String,
        proof_hex: String,
        key_hex: String,
        value_hex: String,
    },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(())]
    VerifyClientMessage(VerifyClientMessageMsg),
    #[returns(CheckForMisbehaviourResult)]
    CheckForMisbehaviour(CheckForMisbehaviourMsg),
    #[returns(TimestampAtHeightResult)]
    TimestampAtHeight(TimestampAtHeightMsg),
    #[returns(StatusResult)]
    Status(StatusMsg),
}

#[cw_serde]
pub enum SudoMsg {
    VerifyMembership(VerifyMembershipMsg),
    VerifyNonMembership(VerifyNonMembershipMsg),
    UpdateState(UpdateStateMsg),
    UpdateStateOnMisbehaviour(MisbehaviourMsg),
    VerifyUpgradeAndUpdateState(VerifyUpgradeAndUpdateStateMsg),
    MigrateClientStore(MigrateClientStoreMsg),
}

#[cw_serde]
pub struct VerifyClientMessageMsg {
    pub client_message: Binary,
}

#[cw_serde]
pub struct CheckForMisbehaviourMsg {
    pub client_message: Binary,
}

#[cw_serde]
pub struct TimestampAtHeightMsg {
    pub height: Height,
}

#[cw_serde]
pub struct StatusMsg {}

#[cw_serde]
pub struct VerifyMembershipMsg {
    pub height: Height,
    pub delay_time_period: u64,
    pub delay_block_period: u64,
    pub proof: Binary,
    pub merkle_path: MerklePath,
    pub value: Binary,
}

#[cw_serde]
pub struct VerifyNonMembershipMsg {
    pub height: Height,
    pub delay_time_period: u64,
    pub delay_block_period: u64,
    pub proof: Binary,
    pub merkle_path: MerklePath,
}

#[cw_serde]
pub struct UpdateStateMsg {
    pub client_message: Binary,
}

#[cw_serde]
pub struct MisbehaviourMsg {
    pub client_message: Binary,
}

#[cw_serde]
pub struct VerifyUpgradeAndUpdateStateMsg {
    pub upgrade_client_state: Binary,
    pub upgrade_consensus_state: Binary,
    pub proof_upgrade_client: Binary,
    pub proof_upgrade_consensus_state: Binary,
}

#[cw_serde]
pub struct MigrateClientStoreMsg {}

#[cw_serde]
pub struct Height {
    #[serde(default)]
    pub revision_number: u64,
    #[serde(default)]
    pub revision_height: u64,
}

#[cw_serde]
pub struct MerklePath {
    pub key_path: Vec<Binary>,
}

#[cw_serde]
pub struct UpdateStateResult {
    pub heights: Vec<Height>,
}

#[cw_serde]
pub struct StatusResult {
    pub status: String,
}

#[cw_serde]
pub struct CheckForMisbehaviourResult {
    pub found_misbehaviour: bool,
}

#[cw_serde]
pub struct TimestampAtHeightResult {
    pub timestamp: u64,
}
