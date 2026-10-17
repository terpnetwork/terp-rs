//! Private-bridge mint surface on `cw-headstash` (Round 2).
//!
//! SSOT: `docs/plans/spectrum/CLARITY-cw-headstash-router-and-asset-registry.md`
//! Pure gates mirror `docs/plans/spectrum/fixtures/bridge_auth_seams` (H-1, A9, A13–A17).
//!
//! ## ZK / LC verify posture (Round 2)
//!
//! - **Mock / stub verify:** [`mock_verify_bridge_proof`] accepts when
//!   `cfg!(test)` **or** `BridgeCfg.mock_verify` is true, and claim flags assert
//!   burn-set membership. Real SP1 / LC membership verify is **not** wired this
//!   round (no guest, no anvil).
//! - **Oracle / external registry never mint balances** — registry only resolves
//!   asset_id / denom; mint authority is burn-set + pure gates.
//!
//! Nullifier domain for bridge claims: `bridge:0x02:{hex(ν)}` (SEAM `NF_BRIDGE_BURN`).

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{
    Addr, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdError, StdResult, Storage,
    to_json_binary,
};
use cw_storage_plus::{Item, Map};
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Constants (aligned with bridge_auth_seams / SEAM-NOTE-OUT)
// ---------------------------------------------------------------------------

pub const TERP_ASSET_DOMAIN_TAG: &[u8] = b"terp-tacit-asset-v1";
pub const TERP_PRIVATE_BRIDGE_DOMAIN_TAG: &[u8] = b"terp-private-bridge-v1";
pub const TERP_BRIDGE_CLAIM_TAG: &[u8] = b"terp-bridge-claim-v1";

pub const ORIGIN_BRIDGE_MINT: u8 = 0x02;
pub const NF_BRIDGE_BURN: u8 = 0x02;
pub const CM_ABSTRACT_LEAF_V0: u8 = 0x03;

pub const DEFAULT_CONFIRMATIONS_K: u64 = 6;
pub const DEFAULT_MAX_LC_LAG: u64 = 64;

pub type Hash32 = [u8; 32];

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// LC / reflection tip snapshot (class A mint-critical public values).
pub const REFLECTION_SNAPSHOT: Item<ReflectionSnapshot> = Item::new("bridge_reflection");

/// Optional external asset registry contract (hook for later cross-chain wiring).
pub const EXTERNAL_ASSET_REGISTRY: Item<Option<Addr>> = Item::new("bridge_ext_registry");

/// Bridge corridor config (dest domain, K, lag, mock-verify flag).
pub const BRIDGE_CFG: Item<BridgeCfg> = Item::new("bridge_cfg");

/// Internal asset registry: `asset_id_hex → entry`.
pub const ASSET_REGISTRY: Map<&str, AssetEntry> = Map::new("bridge_assets");

/// Once-per-claim minted keys: domain-separated bridge claim keys.
/// Key form: [`bridge_claim_storage_key`].
pub const BRIDGE_MINTED: Map<&str, ()> = Map::new("bridge_minted");

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[cw_serde]
pub struct BridgeCfg {
    /// Terp destination domain / chain bind (32 bytes).
    pub dest_domain: Binary,
    pub confirmations_k: u64,
    pub max_lc_lag: u64,
    /// LC client id string (reflection corridor).
    pub lc_client_id: String,
    /// When true, proof verify is mocked (always-ok membership under claim flags).
    /// Production must set false and wire real verify.
    pub mock_verify: bool,
    /// Circuit id for Option D egress `proof_instance_verify` (production path).
    /// Lab / mock_verify ignores this. Default `None` keeps existing BridgeCfg JSON valid.
    #[serde(default)]
    pub egress_zkid: Option<u64>,
}

#[cw_serde]
pub struct ReflectionSnapshot {
    pub pool_root: Binary,
    pub spent_root: Binary,
    pub burn_root: Binary,
    pub source_height: u64,
    pub tip_height: u64,
    pub confirmations_k: u64,
    pub max_lc_lag: u64,
    pub frozen: bool,
}

impl ReflectionSnapshot {
    pub fn conf_mature(&self) -> bool {
        self.tip_height >= self.source_height.saturating_add(self.confirmations_k)
    }

    pub fn lag_ok(&self) -> bool {
        if !self.conf_mature() {
            return false;
        }
        let residual = self
            .tip_height
            .saturating_sub(self.source_height.saturating_add(self.confirmations_k));
        residual <= self.max_lc_lag
    }

    pub fn tip_ok(&self) -> bool {
        !self.frozen && self.tip_height > 0 && !is_zero_hash(&self.burn_root)
    }
}

#[cw_serde]
#[derive(Copy, Default)]
pub enum AssetStatus {
    #[default]
    Active,
    Paused,
    Frozen,
}

#[cw_serde]
pub struct AssetEntry {
    /// Domain-separated Terp asset id (32 bytes).
    pub asset_id: Binary,
    /// Local denom or proof representation string.
    pub local_denom: String,
    /// Optional origin tag (e.g. `native`, `tacit:bitcoin-mainnet`, IBC trace).
    pub origin: Option<String>,
    pub status: AssetStatus,
}

