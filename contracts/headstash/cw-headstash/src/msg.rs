use super::*;
use crate::bridge::{AssetStatus, BridgeCfg, BridgeMintClaimPublic, ReflectionSnapshot};
use crate::distro::DistroHashDomain;
use crate::egress::EgressBurnStatement;

#[cw_serde]
pub struct InstantiateMsg {
    /// Headstash genesis merkle tree root (32 bytes). Registered as `root_id = 0`.
    /// Product A: depth-32 Poseidon-v1 path root (not shallow suite root).
    pub genesis_root: Binary,
    /// Public inclusion hash domain. Defaults to `poseidon-v1` for new Headstashes.
    /// Use `sinsemilla-legacy` only for recovery of pre-Poseidon trees.
    #[serde(default)]
    pub distro_hash_domain: DistroHashDomain,
    /// Optional label for the genesis eligibility set.
    #[serde(default)]
    pub genesis_label: Option<String>,
    /// Token strategies for the tokens distributed to headstash users
    pub token_strategy: TokenStrategy,
    /// Aggregated list of keys
    pub wavs: WavsProofOfOwnership,
    /// Stored circuit id after zkvm `store-circuit` / `upload_circuit` (default 0 until set).
    #[serde(default)]
    pub circuit_id: Option<u64>,
}

#[cfg_attr(feature = "interface", derive(cw_orch::ExecuteFns))]
#[cw_serde]
pub enum ExecuteMsg {
    /// Verify claims and distribute. Each claim binds to a registered `root_id`.
    ProcessHeadstash {
        claims: Vec<HeadstashNote>,
    },
    /// Owner: bind wasmvm store-circuit id used by `proof_instance_verify`.
    SetCircuitId {
        circuit_id: u64,
    },
    /// Register an **additive** eligibility root (Poseidon-v1 only for new drops).
    /// Owner-only. Assigns the next `root_id` and stores root + domain.
    RegisterEligibilityRoot {
        /// 32-byte distro tree root.
        root: Binary,
        /// If omitted, uses the contract's default `distro_hash_domain` (must still
        /// pass additive policy — Poseidon-v1 for new roots).
        #[serde(default)]
        domain: Option<DistroHashDomain>,
        #[serde(default)]
        label: Option<String>,
    },
    Mint {
        to_address: String,
        amount: Uint128,
    },
    Burn {
        from_address: String,
        amount: Uint128,
    },

    // -------------------------------------------------------------------------
    // Private-bridge mint surface (cw-headstash is the mint router — CLARITY)
    // -------------------------------------------------------------------------

    /// Advance reflection / LC attested roots (owner/operator).
    UpdateReflection {
        pool_root: Binary,
        spent_root: Binary,
        burn_root: Binary,
        source_height: u64,
        tip_height: u64,
    },
    /// Replace full reflection snapshot (includes K, lag, frozen).
    SetReflectionSnapshot {
        snapshot: ReflectionSnapshot,
    },
    /// Register asset in **internal** registry (never mints balances).
    RegisterAsset {
        /// 32-byte Terp asset id (or mapped tacit id used as key).
        asset_id: Binary,
        /// Local denom / proof representation.
        local_denom: String,
        /// Optional origin tag (`native`, `tacit:…`, IBC trace).
        #[serde(default)]
        origin: Option<String>,
        #[serde(default)]
        status: Option<AssetStatus>,
    },
    /// Optional external asset registry contract address (hook; not required for demos).
    SetExternalAssetRegistry {
        addr: Option<String>,
    },
    /// Owner-set bridge corridor config (dest domain, K, lag, mock_verify).
    SetBridgeCfg {
        cfg: BridgeCfg,
    },
    /// One-shot bridge mint → SEAM-NOTE-OUT note result.
    /// Proof verify is **mock/stub** in Round 2 (`mock_verify` or cfg(test)).
    BridgeMintNote {
        claim: BridgeMintClaimPublic,
        /// Opaque membership / reflection receipt (mock or SP1 later).
        proof: Binary,
    },
    /// Option D private-bridge **egress burn** (ZEC SEAM → preauth dest).
    /// Dual-path: `BridgeCfg.mock_verify` / cfg(test) lab, or `zk-api` + `egress_zkid`.
    /// Spent under egress-nf-v0 domain (≠ pool-nf, ≠ bridge mint ν).
    BridgeEgressBurn {
        statement: EgressBurnStatement,
        /// Opaque membership / spend proof (mock or Halo2 later).
        proof: Binary,
    },

    /// Owner: fixed CHT amounts for the public Ethereum path. Index `i` pays `amounts[i]`.
    SetDenomTable {
        amounts: Vec<u64>,
    },
    /// Owner: these nullifiers are paid only by the public batch, not the Halo2 batch.
    AssignPublicNotes {
        nullifiers: Vec<Binary>,
    },
    /// Pay a Halo2 epoch batch whose aggregate is the stored `headstash-accepted` root.
    /// Does not verify the proof.
    ApplyAcceptedBatch {
        allocation_root: Binary,
        root_id: u64,
        rows: Vec<crate::epoch::EpochPayout>,
    },
    /// Pay opened Pedersen destinations for Ethereum logs of fixed denomination indexes.
    ApplyPublicBatch {
        rows: Vec<crate::epoch::PublicOpen>,
    },
}

#[cfg_attr(feature = "interface", derive(cw_orch::QueryFns))]
#[cw_ownable::cw_ownable_query]
#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    /// Check if a nullifier exists (raw key or domain-separated `{root_id}:{nf}`).
    #[returns(bool)]
    Nullifer { null: String },
    /// Retrieve all nullifiers
    #[returns(Vec<String>)]
    Nullifiers {
        start_after: Option<String>,
        limit: Option<u32>,
    },
    /// Default distro hash domain + next root id.
    #[returns(DistroConfigResponse)]
    DistroConfig {},
    /// Single eligibility root by id.
    #[returns(crate::distro::EligibilityRootEntry)]
    EligibilityRoot { root_id: u64 },
    /// List registered eligibility roots (additive set).
    #[returns(Vec<crate::distro::EligibilityRootEntry>)]
    EligibilityRoots {
        start_after: Option<u64>,
        limit: Option<u32>,
    },

    // --- Bridge ---
    /// Current reflection tip snapshot (if set).
    #[returns(Option<ReflectionSnapshot>)]
    ReflectionTip {},
    /// Whether a bridge-burn ν has already minted.
    #[returns(bool)]
    IsBridgeMinted { nullifier: Binary },
    /// Whether an egress-nf-v0 ν has already been spent (Option D burn).
    #[returns(bool)]
    IsEgressSpent { nullifier: Binary },
    /// Internal asset registry entry.
    #[returns(Option<crate::bridge::AssetEntry>)]
    BridgeAsset { asset_id: Binary },
    /// Bridge corridor config.
    #[returns(Option<BridgeCfg>)]
    BridgeConfig {},
    /// External asset registry address (if configured).
    #[returns(Option<Addr>)]
    ExternalAssetRegistry {},
    /// Hashmerchant root stored for `headstash-pending` or `headstash-accepted`.
    #[returns(crate::epoch::EpochRootResponse)]
    EpochRoot { algo: String },
}

#[cw_serde]
pub struct DistroConfigResponse {
    pub default_domain: DistroHashDomain,
    pub default_domain_tag: String,
    pub next_root_id: u64,
    pub genesis_root: Binary,
}
