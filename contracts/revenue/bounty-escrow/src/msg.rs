use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::{Coin, Timestamp, Uint256};

#[allow(unused_imports)]
use crate::state::{Bounty, BountyStatus, Config, CreationGate, DenomRule, Submission};
#[allow(unused_imports)]
use cosmwasm_std::Addr;
pub use dao_voting::pre_propose::{PreProposeInfo, ProposalCreationPolicy};

#[cw_serde]
pub struct ProposerGateMsg {
    pub use_allowlist: bool,
    pub min_stake: Option<Uint256>,
    pub cw4_group: Option<String>,
}

#[cw_serde]
pub struct InstantiateMsg {
    pub owner: String,
    pub approver: String,
    pub gate: ProposerGateMsg,
    pub denoms: Vec<DenomRule>,
    pub allow_partial_payouts: bool,
    pub fee_bps: u16,
    pub fee_collector: Option<String>,
    pub min_duration_secs: u64,
    pub max_duration_secs: u64,
    /// Required label of this parameter set; no default.
    pub params_version: String,
    /// Required: true while the values are unvoted placeholders.
    pub params_placeholder: bool,
    /// Required: minimum bonded stake (bond denom) of the assignee to submit work; "0" disables. No default.
    pub min_submit_stake: Uint256,
    /// Required: minimum bonded stake (bond denom) to upload documents through the CID storage service
    /// (checked off chain via `can_upload`); "0" disables. No default.
    pub min_upload_stake: Uint256,
    #[serde(default)]
    pub initial_allowlist: Vec<String>,
    /// DAO DAO core address. Default: the instantiator (a DAO core instantiates its proposal modules,
    /// exactly like dao-proposal-single, which uses `info.sender`).
    #[serde(default)]
    pub dao: Option<String>,
    /// dao-voting pre-propose config. Default `AnyoneMayPropose {}` (then `creation_gate` applies).
    /// `ModuleMayPropose { info }` instantiates the module (admin = dao) and gives it exclusive
    /// creation rights once its instantiate reply arrives (fails open to Anyone until then, as upstream).
    #[serde(default)]
    pub pre_propose_info: Option<PreProposeInfo>,
    /// Creation gate while no pre-propose module is installed. Default `ProposerGate {}` (legacy).
    #[serde(default)]
    pub creation_gate: Option<CreationGate>,
}

#[cw_serde]
#[cfg_attr(feature = "interface", derive(cw_orch::ExecuteFns))]
pub enum ExecuteMsg {
    /// Whitelisted proposer only. `info.funds` must be exactly `[reward]`. Creates a Pending bounty.
    /// `summary` <= 280 bytes UTF-8 (no control chars); `details_hash` = sha256 hex of the canonical
    /// terp-bounty-details/v1 doc; `details_cid` = optional IPFS CID of it.
    #[cfg_attr(feature = "interface", cw_orch(payable))]
    CreateBounty {
        bounty_ref: String,
        title: String,
        summary: String,
        details_hash: String,
        details_cid: Option<String>,
        reward: Coin,
        expires_at: Timestamp,
        /// Must be set if and only if the sender is the installed pre-propose module (it then names the
        /// real proposer, who receives refunds). Same rule as dao-proposal-single `propose.proposer`.
        #[serde(default)]
        proposer: Option<String>,
    },
    /// Approver: Pending -> Open.
    Approve { id: u64 },
    /// Approver: Pending -> Refunded (full refund to proposer).
    Reject { id: u64, reason: Option<String> },
    /// Proposer (Pending/Open) or approver (Pending/Open/Assigned) -> Cancelled (full refund).
    Cancel { id: u64 },
    /// Proposer or approver: Open -> Assigned.
    Assign { id: u64, assignee: String },
    /// Proposer or approver: Assigned -> Open.
    Unassign { id: u64 },
    /// Assignee only, before expiry, with bonded stake >= min_submit_stake: Assigned -> Submitted.
    /// `cid` = IPFS CID of the terp-bounty-submission/v1 doc; `submission_hash` = sha256 hex of its bytes.
    Submit {
        id: u64,
        submission_hash: String,
        cid: String,
    },
    /// Proposer or approver: Submitted -> Assigned.
    RequestChanges { id: u64, reason: Option<String> },
    /// Approver: Submitted -> Accepted.
    Accept { id: u64 },
    /// Approver: release `amount` (default: everything remaining) to the assignee.
    /// Accepted -> Paid when nothing remains; partial amounts need allow_partial_payouts.
    Payout { id: u64, amount: Option<Uint256> },
    /// Approver: after a partial payout, refund what is left to the proposer. Accepted -> Paid.
    SettleRemainder { id: u64 },
    /// Anyone, once block time >= expires_at: Pending/Open/Assigned -> Expired (full refund).
    Expire { id: u64 },
    /// Owner only (governance). Changing any governance parameter (approver, gate, denoms/caps,
    /// partial payouts, fee, fee_collector, duration window) requires a new `params_version`.
    UpdateConfig {
        owner: Option<String>,
        approver: Option<String>,
        gate: Option<ProposerGateMsg>,
        denoms: Option<Vec<DenomRule>>,
        allow_partial_payouts: Option<bool>,
        fee_bps: Option<u16>,
        fee_collector: Option<String>,
        min_duration_secs: Option<u64>,
        max_duration_secs: Option<u64>,
        params_version: Option<String>,
        params_placeholder: Option<bool>,
        min_submit_stake: Option<Uint256>,
        min_upload_stake: Option<Uint256>,
        #[serde(default)]
        creation_gate: Option<CreationGate>,
    },
    /// Owner only.
    UpdateAllowlist {
        add: Vec<String>,
        remove: Vec<String>,
    },
    /// DAO only (dao-proposal-single shape): replace the pre-propose module config.
    UpdatePreProposeInfo { info: PreProposeInfo },
    /// DAO only: register a dao-hooks proposal hook receiver.
    AddProposalHook { address: String },
    /// DAO only: remove a proposal hook receiver.
    RemoveProposalHook { address: String },
}

