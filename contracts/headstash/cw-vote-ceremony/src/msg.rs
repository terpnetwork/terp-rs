use cosmwasm_schema::{QueryResponses, cw_serde};

use cosmwasm_std::{Binary, CosmosMsg, Empty, HexBinary, Uint128};

pub use crate::zk::{
    AnchorRef, InfoResponse, TallyAttestation, TallyAuth, ZkBallot, ZkConfig, ZkModuleInit,
    ZkProposal, ZkStatus, ZkTally,
};

/// Ceremony lifecycle status (string-stable for raw-query consumers).
#[cw_serde]
pub enum CeremonyStatus {
    Active,
    Closed,
}

impl CeremonyStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CeremonyStatus::Active => "Active",
            CeremonyStatus::Closed => "Closed",
        }
    }
}

#[cw_serde]
pub struct InstantiateMsg {
    /// Optional admin (test / bootstrap). DAO path should call via proposal execute.
    pub admin: Option<String>,
    /// Default max nullifiers per MarkSpent call.
    #[serde(default = "default_max_nf")]
    pub max_nullifiers_per_mark: u32,
    /// Enable the zk proposal module (DAO DAO proposal-module mode).
    #[serde(default)]
    pub zk_proposals: Option<ZkModuleInit>,
}

fn default_max_nf() -> u32 {
    8
}

#[cw_serde]
pub enum ExecuteMsg {
    /// Start a ceremony (admin or future DAO proposal path).
    StartCeremony {
        session_id: String,
        domain: String,
        /// When false, Register is rejected.
        #[serde(default = "default_true")]
        registration_open: bool,
        /// Optional anchor policy / root hint (opaque product string).
        #[serde(default)]
        anchor_policy: Option<String>,
        #[serde(default = "default_gate")]
        registration_gate: RegistrationGate,
        /// Clerk address allowed to AttestLeaf (defaults to admin).
        #[serde(default)]
        clerk: Option<String>,
        /// Voting / membership contract for DaoMember gate.
        #[serde(default)]
        membership_module: Option<String>,
        /// Verifier / dummy-SA stand-in for VmProof (VerifyMinHolders).
        #[serde(default)]
        vm_verifier: Option<String>,
    },
    /// Close ceremony (no further spends / registrations).
    CloseCeremony { session_id: String },
    /// Participant self-register. `leaf_commit` required when gate is ClerkAttested.
    Register {
        session_id: String,
        #[serde(default)]
        leaf_commit: Option<String>,
        #[serde(default)]
        proof: Option<Binary>,
    },
    /// Clerk/admin attests a registration leaf (dual L-reg half).
    AttestLeaf {
        session_id: String,
        leaf_commit: String,
    },
    /// Admin can toggle registration window.
    SetRegistrationOpen { session_id: String, open: bool },

    // ---- zk proposal module ----
    /// Any member with voting power > 0. Opens round `zkp-<id>`.
    ZkPropose {
        title: String,
        description: String,
        msgs: Vec<CosmosMsg<Empty>>,
        /// Election authority key, 64 bytes (x || y, LE Pallas Fp).
        ea_pk: HexBinary,
        anchor: AnchorRef,
    },
    /// Anonymous ballot: vote-sdk ZKP #2 proof + 11 x 32-byte public inputs. Any sender.
    ZkCastVote {
        proposal_id: u64,
        proof: Binary,
        instances: Binary,
        /// Opaque vote-share ciphertext for the election authority.
        ballot: Binary,
        /// RedPallas SpendAuth signature by r_vpk over `ZkCastSighash`.
        rvpk_sig: Option<HexBinary>,
    },
    /// After expiration: totals attested by the election authority (or a tally proof).
    ZkSubmitTally {
        proposal_id: u64,
        yes: Uint128,
        no: Uint128,
        abstain: Uint128,
        attestation: TallyAttestation,
    },
    /// Passed proposals call dao-core `execute_proposal_hook`. Anyone may trigger.
    ZkExecute { proposal_id: u64 },
}

fn default_true() -> bool {
    true
}

