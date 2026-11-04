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
    /// Optional WAVS operator set (aggregated BLS keys + PoPs). None = WAVS disabled.
    /// There is no execute path to set it later; only a gov migrate to new code can.
    #[serde(default)]
    pub wavs: Option<WavsProofOfOwnership>,
    /// Stored circuit id after zkvm `store-circuit` / `upload_circuit` (default 0 until set).
    #[serde(default)]
    pub circuit_id: Option<u64>,
    /// Optional contract executed (no funds, gas-capped, errors swallowed) once per
    /// identity when its note list completes. Immutable after instantiate.
    #[serde(default)]
    pub registration_hook: Option<String>,
    /// Gas cap for the hook submessage (default 500,000; max 5,000,000).
    #[serde(default)]
    pub registration_hook_gas_limit: Option<u64>,
    /// Optional cw-headstash-frontend contract hosting the gov-approved claim UI
    /// (gzip chunks + manifest). Discoverable via `QueryMsg::FrontendContract`.
    #[serde(default)]
    pub frontend_contract: Option<String>,
    /// Optional pauser (Appendix P): may Pause/Resume claims and registration only.
    /// None = no pauser (gov, as owner, can still pause). Gov changes it with UpdatePauser.
    #[serde(default)]
    pub pauser: Option<String>,
}