/// Mint-critical public claim fields (Domain B §9 + C §3.1).
#[cw_serde]
pub struct BridgeMintClaimPublic {
    pub source_chain_tag: String,
    pub tacit_asset_id: Binary,
    pub value_u64: u64,
    pub nullifier: Binary,
    pub dest_commitment: Binary,
    pub dest_domain: Binary,
    pub claim_id: Binary,
    pub source_pool_root: Binary,
    pub source_burn_root: Binary,
    pub source_height: u64,
    pub domain_binding: Binary,
    pub unit_scale: u64,
    pub pool_domain: Binary,
    pub cm_public: Binary,
    /// Optional opening trapdoor; non-zero → note rcm_flag=1.
    #[serde(default)]
    pub rcm: Option<Binary>,
    // --- mock LC membership flags (Round 2 stub; real IMT later) ---
    /// ν present in bridge-burn set under snapshot.burn_root.
    pub in_burn_set: bool,
    /// Burned note membership under pool root.
    pub in_pool_root: bool,
    /// True if ν is only in spent set (H-1 reject when !in_burn_set).
    #[serde(default)]
    pub spent_only: bool,
    /// Expected domain_binding re-derive inputs.
    pub src_chain_id: Binary,
    pub dst_chain_id: Binary,
    pub lc_client_id: Binary,
    /// Burn-bound destCommitment recorded in burn set (A6).
    pub burn_dest_commitment: Binary,
}

/// SEAM-NOTE-OUT-compatible mint result emitted as attribute / event payload.
#[cw_serde]
pub struct NoteOutResult {
    pub version: u8,
    pub origin: u8,
    pub asset_id: Binary,
    pub value: u64,
    pub owner_binding: Binary,
    pub cm_public: Binary,
    pub cm_encoding: u8,
    pub nullifier_lineage: Binary,
    pub nullifier_domain: u8,
    pub provenance_anchor: Binary,
    pub claim_id: Binary,
    pub source_chain_tag_hash: Binary,
    pub rcm: Binary,
    pub rcm_flag: u8,
    pub pool_domain: Binary,
}

impl NoteOutResult {
    pub fn is_dex_consumable(&self) -> bool {
        self.version == 0
            && self.origin == ORIGIN_BRIDGE_MINT
            && self.nullifier_domain == NF_BRIDGE_BURN
            && self.cm_encoding != 0
            && !is_zero_hash(&self.asset_id)
            && !is_zero_hash(&self.cm_public)
            && self.rcm_flag == 1
    }
}

// ---------------------------------------------------------------------------
// Pure helpers (no_std-friendly; sha2 only)
// ---------------------------------------------------------------------------

fn is_zero_hash(b: &Binary) -> bool {
    b.as_slice().iter().all(|&x| x == 0) || b.is_empty()
}