#[cw_serde]
#[derive(QueryResponses)]
#[cfg_attr(feature = "interface", derive(cw_orch::QueryFns))]
pub enum QueryMsg {
    #[returns(Config)]
    Config {},
    #[returns(Bounty)]
    Bounty { id: u64 },
    #[returns(Bounty)]
    BountyByRef { bounty_ref: String },
    #[returns(BountiesResponse)]
    Bounties {
        status: Option<BountyStatus>,
        proposer: Option<String>,
        start_after: Option<u64>,
        limit: Option<u32>,
    },
    #[returns(SubmissionsResponse)]
    Submissions {
        id: u64,
        start_after: Option<u32>,
        limit: Option<u32>,
    },
    #[returns(EligibilityResponse)]
    IsEligible { address: String },
    /// Whether `address` passes the submission stake gate (min_submit_stake).
    #[returns(SubmitEligibilityResponse)]
    CanSubmit { address: String },
    /// Whether `address` passes the upload stake gate (min_upload_stake) of the CID storage service.
    #[returns(UploadEligibilityResponse)]
    CanUpload { address: String },
    #[returns(AllowlistResponse)]
    Allowlist {
        start_after: Option<String>,
        limit: Option<u32>,
    },

    // ---- DAO DAO proposal-module queries (dao-proposal-single shapes) ----
    /// cw2 contract name and version.
    #[returns(dao_interface::voting::InfoResponse)]
    Info {},
    /// The DAO core this module belongs to.
    #[returns(Addr)]
    Dao {},
    /// A bounty as a proposal.
    #[returns(BountyProposalResponse)]
    Proposal { proposal_id: u64 },
    /// Bounties as proposals, ascending by id.
    #[returns(BountyProposalListResponse)]
    ListProposals {
        start_after: Option<u64>,
        limit: Option<u64>,
    },
    /// Bounties as proposals, descending by id.
    #[returns(BountyProposalListResponse)]
    ReverseProposals {
        start_before: Option<u64>,
        limit: Option<u64>,
    },
    /// Number of bounties ever created.
    #[returns(u64)]
    ProposalCount {},
    /// Id the next bounty will get.
    #[returns(u64)]
    NextProposalId {},
    /// Who may create bounties (Anyone or a pre-propose module).
    #[returns(ProposalCreationPolicy)]
    ProposalCreationPolicy {},
    /// Registered proposal hook receivers.
    #[returns(::cw_hooks::HooksResponse)]
    ProposalHooks {},
}

/// Mirrors dao-proposal-single `ProposalResponse { id, proposal }`.
#[cw_serde]
pub struct BountyProposalResponse {
    pub id: u64,
    pub proposal: Bounty,
}

/// Mirrors dao-proposal-single `ProposalListResponse { proposals }`.
#[cw_serde]
pub struct BountyProposalListResponse {
    pub proposals: Vec<BountyProposalResponse>,
}

#[cw_serde]
pub struct BountiesResponse {
    pub bounties: Vec<Bounty>,
}

#[cw_serde]
pub struct SubmissionsResponse {
    pub submissions: Vec<Submission>,
}

#[cw_serde]
pub struct EligibilityResponse {
    pub eligible: bool,
    /// "allowlist" | "min_stake" | "cw4" when eligible.
    pub via: Option<String>,
    /// Bonded-denom delegation total (only computed when min_stake is enabled).
    pub staked: Option<Uint256>,
}

#[cw_serde]
pub struct SubmitEligibilityResponse {
    pub eligible: bool,
    /// Bonded-denom delegation total of `address`.
    pub staked: Uint256,
    pub min_submit_stake: Uint256,
    pub bond_denom: String,
}

#[cw_serde]
pub struct UploadEligibilityResponse {
    pub eligible: bool,
    /// Bonded-denom delegation total of `address`.
    pub staked: Uint256,
    pub min_upload_stake: Uint256,
    pub bond_denom: String,
}

#[cw_serde]
pub struct AllowlistResponse {
    pub addresses: Vec<String>,
}

#[cw_serde]
pub struct MigrateMsg {}