/// One-shot admin migrate. The new code copies a still-zero genesis root
/// from the campaign allocation root when that root is a registration anchor.
#[cw_serde]
pub struct MigrateMsg {}

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
    /// Owner (x/gov): set or clear the on-chain frontend contract address.
    SetFrontendContract {
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
    /// Owner: epoch creator parameters. Campaigns that never set them keep the store-only sudo.
    SetEpochCampaign {
        campaign: crate::epoch::EpochCampaignParams,
    },
    /// Owner: send leftover escrow of the campaign denom to `sweep_recipient`.
    /// The block height must be past the cutoff. This message carries no address.
    SweepUnclaimed {},
    /// Gov only (sender must be the cw-ownable owner AND the wasm admin, both x/gov).
    /// Allowed only while height <= cutoff (or before any campaign is set). Sends every
    /// remaining escrowed coin to the community pool (MsgFundCommunityPool, depositor =
    /// this contract) and permanently halts the contract. After the cutoff use
    /// `SweepUnclaimed` (burn). This message carries no address.
    EmergencyReclaim {},
    /// Pauser or owner: halt claims, registration, sudo and the claim/register dry-run
    /// queries. Errors "already paused" if paused. Does not extend the cutoff.
    Pause {},
    /// Pauser or owner: lift a pause. Errors "not paused" if live.
    Resume {},
    /// Owner (gov) only: set, rotate or clear (None) the pauser. Does not resume.
    UpdatePauser { pauser: Option<String> },
    /// Owner: two-step ownership transfer (propose, then the new owner accepts).
    /// `renounce_ownership` is rejected so the campaign can never be orphaned.
    UpdateOwnership(cw_ownable::Action),
    /// Owner: the rows and proof digests an accepted sudo can pay.
    StageEpochBatch {
        digests: Vec<Binary>,
        rows: Vec<crate::epoch::EpochPayout>,
    },
    /// Pay a Halo2 epoch batch whose aggregate is the stored `headstash-accepted` root.
    /// Does not verify the proof.
    ApplyAcceptedBatch {
        allocation_root: Binary,
        root_id: u64,
        rows: Vec<crate::epoch::EpochPayout>,
        /// SHA-256 of the circuit blob the sidecar verified under.
        /// Empty fails closed when the stored campaign hash is not all zeros.
        #[serde(default)]
        vk_hash: Binary,
    },
    /// Pay opened Pedersen destinations for Ethereum logs of fixed denomination indexes.
    ApplyPublicBatch {
        rows: Vec<crate::epoch::PublicOpen>,
    },
    /// Anyone may relay a registration. The eligible address is the recovered signer.
    RegisterHeadstash {
        key_com: Binary,
        amount: u64,
        signature: Binary,
        eligibility_index: u32,
        eligibility_path: Vec<Binary>,
    },
    /// Anyone may relay a Solana registration. `pubkey` (32 bytes) is the eligible
    /// identity; `signature` (64 bytes) is ed25519 over the raw UTF-8 of
    /// `headstash-register-sol-v1:{ctx_hex}:{key_com_hex}:{amount}`.
    RegisterHeadstashSol {
        key_com: Binary,
        amount: u64,
        pubkey: Binary,
        signature: Binary,
        eligibility_index: u32,
        eligibility_path: Vec<Binary>,
    },
    /// Spec rev 5 note registration (vk_id 3 campaigns only). Anyone may relay.
    /// `list` is the identity's full ordered note list on every chunk; `signature`
    /// (required on the first chunk) signs its hash. `eligibility` holds one
    /// depth-17 path per note in `start..start + eligibility.len()` (at most 32).
    RegisterNotes {
        signer: crate::notes::NoteSigner,
        #[serde(default)]
        signature: Option<Binary>,
        list: Vec<crate::notes::NoteListEntry>,
        start: u32,
        eligibility: Vec<crate::notes::NoteEligibility>,
        /// Terp bech32 address bound by the v3 signature (first chunk); later chunks
        /// may omit it or must repeat the same value.
        #[serde(default)]
        reward_addr: Option<String>,
    },
    /// Permissionless. Stores the current registration root when the window is open.
    AcceptAnchor {},
    /// Owner, once: pin the wasmvm circuit id for direct note claims. The stored
    /// circuit must hash to the campaign vk_hash and campaign.ctx must equal
    /// `direct_ctx(chain_id, contract)`.
    BindNoteCircuit { zk_id: u64 },
    /// Anyone: direct vk_id 3 claim, paid to `info.sender` (bound in the proof).
    /// `root` must be a closed-epoch anchor.
    ClaimNote {
        root: Binary,
        nullifier: Binary,
        amount: u64,
        proof: Binary,
    },
    /// Anyone: direct vk_id 3 claim forwarded over an allowlisted ICS-20 channel.
    /// The proof destination is `forward_destination(chain, contract, forward)`
    /// (`hsfwd1…`), so channel, receiver, memo and fallback are all proof-bound.
    /// Address reuse is not checked on-chain (frontend rule). Refunds on
    /// timeout / error ack go to `forward.fallback` via the ICS-20 source callback.
    ClaimNoteForward {
        root: Binary,
        nullifier: Binary,
        amount: u64,
        proof: Binary,
        forward: crate::forward::ForwardTarget,
    },
    /// Owner: add / update / disable an ICS-20 channel for forwards.
    SetForwardChannel { channel_id: String, timeout_seconds: u64, enabled: bool },
    /// Owner: refund a forward whose callback never arrived, to its bound fallback
    /// only, after timeout + 1 day.
    ResolveForward { channel_id: String, sequence: u64 },
    /// Owner: where the UI fetches the eligible set (HAL) and its integrity commitment.
    SetHalSource { source: Option<crate::forward::HalSource> },
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
    /// WAVS status: `enabled` false when instantiated with `wavs: None`.
    #[returns(WavsStatusResponse)]
    WavsStatus {},
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
    /// cw-headstash-frontend address hosting the claim UI (if configured).
    #[returns(Option<Addr>)]
    FrontendContract {},
    /// Hashmerchant root stored for `headstash-pending` or `headstash-accepted`.
    #[returns(crate::epoch::EpochRootResponse)]
    EpochRoot { algo: String },
    /// Creator campaign, including the sidecar `vk_id` and blob hash.
    #[returns(crate::epoch::EpochCampaignParams)]
    EpochCampaign {},
    /// Registration frontier, anonymity minimum, anchor count and latest anchor.
    #[returns(crate::registration::RegistrationTreeResponse)]
    RegistrationTree {},
    /// Accepted anchors in acceptance order (`start_after` = last seen `seq`,
    /// exclusive; `limit` default 30, max 100).
    #[returns(crate::registration::RegistrationAnchorsResponse)]
    RegistrationAnchors {
        #[serde(default)]
        start_after: Option<u32>,
        #[serde(default)]
        limit: Option<u32>,
    },
    /// Sixteen little-endian siblings for `key_com` under one accepted root.
    #[returns(crate::registration::RegistrationPathResponse)]
    RegistrationPath { key_com: Binary, anchor: Binary },
    /// Whether an eligible identity already registered. `identity` is the
    /// 40-char lowercase hex EVM address, or `sol:` + 64-char hex pubkey.
    #[returns(bool)]
    Registered { identity: String },
    /// Signed note list state for an identity key (`<evm hex>` or `sol:<hex>`).
    #[returns(Option<crate::notes::NoteListState>)]
    NoteList { identity: String },
    /// Direct-claim binding, the ctx this contract expects, and the paid count.
    #[returns(crate::direct::DirectClaimInfo)]
    DirectClaim {},
    /// `Some` once governance has run `EmergencyReclaim` (terminal).
    #[returns(Option<crate::reclaim::ReclaimRecord>)]
    ReclaimStatus {},
    /// Pauser address and whether claims are paused (Appendix P).
    #[returns(crate::pause::PauseStatusResponse)]
    PauseStatus {},
    /// Dry-run of `ClaimNote` for `dest` (the fresh wallet). Errors with the same
    /// reason the execute would; `true` when the claim would pay.
    #[returns(bool)]
    CheckRegisterNotes {
        signer: crate::notes::NoteSigner,
        #[serde(default)]
        signature: Option<Binary>,
        list: Vec<crate::notes::NoteListEntry>,
        start: u32,
        eligibility: Vec<crate::notes::NoteEligibility>,
        /// Terp bech32 address bound by the v3 signature (first chunk); later chunks
        /// may omit it or must repeat the same value.
        #[serde(default)]
        reward_addr: Option<String>,
    },
    /// Allowlisted forward channels.
    #[returns(Vec<crate::forward::ForwardChannel>)]
    ForwardChannels {},
    /// HAL (eligible set) URI and integrity commitment, if set.
    #[returns(Option<crate::forward::HalSource>)]
    HalSource {},
    #[returns(Option<crate::forward::PendingForward>)]
    PendingForward { channel_id: String, sequence: u64 },
    #[returns(Vec<crate::forward::PendingForward>)]
    PendingForwards {
        #[serde(default)]
        start_after: Option<(String, u64)>,
        #[serde(default)]
        limit: Option<u32>,
    },
    /// The proof destination string (`hsfwd1…`) for a forward target.
    #[returns(String)]
    ForwardDestination { forward: crate::forward::ForwardTarget },
    /// Dry-run of `ClaimNoteForward`.
    #[returns(bool)]
    CheckNoteForward {
        root: Binary,
        nullifier: Binary,
        amount: u64,
        proof: Binary,
        forward: crate::forward::ForwardTarget,
    },
    /// Dry-run of `ClaimNote` for `dest` (the fresh wallet).
    #[returns(bool)]
    CheckNoteClaim {
        dest: String,
        root: Binary,
        nullifier: Binary,
        amount: u64,
        proof: Binary,
    },
}

#[cw_serde]
pub struct WavsStatusResponse {
    pub enabled: bool,
    pub operator_set: Option<crate::wavs::WavsOperatorSet>,
}

#[cosmwasm_schema::cw_serde]
pub struct DistroConfigResponse {
    pub default_domain: DistroHashDomain,
    pub default_domain_tag: String,
    pub next_root_id: u64,
    pub genesis_root: Binary,
}
