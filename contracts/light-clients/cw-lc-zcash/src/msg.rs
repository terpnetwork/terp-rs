use cosmwasm_schema::{QueryResponses, cw_serde};

use crate::state::Inclusion;

#[cw_serde]
pub struct InstantiateMsg {
    /// Label with role `client`, e.g. `terp/lc/client/zcash-regtest/v1`.
    pub label: String,
    pub registry: String,
    pub aggregator: String,
    pub registry_client_id: String,
    pub ibc_client_id: String,
    pub accept_preconfirmed: bool,
}

#[cw_serde]
pub struct Zap1Step {
    pub sibling_hex: String,
    pub sibling_is_left: bool,
}

/// IBC v2 packet fields + zap-1 inclusion witness.
#[cw_serde]
pub struct PaymentInclusionMsg {
    pub height: u64,
    pub source_client: String,
    pub dest_client: String,
    pub sequence: u64,
    pub timeout_timestamp_ns: u64,
    pub app_data_hash_hex: String,
    pub note_hex: String,
    pub amount_zat: u64,
    /// FROST group key the payment is bound to (e.g. the auction bid group).
    pub frost_group_hex: String,
    pub leaf_hex: String,
    /// zap-1 event kind: `hosting_payment` | `program_entry` | `ownership_attest`.
    pub event_kind: String,
    pub wallet_or_serial: String,
    pub proof: Vec<Zap1Step>,
}

#[cw_serde]
pub enum ExecuteMsg {
    VerifyPaymentInclusion(Box<PaymentInclusionMsg>),
}

#[cw_serde]
pub struct MigrateMsg {}

#[cw_serde]
pub enum InclusionStatus {
    /// Header still preconfirmed (committee), not yet k-of-n deep.
    Preconfirmed,
    /// Header confirmed against the registry tip at depth: safe to settle.
    Confirmed,
    /// The header at that height changed after the inclusion was recorded.
    Reorged,
}

#[cw_serde]
pub struct PaymentInclusionResponse {
    pub inclusion: Inclusion,
    pub status: InclusionStatus,
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(crate::state::Config)]
    Config {},
    /// Interface for settlement contracts (e.g. private auctions).
    #[returns(Option<PaymentInclusionResponse>)]
    PaymentInclusion { app_data_hash_hex: String },
    /// Best header (committee preconf or k-of-n fallback).
    #[returns(cw_lc_aggregator::msg::BestHeaderResponse)]
    LatestHeader {},
    /// Registry k-of-n tip (hash-market quorum checked).
    #[returns(cw_lc_registry::msg::ConsensusTipResponse)]
    ConsensusTip {},
    /// Raw zakura node state reports from the attesting operators.
    #[returns(cw_lc_registry::msg::NodeStatesResponse)]
    NodeStates {},
}