pub(crate) fn require_hash32(b: &Binary, name: &str) -> StdResult<Hash32> {
    let s = b.as_slice();
    if s.len() != 32 {
        return Err(StdError::msg(format!(
            "bridge: {name} must be 32 bytes, got {}",
            s.len()
        )));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(s);
    Ok(out)
}

pub fn hash32_label(label: &str) -> Hash32 {
    let mut h = Sha256::new();
    h.update(label.as_bytes());
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

pub fn hash_source_chain_tag(tag: &str) -> Hash32 {
    hash32_label(tag)
}

pub fn terp_asset_id_from_tacit(
    source_chain_tag: &str,
    tacit_asset_id: &Hash32,
    unit_scale: u64,
) -> Hash32 {
    let mut h = Sha256::new();
    h.update(TERP_ASSET_DOMAIN_TAG);
    h.update(source_chain_tag.as_bytes());
    h.update(tacit_asset_id);
    h.update(unit_scale.to_be_bytes());
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

pub fn derive_claim_id_with_dest(
    dest_domain: &Hash32,
    dest_commitment: &Hash32,
    nu: &Hash32,
    asset_id: &Hash32,
    value: u64,
) -> Hash32 {
    let mut h = Sha256::new();
    h.update(TERP_BRIDGE_CLAIM_TAG);
    h.update(dest_domain);
    h.update(dest_commitment);
    h.update(nu);
    h.update(asset_id);
    h.update(value.to_be_bytes());
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

pub fn derive_domain_binding(
    src_chain_id: &Hash32,
    dst_chain_id: &Hash32,
    lc_client_id: &Hash32,
    asset_id: &Hash32,
    claim_or_nu: &Hash32,
    consensus_height: u64,
    commitment_root: &Hash32,
) -> Hash32 {
    let mut h = Sha256::new();
    h.update(TERP_PRIVATE_BRIDGE_DOMAIN_TAG);
    h.update(src_chain_id);
    h.update(dst_chain_id);
    h.update(lc_client_id);
    h.update(asset_id);
    h.update(claim_or_nu);
    h.update(consensus_height.to_be_bytes());
    h.update(commitment_root);
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

/// Domain-separated storage key for bridge claim once-per-ν (SEAM 0x02).
pub fn bridge_claim_storage_key(nullifier: &Hash32) -> String {
    format!("bridge:{:02x}:{}", NF_BRIDGE_BURN, hex::encode(nullifier))
}

fn asset_id_key(asset_id: &Hash32) -> String {
    hex::encode(asset_id)
}

// ---------------------------------------------------------------------------
// Mock ZK / LC verify (Round 2)
// ---------------------------------------------------------------------------

/// Stub proof verify. Real SP1/LC not required this round.
///
/// Accepts when:
/// - `mock_verify` config is true (or under `cfg!(test)`), **and**
/// - claim asserts `in_burn_set` (membership bool stands in for IMT),
/// - proof is non-empty under production mock flag, or empty allowed in unit tests.
///
/// Rejects spent-only / !in_burn_set before this is called (H-1 gate).
pub fn mock_verify_bridge_proof(
    mock_verify: bool,
    claim: &BridgeMintClaimPublic,
    proof: &Binary,
) -> StdResult<()> {
    let allow_mock = mock_verify || cfg!(test);
    if !allow_mock {
        return Err(StdError::msg(
            "bridge: real proof verify not wired (set mock_verify or wait for SP1/LC)",
        ));
    }
    if !claim.in_burn_set {
        return Err(StdError::msg("bridge: mock verify requires in_burn_set"));
    }
    // Empty proof allowed only in tests; production mock still wants a blob.
    if proof.is_empty() && !cfg!(test) {
        return Err(StdError::msg("bridge: empty proof under mock_verify"));
    }
    let _ = proof;
    Ok(())
}

// ---------------------------------------------------------------------------
// Pure authorize (reimplementation of bridge_auth_seams hinge)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BridgeMintError {
    TipNotOk,
    ZeroBurnRoot,
    ImmatureConfirmation,
    LagExceeded,
    StaleBurnRoot,
    StalePoolRoot,
    NotInBurnSet,
    NotInPoolRoot,
    AlreadyMinted,
    UnmappedAsset,
    AssetNotActive,
    DomainMismatch,
    DestCommitmentMismatch,
    ValueMismatch,
    ClaimIdMismatch,
    DomainBindingMismatch,
    ProofRejected,
    BridgeNotConfigured,
    Unauthorized,
}

impl BridgeMintError {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TipNotOk => "bridge: tip not ok / frozen",
            Self::ZeroBurnRoot => "bridge: zero burn root",
            Self::ImmatureConfirmation => "bridge: immature confirmation",
            Self::LagExceeded => "bridge: lag exceeded",
            Self::StaleBurnRoot => "bridge: stale burn root",
            Self::StalePoolRoot => "bridge: stale pool root",
            Self::NotInBurnSet => "bridge: not in burn set (H-1)",
            Self::NotInPoolRoot => "bridge: not in pool root",
            Self::AlreadyMinted => "bridge: already minted",
            Self::UnmappedAsset => "bridge: unregistered / unmapped asset",
            Self::AssetNotActive => "bridge: asset not active",
            Self::DomainMismatch => "bridge: domain mismatch",
            Self::DestCommitmentMismatch => "bridge: dest commitment mismatch",
            Self::ValueMismatch => "bridge: value mismatch",
            Self::ClaimIdMismatch => "bridge: claim_id mismatch",
            Self::DomainBindingMismatch => "bridge: domain_binding mismatch",
            Self::ProofRejected => "bridge: proof rejected",
            Self::BridgeNotConfigured => "bridge: not configured (missing snapshot or cfg)",
            Self::Unauthorized => "bridge: unauthorized",
        }
    }
}

impl From<BridgeMintError> for StdError {
    fn from(e: BridgeMintError) -> Self {
        StdError::msg(e.as_str())
    }
}

/// Pure gates + registry check → NoteOutResult (does not mutate storage).
pub fn authorize_bridge_mint_pure(
    snapshot: &ReflectionSnapshot,
    cfg: &BridgeCfg,
    claim: &BridgeMintClaimPublic,
    already_minted: bool,
    asset: Option<&AssetEntry>,
) -> Result<NoteOutResult, BridgeMintError> {
    if !snapshot.tip_ok() {
        if snapshot.frozen || snapshot.tip_height == 0 {
            return Err(BridgeMintError::TipNotOk);
        }
        if is_zero_hash(&snapshot.burn_root) {
            return Err(BridgeMintError::ZeroBurnRoot);
        }
        return Err(BridgeMintError::TipNotOk);
    }
    if is_zero_hash(&snapshot.burn_root) {
        return Err(BridgeMintError::ZeroBurnRoot);
    }

    if claim.source_height != snapshot.source_height {
        return Err(BridgeMintError::StaleBurnRoot);
    }
    if !snapshot.conf_mature() {
        return Err(BridgeMintError::ImmatureConfirmation);
    }
    if !snapshot.lag_ok() {
        return Err(BridgeMintError::LagExceeded);
    }

    let burn_root = require_hash32(&snapshot.burn_root, "burn_root")
        .map_err(|_| BridgeMintError::StaleBurnRoot)?;
    let pool_root = require_hash32(&snapshot.pool_root, "pool_root")
        .map_err(|_| BridgeMintError::StalePoolRoot)?;
    let source_burn =
        require_hash32(&claim.source_burn_root, "source_burn_root")
            .map_err(|_| BridgeMintError::StaleBurnRoot)?;
    let source_pool =
        require_hash32(&claim.source_pool_root, "source_pool_root")
            .map_err(|_| BridgeMintError::StalePoolRoot)?;

    if source_burn != burn_root {
        return Err(BridgeMintError::StaleBurnRoot);
    }
    if source_pool != pool_root {
        return Err(BridgeMintError::StalePoolRoot);
    }

    // H-1: burn-set membership is mint authority; spent_only alone rejects.
    if claim.spent_only && !claim.in_burn_set {
        return Err(BridgeMintError::NotInBurnSet);
    }
    if !claim.in_burn_set {
        return Err(BridgeMintError::NotInBurnSet);
    }
    if !claim.in_pool_root {
        return Err(BridgeMintError::NotInPoolRoot);
    }

    if already_minted {
        return Err(BridgeMintError::AlreadyMinted);
    }

    let tacit = require_hash32(&claim.tacit_asset_id, "tacit_asset_id")
        .map_err(|_| BridgeMintError::UnmappedAsset)?;
    let terp_asset =
        terp_asset_id_from_tacit(&claim.source_chain_tag, &tacit, claim.unit_scale);

    let entry = asset.ok_or(BridgeMintError::UnmappedAsset)?;
    let entry_id =
        require_hash32(&entry.asset_id, "asset_id").map_err(|_| BridgeMintError::UnmappedAsset)?;
    // Accept if registered asset_id equals mapped terp id or raw tacit id.
    if entry_id != terp_asset && entry_id != tacit {
        return Err(BridgeMintError::UnmappedAsset);
    }
    if !matches!(entry.status, AssetStatus::Active) {
        return Err(BridgeMintError::AssetNotActive);
    }

    let dest_domain =
        require_hash32(&claim.dest_domain, "dest_domain").map_err(|_| BridgeMintError::DomainMismatch)?;
    let expected_dest = require_hash32(&cfg.dest_domain, "cfg.dest_domain")
        .map_err(|_| BridgeMintError::DomainMismatch)?;
    if dest_domain != expected_dest {
        return Err(BridgeMintError::DomainMismatch);
    }

    let dest_cm = require_hash32(&claim.dest_commitment, "dest_commitment")
        .map_err(|_| BridgeMintError::DestCommitmentMismatch)?;
    let burn_dest = require_hash32(&claim.burn_dest_commitment, "burn_dest_commitment")
        .map_err(|_| BridgeMintError::DestCommitmentMismatch)?;
    if dest_cm != burn_dest {
        return Err(BridgeMintError::DestCommitmentMismatch);
    }

    // Conservation: claim value is mint value (A9).
    if claim.value_u64 == 0 {
        return Err(BridgeMintError::ValueMismatch);
    }

    let nu = require_hash32(&claim.nullifier, "nullifier")
        .map_err(|_| BridgeMintError::ClaimIdMismatch)?;
    let expected_claim =
        derive_claim_id_with_dest(&dest_domain, &dest_cm, &nu, &tacit, claim.value_u64);
    let presented_claim =
        require_hash32(&claim.claim_id, "claim_id").map_err(|_| BridgeMintError::ClaimIdMismatch)?;
    if presented_claim != expected_claim {
        return Err(BridgeMintError::ClaimIdMismatch);
    }

    let src_chain = require_hash32(&claim.src_chain_id, "src_chain_id")
        .map_err(|_| BridgeMintError::DomainBindingMismatch)?;
    let dst_chain = require_hash32(&claim.dst_chain_id, "dst_chain_id")
        .map_err(|_| BridgeMintError::DomainBindingMismatch)?;
    let lc_id = require_hash32(&claim.lc_client_id, "lc_client_id")
        .map_err(|_| BridgeMintError::DomainBindingMismatch)?;
    let expected_binding = derive_domain_binding(
        &src_chain,
        &dst_chain,
        &lc_id,
        &tacit,
        &nu,
        claim.source_height,
        &source_burn,
    );
    let presented_binding = require_hash32(&claim.domain_binding, "domain_binding")
        .map_err(|_| BridgeMintError::DomainBindingMismatch)?;
    if presented_binding != expected_binding {
        return Err(BridgeMintError::DomainBindingMismatch);
    }

    let cm_public =
        require_hash32(&claim.cm_public, "cm_public").map_err(|_| BridgeMintError::ValueMismatch)?;
    let pool_domain = require_hash32(&claim.pool_domain, "pool_domain")
        .map_err(|_| BridgeMintError::DomainMismatch)?;

    let (rcm, rcm_flag) = match &claim.rcm {
        Some(b) if !is_zero_hash(b) => {
            let r = require_hash32(b, "rcm").map_err(|_| BridgeMintError::ValueMismatch)?;
            (r, 1u8)
        }
        _ => ([0u8; 32], 0u8),
    };

    Ok(NoteOutResult {
        version: 0,
        origin: ORIGIN_BRIDGE_MINT,
        asset_id: Binary::from(entry_id.to_vec()),
        value: claim.value_u64,
        owner_binding: Binary::from(dest_cm.to_vec()),
        cm_public: Binary::from(cm_public.to_vec()),
        cm_encoding: CM_ABSTRACT_LEAF_V0,
        nullifier_lineage: Binary::from(nu.to_vec()),
        nullifier_domain: NF_BRIDGE_BURN,
        provenance_anchor: Binary::from(source_burn.to_vec()),
        claim_id: Binary::from(presented_claim.to_vec()),
        source_chain_tag_hash: Binary::from(hash_source_chain_tag(&claim.source_chain_tag).to_vec()),
        rcm: Binary::from(rcm.to_vec()),
        rcm_flag,
        pool_domain: Binary::from(pool_domain.to_vec()),
    })
}

// ---------------------------------------------------------------------------
// Execute handlers
// ---------------------------------------------------------------------------

/// Ensure bridge corridor config exists (lazy default for tests / gradual rollout).
pub fn ensure_bridge_cfg(
    storage: &mut dyn Storage,
    dest_domain: Binary,
    mock_verify: bool,
) -> StdResult<BridgeCfg> {
    if let Some(c) = BRIDGE_CFG.may_load(storage)? {
        return Ok(c);
    }
    let c = BridgeCfg {
        dest_domain,
        confirmations_k: DEFAULT_CONFIRMATIONS_K,
        max_lc_lag: DEFAULT_MAX_LC_LAG,
        lc_client_id: "08-wasm-tacit-reflection-0".into(),
        mock_verify,
        egress_zkid: None,
    };
    BRIDGE_CFG.save(storage, &c)?;
    Ok(c)
}

pub fn execute_update_reflection(
    deps: DepsMut,
    info: MessageInfo,
    pool_root: Binary,
    spent_root: Binary,
    burn_root: Binary,
    source_height: u64,
    tip_height: u64,
) -> Result<Response, StdError> {
    cw_ownable::assert_owner(deps.storage, &info.sender)?;
    let cfg = BRIDGE_CFG.may_load(deps.storage)?;
    let (k, lag) = match cfg {
        Some(c) => (c.confirmations_k, c.max_lc_lag),
        None => (DEFAULT_CONFIRMATIONS_K, DEFAULT_MAX_LC_LAG),
    };
    let snap = ReflectionSnapshot {
        pool_root: pool_root.clone(),
        spent_root: spent_root.clone(),
        burn_root: burn_root.clone(),
        source_height,
        tip_height,
        confirmations_k: k,
        max_lc_lag: lag,
        frozen: false,
    };
    REFLECTION_SNAPSHOT.save(deps.storage, &snap)?;
    Ok(Response::new()
        .add_attribute("action", "update_reflection")
        .add_attribute("source_height", source_height.to_string())
        .add_attribute("tip_height", tip_height.to_string()))
}

pub fn execute_set_reflection_snapshot(
    deps: DepsMut,
    info: MessageInfo,
    snapshot: ReflectionSnapshot,
) -> Result<Response, StdError> {
    cw_ownable::assert_owner(deps.storage, &info.sender)?;
    REFLECTION_SNAPSHOT.save(deps.storage, &snapshot)?;
    Ok(Response::new()
        .add_attribute("action", "set_reflection_snapshot")
        .add_attribute("source_height", snapshot.source_height.to_string())
        .add_attribute("tip_height", snapshot.tip_height.to_string())
        .add_attribute("frozen", snapshot.frozen.to_string()))
}

pub fn execute_register_asset(
    deps: DepsMut,
    info: MessageInfo,
    asset_id: Binary,
    local_denom: String,
    origin: Option<String>,
    status: Option<AssetStatus>,
) -> Result<Response, StdError> {
    cw_ownable::assert_owner(deps.storage, &info.sender)?;
    let id = require_hash32(&asset_id, "asset_id")?;
    let key = asset_id_key(&id);
    if ASSET_REGISTRY.may_load(deps.storage, &key)?.is_some() {
        return Err(StdError::msg("bridge: asset already registered"));
    }
    let entry = AssetEntry {
        asset_id: Binary::from(id.to_vec()),
        local_denom: local_denom.clone(),
        origin,
        status: status.unwrap_or(AssetStatus::Active),
    };
    ASSET_REGISTRY.save(deps.storage, &key, &entry)?;
    Ok(Response::new()
        .add_attribute("action", "register_asset")
        .add_attribute("asset_id", key)
        .add_attribute("local_denom", local_denom))
}

pub fn execute_set_external_asset_registry(
    deps: DepsMut,
    info: MessageInfo,
    addr: Option<String>,
) -> Result<Response, StdError> {
    cw_ownable::assert_owner(deps.storage, &info.sender)?;
    let validated = match addr {
        Some(a) => Some(deps.api.addr_validate(&a)?),
        None => None,
    };
    EXTERNAL_ASSET_REGISTRY.save(deps.storage, &validated)?;
    Ok(Response::new()
        .add_attribute("action", "set_external_asset_registry")
        .add_attribute(
            "addr",
            validated
                .as_ref()
                .map(|a| a.to_string())
                .unwrap_or_else(|| "none".into()),
        ))
}

pub fn execute_set_bridge_cfg(
    deps: DepsMut,
    info: MessageInfo,
    cfg: BridgeCfg,
) -> Result<Response, StdError> {
    cw_ownable::assert_owner(deps.storage, &info.sender)?;
    require_hash32(&cfg.dest_domain, "dest_domain")?;
    BRIDGE_CFG.save(deps.storage, &cfg)?;
    Ok(Response::new()
        .add_attribute("action", "set_bridge_cfg")
        .add_attribute("mock_verify", cfg.mock_verify.to_string())
        .add_attribute("confirmations_k", cfg.confirmations_k.to_string()))
}

pub fn execute_bridge_mint_note(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    claim: BridgeMintClaimPublic,
    proof: Binary,
) -> Result<Response, StdError> {
    let cfg = BRIDGE_CFG
        .may_load(deps.storage)?
        .ok_or(BridgeMintError::BridgeNotConfigured)?;
    let snapshot = REFLECTION_SNAPSHOT
        .may_load(deps.storage)?
        .ok_or(BridgeMintError::BridgeNotConfigured)?;

    let nu = require_hash32(&claim.nullifier, "nullifier")?;
    let mint_key = bridge_claim_storage_key(&nu);
    let already = BRIDGE_MINTED.may_load(deps.storage, &mint_key)?.is_some();

    // Resolve asset from internal registry (external query hook later).
    let tacit = require_hash32(&claim.tacit_asset_id, "tacit_asset_id")?;
    let terp_asset =
        terp_asset_id_from_tacit(&claim.source_chain_tag, &tacit, claim.unit_scale);
    let mut asset = ASSET_REGISTRY.may_load(deps.storage, &asset_id_key(&terp_asset))?;
    if asset.is_none() {
        asset = ASSET_REGISTRY.may_load(deps.storage, &asset_id_key(&tacit))?;
    }

    // Pure gates first (includes H-1 spent-only).
    let note = authorize_bridge_mint_pure(
        &snapshot,
        &cfg,
        &claim,
        already,
        asset.as_ref(),
    )?;

    // Mock / stub proof verify (Round 2).
    mock_verify_bridge_proof(cfg.mock_verify, &claim, &proof)
        .map_err(|_| BridgeMintError::ProofRejected)?;

    // Mark once-per-ν (domain-separated bridge claim key).
    BRIDGE_MINTED.save(deps.storage, &mint_key, &())?;

    // Also pin into shared NULLIFIERS map under bridge domain for product surface unity.
    // (Claim path uses root_id-scoped keys; bridge uses bridge:0x02:hex.)
    crate::NULLIFIERS.save(deps.storage, mint_key.clone(), &())?;

    let note_bin = to_json_binary(&note)?;
    Ok(Response::new()
        .add_attribute("action", "bridge_mint_note")
        .add_attribute("nullifier", hex::encode(nu))
        .add_attribute("claim_key", mint_key)
        .add_attribute("value", note.value.to_string())
        .add_attribute("rcm_flag", note.rcm_flag.to_string())
        .add_attribute("dex_consumable", note.is_dex_consumable().to_string())
        .add_attribute("note_out", note_bin.to_base64()))
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

pub fn query_reflection_tip(deps: Deps) -> StdResult<Binary> {
    let snap = REFLECTION_SNAPSHOT.may_load(deps.storage)?;
    to_json_binary(&snap)
}

pub fn query_is_bridge_minted(deps: Deps, nullifier: Binary) -> StdResult<Binary> {
    let nu = require_hash32(&nullifier, "nullifier")?;
    let key = bridge_claim_storage_key(&nu);
    let minted = BRIDGE_MINTED.may_load(deps.storage, &key)?.is_some();
    to_json_binary(&minted)
}

pub fn query_asset(deps: Deps, asset_id: Binary) -> StdResult<Binary> {
    let id = require_hash32(&asset_id, "asset_id")?;
    let entry = ASSET_REGISTRY.may_load(deps.storage, &asset_id_key(&id))?;
    to_json_binary(&entry)
}

pub fn query_bridge_cfg(deps: Deps) -> StdResult<Binary> {
    to_json_binary(&BRIDGE_CFG.may_load(deps.storage)?)
}

pub fn query_external_asset_registry(deps: Deps) -> StdResult<Binary> {
    to_json_binary(&EXTERNAL_ASSET_REGISTRY.may_load(deps.storage)?)
}

// ---------------------------------------------------------------------------
// Public L1 fixtures (shared by unit tests + zk-test-press PrivateBridgeSuite)
// ---------------------------------------------------------------------------

fn bin32(h: Hash32) -> Binary {
    Binary::from(h.to_vec())
}

/// Happy-path bridge world aligned with `bridge_auth_seams::hinge_happy_fixture`
/// and ROUND2-BRIDGE unit tests (Binary-encoded CosmWasm types).
///
/// Returns `(snapshot, cfg, claim, terp_asset_id, asset_entry)`.
/// `cfg.mock_verify = true` so multi-test / cw-orch Mock can mint without SP1.
pub fn happy_bridge_mint_world() -> (
    ReflectionSnapshot,
    BridgeCfg,
    BridgeMintClaimPublic,
    Hash32,
    AssetEntry,
) {
    let dest = hash32_label("terp-chain-1");
    let tacit = hash32_label("tacit-btc-etch-1");
    let unit_scale = 1u64;
    let terp_asset = terp_asset_id_from_tacit("bitcoin-mainnet", &tacit, unit_scale);
    let nu = hash32_label("nu-hinge-happy");
    let dest_cm = hash32_label("dest-commitment-A");
    let pool_root = hash32_label("pool-root-1");
    let spent_root = hash32_label("spent-root-1");
    let burn_root = hash32_label("burn-root-1");
    let height = 100u64;
    let tip = height + DEFAULT_CONFIRMATIONS_K;
    let claim_id = derive_claim_id_with_dest(&dest, &dest_cm, &nu, &tacit, 1_000_000);
    let src_chain = hash32_label("src-bitcoin-mainnet");
    let dst_chain = dest;
    let lc_client = hash32_label("lc-client-reflection-0");
    let domain_binding = derive_domain_binding(
        &src_chain,
        &dst_chain,
        &lc_client,
        &tacit,
        &nu,
        height,
        &burn_root,
    );
    let rcm = hash32_label("rcm-hinge-happy");

    let snapshot = ReflectionSnapshot {
        pool_root: bin32(pool_root),
        spent_root: bin32(spent_root),
        burn_root: bin32(burn_root),
        source_height: height,
        tip_height: tip,
        confirmations_k: DEFAULT_CONFIRMATIONS_K,
        max_lc_lag: DEFAULT_MAX_LC_LAG,
        frozen: false,
    };
    let cfg = BridgeCfg {
        dest_domain: bin32(dest),
        confirmations_k: DEFAULT_CONFIRMATIONS_K,
        max_lc_lag: DEFAULT_MAX_LC_LAG,
        lc_client_id: "08-wasm-tacit-reflection-0".into(),
        mock_verify: true,
        egress_zkid: None,
    };
    let claim = BridgeMintClaimPublic {
        source_chain_tag: "bitcoin-mainnet".into(),
        tacit_asset_id: bin32(tacit),
        value_u64: 1_000_000,
        nullifier: bin32(nu),
        dest_commitment: bin32(dest_cm),
        dest_domain: bin32(dest),
        claim_id: bin32(claim_id),
        source_pool_root: bin32(pool_root),
        source_burn_root: bin32(burn_root),
        source_height: height,
        domain_binding: bin32(domain_binding),
        unit_scale,
        pool_domain: bin32(hash32_label("terp-pool-0")),
        cm_public: bin32(hash32_label("cm-leaf-dest-1")),
        rcm: Some(bin32(rcm)),
        in_burn_set: true,
        in_pool_root: true,
        spent_only: false,
        src_chain_id: bin32(src_chain),
        dst_chain_id: bin32(dst_chain),
        lc_client_id: bin32(lc_client),
        burn_dest_commitment: bin32(dest_cm),
    };
    let asset = AssetEntry {
        asset_id: bin32(terp_asset),
        local_denom: "ubtc".into(),
        origin: Some("tacit:bitcoin-mainnet".into()),
        status: AssetStatus::Active,
    };
    (snapshot, cfg, claim, terp_asset, asset)
}

/// Opaque mock membership / reflection receipt blob (non-empty for production mock_verify).
pub fn mock_bridge_proof_bytes() -> Binary {
    Binary::from(vec![0xde, 0xad, 0xbe, 0xef])
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
    use cosmwasm_std::Addr;

    fn fixture() -> (
        ReflectionSnapshot,
        BridgeCfg,
        BridgeMintClaimPublic,
        Hash32,
        AssetEntry,
    ) {
        happy_bridge_mint_world()
    }

    fn seed_owner_and_bridge(
        deps: &mut cosmwasm_std::OwnedDeps<
            cosmwasm_std::testing::MockStorage,
            cosmwasm_std::testing::MockApi,
            cosmwasm_std::testing::MockQuerier,
        >,
        owner: &Addr,
    ) {
        cw_ownable::initialize_owner(&mut deps.storage, &deps.api, Some(owner.as_str())).unwrap();
        let (snapshot, cfg, _, terp_asset, asset) = fixture();
        BRIDGE_CFG.save(&mut deps.storage, &cfg).unwrap();
        REFLECTION_SNAPSHOT
            .save(&mut deps.storage, &snapshot)
            .unwrap();
        ASSET_REGISTRY
            .save(&mut deps.storage, &asset_id_key(&terp_asset), &asset)
            .unwrap();
    }

    #[test]
    fn pure_happy_path_dex_consumable() {
        let (snapshot, cfg, claim, _, asset) = fixture();
        let note = authorize_bridge_mint_pure(&snapshot, &cfg, &claim, false, Some(&asset))
            .expect("happy");
        assert_eq!(note.origin, ORIGIN_BRIDGE_MINT);
        assert_eq!(note.nullifier_domain, NF_BRIDGE_BURN);
        assert_eq!(note.rcm_flag, 1);
        assert!(note.is_dex_consumable());
        assert_eq!(note.value, 1_000_000);
    }

    #[test]
    fn pure_h1_spent_only_reject() {
        let (snapshot, cfg, mut claim, _, asset) = fixture();
        claim.in_burn_set = false;
        claim.spent_only = true;
        assert_eq!(
            authorize_bridge_mint_pure(&snapshot, &cfg, &claim, false, Some(&asset)),
            Err(BridgeMintError::NotInBurnSet)
        );
    }

    #[test]
    fn pure_unmapped_asset_reject() {
        let (snapshot, cfg, claim, _, _) = fixture();
        assert_eq!(
            authorize_bridge_mint_pure(&snapshot, &cfg, &claim, false, None),
            Err(BridgeMintError::UnmappedAsset)
        );
    }

    #[test]
    fn pure_double_mint_flag_reject() {
        let (snapshot, cfg, claim, _, asset) = fixture();
        assert_eq!(
            authorize_bridge_mint_pure(&snapshot, &cfg, &claim, true, Some(&asset)),
            Err(BridgeMintError::AlreadyMinted)
        );
    }

    #[test]
    fn execute_unregistered_asset_reject() {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make("owner");
        cw_ownable::initialize_owner(&mut deps.storage, &deps.api, Some(owner.as_str())).unwrap();
        let (snapshot, cfg, claim, _, _) = fixture();
        // Snapshot + cfg, but NO asset registration.
        BRIDGE_CFG.save(&mut deps.storage, &cfg).unwrap();
        REFLECTION_SNAPSHOT
            .save(&mut deps.storage, &snapshot)
            .unwrap();

        let info = message_info(&owner, &[]);
        let err = execute_bridge_mint_note(
            deps.as_mut(),
            mock_env(),
            info,
            claim,
            Binary::from(vec![0xde, 0xad]),
        )
        .unwrap_err();
        assert!(err.to_string().contains("unregistered") || err.to_string().contains("unmapped"));
    }

    #[test]
    fn execute_double_mint_reject() {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make("owner");
        seed_owner_and_bridge(&mut deps, &owner);
        let (_, _, claim, _, _) = fixture();
        let info = message_info(&owner, &[]);
        let env = mock_env();

        execute_bridge_mint_note(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            claim.clone(),
            Binary::from(vec![0xde, 0xad]),
        )
        .expect("first mint");

        let err = execute_bridge_mint_note(
            deps.as_mut(),
            env,
            info,
            claim,
            Binary::from(vec![0xde, 0xad]),
        )
        .unwrap_err();
        assert!(err.to_string().contains("already minted"));
    }

    #[test]
    fn execute_h1_spent_only_reject() {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make("owner");
        seed_owner_and_bridge(&mut deps, &owner);
        let (_, _, mut claim, _, _) = fixture();
        claim.in_burn_set = false;
        claim.spent_only = true;
        let info = message_info(&owner, &[]);
        let err = execute_bridge_mint_note(
            deps.as_mut(),
            mock_env(),
            info,
            claim,
            Binary::from(vec![0x01]),
        )
        .unwrap_err();
        assert!(err.to_string().contains("H-1") || err.to_string().contains("burn set"));
    }

    #[test]
    fn execute_happy_path_mock_verify() {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make("owner");
        seed_owner_and_bridge(&mut deps, &owner);
        let (_, _, claim, terp_asset, _) = fixture();
        let info = message_info(&owner, &[]);
        let res = execute_bridge_mint_note(
            deps.as_mut(),
            mock_env(),
            info,
            claim.clone(),
            Binary::from(vec![0xde, 0xad, 0xbe, 0xef]),
        )
        .expect("happy mint");

        assert!(
            res.attributes
                .iter()
                .any(|a| a.key == "action" && a.value == "bridge_mint_note")
        );
        assert!(
            res.attributes
                .iter()
                .any(|a| a.key == "dex_consumable" && a.value == "true")
        );
        assert!(
            res.attributes
                .iter()
                .any(|a| a.key == "rcm_flag" && a.value == "1")
        );

        let nu = require_hash32(&claim.nullifier, "nullifier").unwrap();
        let key = bridge_claim_storage_key(&nu);
        assert!(BRIDGE_MINTED
            .may_load(&deps.storage, &key)
            .unwrap()
            .is_some());
        // Registered asset still present; mint did not invent a new balance path via registry.
        assert!(ASSET_REGISTRY
            .may_load(&deps.storage, &asset_id_key(&terp_asset))
            .unwrap()
            .is_some());
    }

    #[test]
    fn register_asset_and_set_external() {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make("owner");
        cw_ownable::initialize_owner(&mut deps.storage, &deps.api, Some(owner.as_str())).unwrap();
        let info = message_info(&owner, &[]);
        let aid = hash32_label("asset-x");
        execute_register_asset(
            deps.as_mut(),
            info.clone(),
            bin32(aid),
            "ux".into(),
            Some("native".into()),
            None,
        )
        .unwrap();
        let entry = ASSET_REGISTRY
            .load(&deps.storage, &asset_id_key(&aid))
            .unwrap();
        assert_eq!(entry.local_denom, "ux");
        assert!(matches!(entry.status, AssetStatus::Active));

        let ext = deps.api.addr_make("ext-registry");
        execute_set_external_asset_registry(deps.as_mut(), info, Some(ext.to_string())).unwrap();
        let stored = EXTERNAL_ASSET_REGISTRY.load(&deps.storage).unwrap();
        assert_eq!(stored, Some(ext));
    }

    #[test]
    fn domain_separated_claim_key() {
        let nu = hash32_label("nu-1");
        let k = bridge_claim_storage_key(&nu);
        assert!(k.starts_with("bridge:02:"));
        assert_ne!(k, hex::encode(nu));
    }
}
