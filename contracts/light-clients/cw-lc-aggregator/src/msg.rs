use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::Binary;

use crate::state::PreconfStatus;

#[cw_serde]
pub struct InstantiateMsg {
    /// Label with role `aggregator`, e.g. `terp/lc/aggregator/zcash-regtest/v1`.
    pub label: String,
    pub registry: String,
    pub rotation: String,
    pub client_id: String,
    pub confirm_depth: u64,
    pub stall_blocks: u64,
    pub max_confirm_walk: Option<u32>,
}

/// Committed header fields (hex, display order like the registry).
#[cw_serde]
pub struct Header {
    pub height: u64,
    pub hash: String,
    pub prev_hash: String,
    pub chain_work: String,
    /// Optional zap-1 Merkle root (hex32) of payment events the committee's
    /// zakura nodes observed in this block. `cw-lc-zcash` checks payment
    /// inclusion proofs against it (atomic settlement).
    #[serde(default)]
    pub payment_root: Option<String>,
}

#[cw_serde]
pub enum ExecuteMsg {
    /// Anyone relays; `signature` is the epoch committee over [`QueryMsg::PreconfDigest`].
    SubmitPreconf {
        epoch: u64,
        header: Header,
        signature: Binary,
    },
    /// Anyone: confirm `height` against the registry k-of-n tip.
    Confirm { height: u64 },
}

#[cw_serde]
pub struct MigrateMsg {}

#[cw_serde]
pub enum HeaderSource {
    /// Committee preconfirmation from the active epoch.
    Preconf,
    /// Registry k-of-n `ConsensusTip` (no active / stalled committee).
    RegistryFallback,
    None,
}

#[cw_serde]
pub struct BestHeaderResponse {
    pub source: HeaderSource,
    pub height: Option<u64>,
    pub hash: Option<String>,
    pub epoch: Option<u64>,
    pub status: Option<PreconfStatus>,
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(crate::state::Config)]
    Config {},
    #[returns(Option<crate::state::PreconfRecord>)]
    Preconf { height: u64 },
    #[returns(BestHeaderResponse)]
    BestHeader {},
    #[returns(cw_lc_rotation::msg::LivenessResponse)]
    Liveness {},
    #[returns(Option<crate::state::Equivocation>)]
    Evidence { epoch: u64 },
    #[returns(Binary)]
    PreconfDigest { epoch: u64, header: Header },
}
