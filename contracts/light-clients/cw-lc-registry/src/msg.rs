use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::{Addr, Binary};

use crate::quorum::{HashMarketCapability, QuorumCheck};
use crate::state::{ClientRecord, OperatorInfo, StoredReport};

#[cw_serde]
pub struct InstantiateMsg {
    /// Defaults to the sender.
    pub admin: Option<String>,
    /// Must follow the labelling spec with role `registry`
    /// (e.g. `terp/lc/registry/zcash-regtest/v1`).
    pub label: String,
    /// hm-vext capability `(chain_uid, algo)` whose quorum root the tip is
    /// checked against, if any.
    pub hash_market_capability: Option<HashMarketCapability>,
    /// Reports older than this many blocks are ignored by `ConsensusTip`.
    pub max_report_age_blocks: u64,
}

#[cw_serde]
pub enum PowAlgo {
    /// Zcash Equihash (n=200, k=9 on mainnet).
    Equihash {
        n: u32,
        k: u32,
    },
    /// Bitcoin double SHA-256.
    Sha256d,
    Other(String),
}

#[cw_serde]
pub struct TargetChain {
    /// e.g. `zcash-main`, `zcash-regtest`, `zakura-crosslink`.
    pub chain_id: String,
    pub algo: PowAlgo,
    /// NU7 targets 20–25 s; used for freshness windows off-chain.
    pub target_block_time_secs: u32,
}

#[cw_serde]
pub enum ClientKind {
    /// 08-wasm light client on a real chain (bind its client id later).
    Ics08Wasm,
    /// Permissionless client: any party may register, attestors carry trust.
    Permissionless,
}

/// Internal zakura node state as an operator reports it.
#[cw_serde]
pub struct NodeState {
    pub tip_height: u64,
    /// 32-byte block hash, hex (display order).
    pub tip_hash: String,
    pub prev_hash: String,
    /// Cumulative chain work, big-endian hex (no 0x).
    pub chain_work: String,
    /// Crosslink / TFL finalized height, when the node runs finality.
    pub finalized_height: Option<u64>,
    pub peer_count: u32,
    /// Operator clock, unix seconds; must strictly increase per (client, operator).
    pub observed_at_unix: u64,
}

#[cw_serde]
pub enum ExecuteMsg {
    RegisterOperator {
        /// zakura p2pv2 peer id.
        peer_id: String,
        /// p2pv2 / RPC endpoint the service exposes (multiaddr or URL).
        endpoint: String,
        /// Ed25519 verifying key (32 bytes). A FROST-Ed25519 group key works too.
        pubkey: Binary,
        /// Capability tags, e.g. `pow-headers`, `node-state`, `preconf`.
        capabilities: Vec<String>,
    },
    UpdateOperator {
        endpoint: Option<String>,
        pubkey: Option<Binary>,
        capabilities: Option<Vec<String>>,
    },
    DeregisterOperator {},
    RegisterClient {
        client_id: String,
        kind: ClientKind,
        target: TargetChain,
        attestors: Vec<String>,
        min_attestations: u32,
        ibc_client_id: Option<String>,
    },
    /// Client owner only.
    SetAttestors {
        client_id: String,
        attestors: Vec<String>,
        min_attestations: u32,
    },
    /// Client owner only: bind the on-chain 08-wasm / IBC v2 client id.
    BindIbcClient {
        client_id: String,
        ibc_client_id: String,
    },
    /// Attestor only. `signature` is Ed25519 over [`crate::digest::report_digest`].
    ReportNodeState {
        client_id: String,
        state: NodeState,
        signature: Binary,
    },
    /// Admin only. Committee weight for an operator (0 removes it from the
    /// next committee epoch; it can still attest k-of-n).
    SetOperatorWeight {
        operator: String,
        weight: u64,
    },
    /// Admin only.
    UpdateConfig {
        admin: Option<String>,
        hash_market_capability: Option<HashMarketCapability>,
        max_report_age_blocks: Option<u64>,
    },
}

#[cw_serde]
pub struct MigrateMsg {}

#[cw_serde]
pub struct AgreedTip {
    pub height: u64,
    pub hash: String,
    pub finalized_height: Option<u64>,
    pub attestors: Vec<Addr>,
}

#[cw_serde]
pub struct ConsensusTipResponse {
    pub client_id: String,
    pub min_attestations: u32,
    pub fresh_reports: u32,
    /// `None` when no group reaches `min_attestations`, or the hash-market
    /// quorum check blocks it (see `quorum`).
    pub tip: Option<AgreedTip>,
    /// Present when a hash-market capability is configured.
    pub quorum: Option<QuorumCheck>,
}

#[cw_serde]
pub struct OperatorsResponse {
    pub operators: Vec<OperatorInfo>,
}

#[cw_serde]
pub struct ClientsResponse {
    pub clients: Vec<ClientRecord>,
}

#[cw_serde]
pub struct NodeStatesResponse {
    pub reports: Vec<StoredReport>,
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(crate::state::Config)]
    Config {},
    #[returns(OperatorInfo)]
    Operator { operator: String },
    #[returns(OperatorsResponse)]
    Operators {
        start_after: Option<String>,
        limit: Option<u32>,
    },
    #[returns(ClientRecord)]
    Client { client_id: String },
    #[returns(ClientsResponse)]
    Clients {
        start_after: Option<String>,
        limit: Option<u32>,
    },
    #[returns(Option<StoredReport>)]
    NodeState { client_id: String, operator: String },
    #[returns(NodeStatesResponse)]
    NodeStates { client_id: String },
    /// Highest (height, hash) reported by ≥ `min_attestations` fresh attestors.
    #[returns(ConsensusTipResponse)]
    ConsensusTip { client_id: String },
    /// Latest hash-market quorum root for the configured capability.
    #[returns(Option<crate::quorum::QuorumRoot>)]
    QuorumRoot {},
    /// Digest an operator must sign for a report (helper for off-chain services).
    #[returns(Binary)]
    ReportDigest {
        client_id: String,
        operator: String,
        state: NodeState,
    },
}
