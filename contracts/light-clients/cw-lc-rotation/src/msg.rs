use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::Binary;

use crate::committee::CommitteePolicy;

#[cw_serde]
pub struct InstantiateMsg {
    pub admin: Option<String>,
    /// Label with role `rotation`, e.g. `terp/lc/rotation/zcash-regtest/v1`.
    pub label: String,
    pub registry: String,
    pub client_id: String,
    pub policy: Option<CommitteePolicy>,
}

#[cw_serde]
pub enum ExecuteMsg {
    /// Admin, once: the aggregator allowed to report faults / queried for liveness.
    SetAggregator { aggregator: String },
    /// Admin: applies from the next proposed epoch.
    UpdatePolicy { policy: CommitteePolicy },
    /// Anyone, when no epoch is active or pending.
    Bootstrap {},
    /// Planned rotation / reshare (admin or an active member). The active
    /// epoch keeps serving until the new one activates with its hand-off.
    RotateShard { note: String },
    /// Anyone: faults the active epoch if the aggregator reports a stall.
    ReportLivenessFault {},
    /// Aggregator only: equivocation evidence was recorded.
    ReportFault { epoch: u64, note: String },
    /// Pending member: "I hold my shares of `group_key`".
    AckEpoch { epoch: u64, group_key: Binary },
    /// Anyone. `proof_sig`: new key over the activation digest. `handoff_sig`:
    /// active key over the same digest (required unless no epoch is active).
    ActivateEpoch {
        epoch: u64,
        group_key: Binary,
        proof_sig: Binary,
        handoff_sig: Option<Binary>,
    },
}

#[cw_serde]
pub struct MigrateMsg {}

/// Query understood by `cw-lc-aggregator` (kept here to avoid a crate cycle).
#[cw_serde]
pub enum AggregatorQuery {
    Liveness {},
}

#[cw_serde]
pub struct LivenessResponse {
    pub active_epoch: Option<u64>,
    pub last_preconf_block: Option<u64>,
    pub latest_height: Option<u64>,
    pub stalled: bool,
}

#[cw_serde]
pub struct AckView {
    pub operator: String,
    pub group_key: Binary,
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(crate::state::Config)]
    Config {},
    #[returns(crate::state::Epoch)]
    Epoch { epoch: u64 },
    #[returns(Option<crate::state::Epoch>)]
    ActiveEpoch {},
    #[returns(Option<crate::state::Epoch>)]
    PendingEpoch {},
    #[returns(Vec<AckView>)]
    Acks { epoch: u64 },
    /// Committee the current registry set would produce now.
    #[returns(crate::committee::Committee)]
    PreviewCommittee {},
    /// Digest `proof_sig` / `handoff_sig` must sign.
    #[returns(Binary)]
    ActivationDigest { epoch: u64, group_key: Binary },
}