/// How Register is gated for this session (configurable product policy).
#[cw_serde]
#[derive(Default)]
pub enum RegistrationGate {
    /// Anyone with gas while registration_open (legacy).
    #[default]
    Open,
    /// Register requires a clerk-attested leaf_commit (dregg / event-reg clerk).
    ClerkAttested,
    /// Register requires non-zero voting power on membership_module.
    DaoMember,
    /// Hook site for pre-propose; not fully wired (rejects until implemented).
    PrePropose,
    /// Dummy SA / wasmvm Authenticate stand-in (`vm_verifier` query VerifyMinHolders).
    VmProof,
}

fn default_gate() -> RegistrationGate {
    RegistrationGate::Open
}

/// Privileged path: thin gate ConfirmExecution → chain sudo → MarkSpent.
/// Also usable from privileged execute if product wires permissioned keeper.
#[cw_serde]
pub enum SudoMsg {
    MarkSpent {
        domain: String,
        session_id: String,
        nullifiers: Vec<Binary>,
    },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(ConfigResponse)]
    Config {},

    #[returns(CeremonyResponse)]
    Ceremony { session_id: String },

    #[returns(IsSpentResponse)]
    IsSpent {
        domain: String,
        session_id: String,
        nullifier: Binary,
    },

    #[returns(IsRegisteredResponse)]
    IsRegistered { session_id: String, addr: String },

    /// Documented raw key bytes for ante raw-query consumers (V2).
    #[returns(RawSpentKeyResponse)]
    RawSpentKey {
        domain: String,
        session_id: String,
        nullifier: Binary,
    },

    // ---- zk proposal module ----
    #[returns(ZkConfig)]
    ZkConfig {},
    #[returns(ZkProposal)]
    ZkProposal { proposal_id: u64 },
    #[returns(Vec<ZkProposal>)]
    ZkListProposals {
        start_after: Option<u64>,
        limit: Option<u32>,
    },
    #[returns(Vec<ZkBallot>)]
    ZkBallots {
        proposal_id: u64,
        start_after: Option<u64>,
        limit: Option<u32>,
    },
    /// Message the voter's r_vpk signs for `ZkCastVote`.
    #[returns(HexBinary)]
    ZkCastSighash {
        proposal_id: u64,
        nullifier: HexBinary,
        vote_commitment: HexBinary,
        ballot: Binary,
    },
    /// Message the election authority group key signs for `ZkSubmitTally`.
    #[returns(HexBinary)]
    ZkTallyDigest {
        proposal_id: u64,
        yes: Uint128,
        no: Uint128,
        abstain: Uint128,
    },
    /// DAO DAO proposal-module surface.
    #[returns(cosmwasm_std::Addr)]
    Dao {},
    #[returns(InfoResponse)]
    Info {},
    #[returns(u64)]
    ProposalCount {},
    #[returns(u64)]
    NextProposalId {},
}

#[cw_serde]
pub struct ConfigResponse {
    pub admin: Option<String>,
    pub max_nullifiers_per_mark: u32,
}

#[cw_serde]
pub struct CeremonyInfo {
    pub session_id: String,
    pub domain: String,
    pub status: CeremonyStatus,
    pub registration_open: bool,
    pub anchor_policy: Option<String>,
    #[serde(default)]
    pub registration_gate: RegistrationGate,
    #[serde(default)]
    pub clerk: Option<String>,
    #[serde(default)]
    pub membership_module: Option<String>,
    #[serde(default)]
    pub vm_verifier: Option<String>,
}

#[cw_serde]
pub struct CeremonyResponse {
    pub ceremony: CeremonyInfo,
}

#[cw_serde]
pub struct IsSpentResponse {
    pub spent: bool,
}

#[cw_serde]
pub struct IsRegisteredResponse {
    pub registered: bool,
    /// Bound leaf when Register stored one (ClerkAttested). Absent/empty on Open.
    #[serde(default)]
    pub leaf_commit: Option<String>,
}

#[cw_serde]
pub struct RawSpentKeyResponse {
    /// Full storage key as used by cw-storage-plus Map under prefix `spent`
    /// for debugging; normative layout is also in `raw_keys` module / README.
    pub map_namespace: String,
    pub domain: String,
    pub session_id: String,
    pub nullifier: Binary,
}
