use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::Binary;

/// DAO (or factory) registers this authenticator as a **voting module** for
/// private ballots following the Shielded Vote / vote-sdk (VSCK) workflow.
#[cw_serde]
pub struct InstantiateMsg {
    pub admin: Option<String>,
    /// DAO core / voting module address that may open sessions.
    pub dao: String,
    /// Verifying keys for vote-sdk circuit families (opaque until circuits linked).
    pub circuit_vks: CircuitVerifyingKeys,
}

/// Maps to vote-sdk circuit crates: delegation, vote_proof, share_reveal.
#[cw_serde]
pub struct CircuitVerifyingKeys {
    pub delegation_vk: Binary,
    pub vote_proof_vk: Binary,
    pub share_reveal_vk: Binary,
}

#[cw_serde]
pub enum ExecuteMsg {
    /// Open a private voting session (ceremony snapshot id, merkle root, etc.).
    OpenSession {
        session_id: String,
        /// Note commitment tree root / IMT root from vote-sdk.
        note_tree_root: Binary,
        /// Optional metadata (proposal id on parent DAO).
        proposal_ref: Option<String>,
    },
    CloseSession {
        session_id: String,
    },
    UpdateCircuitVks(CircuitVerifyingKeys),
    UpdateAdmin {
        admin: String,
    },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(ConfigResponse)]
    Config {},
    #[returns(SessionResponse)]
    Session { session_id: String },
}

#[cw_serde]
pub struct ConfigResponse {
    pub admin: String,
    pub dao: String,
}

#[cw_serde]
pub struct SessionResponse {
    pub session_id: String,
    pub note_tree_root: Binary,
    pub proposal_ref: Option<String>,
    pub open: bool,
}

/// Roles in the VSCK / vote-sdk workflow (mirrors private voting pipeline).
#[cw_serde]
pub enum VsckRole {
    /// Registers delegation note / voting power (ZKP #1 delegation circuit).
    Delegator,
    /// Casts private ballot (vote_proof circuit).
    Voter,
    /// Ceremony / tally helper share reveal (share_reveal circuit).
    Tallier,
    /// DAO module operator (open/close session via execute; not sudo auth).
    Coordinator,
}

/// Placed in `AuthenticationRequest.signature`.
#[cw_serde]
pub struct VsckAuthPayload {
    pub session_id: String,
    pub role: VsckRole,
    /// Circuit family selector.
    pub circuit: VsckCircuit,
    pub public_inputs: Binary,
    pub proof: Binary,
    /// Vote nullifier / note nullifier for replay protection.
    pub nullifier: Binary,
}

#[cw_serde]
pub enum VsckCircuit {
    Delegation,
    VoteProof,
    ShareReveal,
}
