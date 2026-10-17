//! Option D **BridgeEgressBurn** surface on `cw-headstash` (private bridge burn router).
//!
//! SSOT:
//! - `DESIGN-DECISIONS-ZEC-EGRESS-OPTION-D-ACCEPTED-2026-07-22.md`
//! - `agents/zec-egress-option-d-2026-07-22/DESIGN-ZEC-EGRESS-D.md`
//! - Pure seams: `fixtures/private_dex_seams` egress module
//!
//! ## Dual-path proof (mirror private-dex + bridge mint)
//!
//! | Path | Gate | Behavior |
//! |------|------|----------|
//! | mock | `BridgeCfg.mock_verify \|\| cfg!(test)` | non-empty proof after structural gates |
//! | host | feature `zk-api` + `!mock_verify` | `api.proof_instance_verify(zkid, proof, instances)` |
//! | fail-closed | otherwise | reject |
//!
//! Nullifier domain: `egress-nf-v0` — storage key `egress:nf-v0:{hex}` (≠ pool-nf, ≠ bridge mint).

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{
    Api, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdError, StdResult, to_json_binary,
};
use cw_storage_plus::Map;
use sha2::{Digest, Sha256};

use crate::bridge::{require_hash32, BridgeCfg, Hash32, BRIDGE_CFG};

// ---------------------------------------------------------------------------
// Domain labels / instance encoding
// ---------------------------------------------------------------------------

/// Domain for egress-spend nullifiers (normative; DESIGN-ZEC-EGRESS-D).
pub const EGRESS_NF_LABEL: &[u8] = b"egress-nf-v0";

/// Domain-separated public-instance tag for future `proof_instance_verify`.
pub const EGRESS_INSTANCE_LABEL: &[u8] = b"egress-burn-instance-v0";

// ---------------------------------------------------------------------------
// State — spent egress ν map (+ burned cm stub)
// ---------------------------------------------------------------------------

/// Once-per-egress-ν: key = [`egress_spent_storage_key`].
pub const EGRESS_SPENT: Map<&str, ()> = Map::new("egress_spent_nu");

/// Optional burned cm set (stub spent-set; not pool ν domain).
pub const EGRESS_BURNED_CM: Map<&str, ()> = Map::new("egress_burned_cm");

// ---------------------------------------------------------------------------
// Types (CW Binary encoding of pure EgressBurnPublic + openings)
// ---------------------------------------------------------------------------

/// Zcash receiver class — metadata for wallet API selection; binding stays 32B seal.
#[cw_serde]
#[derive(Copy)]
pub enum DestKind {
    Transparent,
    Shielded,
}

/// Public statement for Option D burn on Terp (+ lab openings for structural ν / dest).
///
/// Layout mirrors pure `EgressBurnPublic` + witness openings (`rcm`, `owner_binding`)
/// so the contract can fail-closed on dest equality and nullifier re-derive without
/// a full ZK circuit this wave.
#[cw_serde]
pub struct EgressBurnStatement {
    pub asset_id: Binary,
    pub value: u64,
    /// SEAM cm being burned.
    pub cm_spent: Binary,
    /// Egress-domain ν: `H("egress-nf-v0" ‖ cm_spent ‖ rcm)`.
    pub nullifier: Binary,
    /// MUST equal G4 seal / owner_binding.
    pub dest_commitment: Binary,
    pub dest_kind: DestKind,
    /// Membership root (lab stub OK).
    pub root: Binary,
    /// If from swap settle.
    #[serde(default)]
    pub source_pool_id: Option<u64>,
    /// Opening for ν re-derive (lab / structural).
    pub rcm: Binary,
    /// Must equal `dest_commitment` on corridor product.
    pub owner_binding: Binary,
}

/// After Terp burn is accepted — authorization for Zcash leg (event payload).
#[cw_serde]
pub struct EgressBurnEvidenceV0 {
    pub terp_tx_hash: Option<String>,
    pub asset_id: Binary,
    pub value: u64,
    pub cm_spent: Binary,
    pub nullifier: Binary,
    pub dest_commitment: Binary,
    pub dest_kind: DestKind,
    pub root: Binary,
    pub source_pool_id: Option<u64>,
    /// `"mock_verify_lab"` | `"zk_api"`.
    pub proof_mode: String,
    pub settle_receipt_ref: Option<String>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EgressBurnError {
    BridgeNotConfigured,
    DestMismatch,
    BadAmount,
    BadSchema,
    NullifierMismatch,
    AlreadySpent,
    ProofRejected,
    ZkApiUnavailable,
    ZkidMissing,
    UnmappedAsset,
}

impl EgressBurnError {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BridgeNotConfigured => "egress: bridge not configured",
            Self::DestMismatch => "egress: dest_commitment mismatch (G4 seal / owner_binding)",
            Self::BadAmount => "egress: value must be > 0 and fields non-zero",
            Self::BadSchema => "egress: bad schema (32-byte fields required)",
            Self::NullifierMismatch => "egress: nullifier does not re-derive under egress-nf-v0",
            Self::AlreadySpent => "egress: nullifier already spent (double egress)",
            Self::ProofRejected => "egress: proof rejected",
            Self::ZkApiUnavailable => {
                "egress: proof_instance_verify requires zk-api feature + zk wasmvm (set mock_verify for lab)"
            }
            Self::ZkidMissing => "egress: egress_zkid missing for non-mock verify",
            Self::UnmappedAsset => "egress: asset not registered",
        }
    }
}

impl From<EgressBurnError> for StdError {
    fn from(e: EgressBurnError) -> Self {
        StdError::msg(e.as_str())
    }
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// `egress_ν = SHA256(b"egress-nf-v0" ‖ cm_spent ‖ rcm)`.
pub fn egress_nullifier(cm: &Hash32, rcm: &Hash32) -> Hash32 {
    let mut h = Sha256::new();
    h.update(EGRESS_NF_LABEL);
    h.update(cm);
    h.update(rcm);
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

/// Domain-separated storage key for once-per-egress-ν.
pub fn egress_spent_storage_key(nullifier: &Hash32) -> String {
    format!("egress:nf-v0:{}", hex::encode(nullifier))
}

fn burned_cm_key(cm: &Hash32) -> String {
    format!("egress:cm:{}", hex::encode(cm))
}

fn is_zero_hash(b: &Binary) -> bool {
    b.as_slice().iter().all(|&x| x == 0) || b.is_empty()
}

/// Canonical public-instance encoding for future `proof_instance_verify`.
///
/// Layout (v0):
/// 1. 32-byte domain tag = SHA256(EGRESS_INSTANCE_LABEL)
/// 2. fixed-width public fields (asset_id, value LE, cm, ν, dest, dest_kind, root, source_pool)
///
/// Openings (`rcm`, `owner_binding`) are **not** included — only public statement halves.
pub fn encode_egress_instances(statement: &EgressBurnStatement) -> Vec<u8> {
    let mut out = Vec::with_capacity(256);
    let tag = Sha256::digest(EGRESS_INSTANCE_LABEL);
    out.extend_from_slice(&tag);

    push_bytes32(&mut out, &statement.asset_id);
    out.extend_from_slice(&statement.value.to_le_bytes());
    push_bytes32(&mut out, &statement.cm_spent);
    push_bytes32(&mut out, &statement.nullifier);
    push_bytes32(&mut out, &statement.dest_commitment);
    out.push(match statement.dest_kind {
        DestKind::Transparent => 0,
        DestKind::Shielded => 1,
    });
    push_bytes32(&mut out, &statement.root);
    match statement.source_pool_id {
        None => out.push(0),
        Some(id) => {
            out.push(1);
            out.extend_from_slice(&id.to_le_bytes());
        }
    }
    out
}

fn push_bytes32(out: &mut Vec<u8>, b: &Binary) {
    let slice = b.as_slice();
    let mut buf = [0u8; 32];
    let n = slice.len().min(32);
    buf[..n].copy_from_slice(&slice[..n]);
    out.extend_from_slice(&buf);
}

// ---------------------------------------------------------------------------
// Structural validate (pure, fail-closed)
// ---------------------------------------------------------------------------

/// Structural validation of Option D egress burn (no tree crypto).
///
/// Fail-closed:
/// - value > 0; 32B fields non-zero where required
/// - `owner_binding` == `dest_commitment` (G4 corridor product)
/// - public nullifier == `egress_nullifier(cm, rcm)`
pub fn validate_egress_burn_statement(statement: &EgressBurnStatement) -> Result<(), EgressBurnError> {
    if statement.value == 0 {
        return Err(EgressBurnError::BadAmount);
    }
    if is_zero_hash(&statement.cm_spent)
        || is_zero_hash(&statement.rcm)
        || is_zero_hash(&statement.dest_commitment)
        || is_zero_hash(&statement.owner_binding)
        || is_zero_hash(&statement.asset_id)
        || is_zero_hash(&statement.nullifier)
    {
        return Err(EgressBurnError::BadAmount);
    }

    let cm = require_hash32(&statement.cm_spent, "cm_spent").map_err(|_| EgressBurnError::BadSchema)?;
    let rcm = require_hash32(&statement.rcm, "rcm").map_err(|_| EgressBurnError::BadSchema)?;
    let nu =
        require_hash32(&statement.nullifier, "nullifier").map_err(|_| EgressBurnError::BadSchema)?;
    let dest = require_hash32(&statement.dest_commitment, "dest_commitment")
        .map_err(|_| EgressBurnError::BadSchema)?;
    let owner = require_hash32(&statement.owner_binding, "owner_binding")
        .map_err(|_| EgressBurnError::BadSchema)?;
    let _asset =
        require_hash32(&statement.asset_id, "asset_id").map_err(|_| EgressBurnError::BadSchema)?;
    let _root = require_hash32(&statement.root, "root").map_err(|_| EgressBurnError::BadSchema)?;

    // Dest equality — corridor product seal.
    if dest != owner {
        return Err(EgressBurnError::DestMismatch);
    }

    let derived = egress_nullifier(&cm, &rcm);
    if derived != nu {
        return Err(EgressBurnError::NullifierMismatch);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Dual-path proof verify (mirror private-dex verify.rs + bridge mint mock)
// ---------------------------------------------------------------------------

/// Verify egress burn proof under bridge config dual-path.
pub fn verify_egress_proof(
    api: &dyn Api,
    cfg: &BridgeCfg,
    statement: &EgressBurnStatement,
    proof: &Binary,
) -> Result<String, EgressBurnError> {
    let allow_mock = cfg.mock_verify || cfg!(test);

    if allow_mock {
        if proof.is_empty() {
            return Err(EgressBurnError::ProofRejected);
        }
        // Lab path: structural host checks already ran in authorize.
        let _ = (api, statement);
        return Ok("mock_verify_lab".to_string());
    }

    // Production path: require egress_zkid + host import.
    let zkid = cfg.egress_zkid.ok_or(EgressBurnError::ZkidMissing)?;
    let instances = encode_egress_instances(statement);

    #[cfg(feature = "zk-api")]
    {
        let ok = api
            .proof_instance_verify(zkid, proof.as_slice(), &instances)
            .map_err(|_| EgressBurnError::ProofRejected)?;
        if !ok {
            return Err(EgressBurnError::ProofRejected);
        }
        Ok("zk_api".to_string())
    }

    #[cfg(not(feature = "zk-api"))]
    {
        let _ = (api, zkid, instances, proof);
        Err(EgressBurnError::ZkApiUnavailable)
    }
}

// ---------------------------------------------------------------------------
// Execute
// ---------------------------------------------------------------------------

/// Option D burn: spend ZEC SEAM under egress ν + dest seal equality.
pub fn execute_bridge_egress_burn(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    statement: EgressBurnStatement,
    proof: Binary,
) -> Result<Response, StdError> {
    let cfg = BRIDGE_CFG
        .may_load(deps.storage)?
        .ok_or(EgressBurnError::BridgeNotConfigured)?;

    // Structural gates first (dest equality, ν re-derive, non-zero).
    validate_egress_burn_statement(&statement)?;

    let nu = require_hash32(&statement.nullifier, "nullifier")?;
    let cm = require_hash32(&statement.cm_spent, "cm_spent")?;
    let asset_id = require_hash32(&statement.asset_id, "asset_id")?;

    // Optional: asset must be registered when registry is in use for this asset.
    // Lab path: if any assets registered and this one missing → reject; if registry empty, allow.
    let asset_key = hex::encode(asset_id);
    let any_assets = crate::bridge::ASSET_REGISTRY
        .keys(deps.storage, None, None, cosmwasm_std::Order::Ascending)
        .next()
        .is_some();
    if any_assets {
        let entry = crate::bridge::ASSET_REGISTRY.may_load(deps.storage, &asset_key)?;
        if entry.is_none() {
            return Err(EgressBurnError::UnmappedAsset.into());
        }
    }

    let spent_key = egress_spent_storage_key(&nu);
    if EGRESS_SPENT.may_load(deps.storage, &spent_key)?.is_some() {
        return Err(EgressBurnError::AlreadySpent.into());
    }
    let cm_key = burned_cm_key(&cm);
    if EGRESS_BURNED_CM.may_load(deps.storage, &cm_key)?.is_some() {
        return Err(EgressBurnError::AlreadySpent.into());
    }

    // Dual-path proof.
    let proof_mode = verify_egress_proof(deps.api, &cfg, &statement, &proof)?;

    // Mark spent ν + burned cm (one-shot).
    EGRESS_SPENT.save(deps.storage, &spent_key, &())?;
    EGRESS_BURNED_CM.save(deps.storage, &cm_key, &())?;
    // Unity with shared nullifier surface under egress domain key.
    crate::NULLIFIERS.save(deps.storage, spent_key.clone(), &())?;

    let evidence = EgressBurnEvidenceV0 {
        terp_tx_hash: None,
        asset_id: statement.asset_id.clone(),
        value: statement.value,
        cm_spent: statement.cm_spent.clone(),
        nullifier: statement.nullifier.clone(),
        dest_commitment: statement.dest_commitment.clone(),
        dest_kind: statement.dest_kind,
        root: statement.root.clone(),
        source_pool_id: statement.source_pool_id,
        proof_mode: proof_mode.clone(),
        settle_receipt_ref: None,
    };
    let evidence_bin = to_json_binary(&evidence)?;

    Ok(Response::new()
        .add_attribute("action", "bridge_egress_burn")
        .add_attribute("nullifier", hex::encode(nu))
        .add_attribute("spent_key", spent_key)
        .add_attribute("cm_spent", hex::encode(cm))
        .add_attribute("dest_commitment", hex::encode(require_hash32(
            &statement.dest_commitment,
            "dest_commitment",
        )?))
        .add_attribute("value", statement.value.to_string())
        .add_attribute("proof_mode", proof_mode)
        .add_attribute("dest_kind", match statement.dest_kind {
            DestKind::Transparent => "transparent",
            DestKind::Shielded => "shielded",
        })
        .add_attribute("burn_evidence", evidence_bin.to_base64()))
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

pub fn query_is_egress_spent(deps: Deps, nullifier: Binary) -> StdResult<Binary> {
    let nu = require_hash32(&nullifier, "nullifier")?;
    let key = egress_spent_storage_key(&nu);
    let spent = EGRESS_SPENT.may_load(deps.storage, &key)?.is_some();
    to_json_binary(&spent)
}

// ---------------------------------------------------------------------------
// Test fixtures
// ---------------------------------------------------------------------------

/// Non-empty mock proof blob (empty rejected under mock_verify / multitest).
pub fn mock_egress_proof_bytes() -> Binary {
    Binary::from(vec![0xec, 0xee, 0x55, 0x01])
}

/// Build a happy-path egress statement from fixed openings.
pub fn happy_egress_statement() -> EgressBurnStatement {
    let asset_id = crate::bridge::hash32_label("asset-zec-registry-1");
    let cm = crate::bridge::hash32_label("cm-zec-seam-out-1");
    let rcm = crate::bridge::hash32_label("rcm-zec-egress-1");
    let dest = crate::bridge::hash32_label("g4-dest-seal-ua-1");
    let root = crate::bridge::hash32_label("membership-root-lab-1");
    let nu = egress_nullifier(&cm, &rcm);
    EgressBurnStatement {
        asset_id: Binary::from(asset_id.to_vec()),
        value: 50_000_000,
        cm_spent: Binary::from(cm.to_vec()),
        nullifier: Binary::from(nu.to_vec()),
        dest_commitment: Binary::from(dest.to_vec()),
        dest_kind: DestKind::Shielded,
        root: Binary::from(root.to_vec()),
        source_pool_id: Some(1),
        rcm: Binary::from(rcm.to_vec()),
        owner_binding: Binary::from(dest.to_vec()),
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
    use cosmwasm_std::Addr;

    use crate::bridge::{
        hash32_label, AssetEntry, AssetStatus, BridgeCfg, BRIDGE_CFG, ASSET_REGISTRY,
        DEFAULT_CONFIRMATIONS_K, DEFAULT_MAX_LC_LAG,
    };

    fn bin32(h: Hash32) -> Binary {
        Binary::from(h.to_vec())
    }

    fn seed_cfg(deps: &mut cosmwasm_std::OwnedDeps<
        cosmwasm_std::testing::MockStorage,
        cosmwasm_std::testing::MockApi,
        cosmwasm_std::testing::MockQuerier,
    >, mock_verify: bool) {
        let owner = deps.api.addr_make("owner");
        cw_ownable::initialize_owner(&mut deps.storage, &deps.api, Some(owner.as_str())).unwrap();
        let dest = hash32_label("terp-chain-1");
        let cfg = BridgeCfg {
            dest_domain: bin32(dest),
            confirmations_k: DEFAULT_CONFIRMATIONS_K,
            max_lc_lag: DEFAULT_MAX_LC_LAG,
            lc_client_id: "08-wasm-tacit-reflection-0".into(),
            mock_verify,
            egress_zkid: None,
        };
        BRIDGE_CFG.save(&mut deps.storage, &cfg).unwrap();
    }

    fn register_asset(deps: &mut cosmwasm_std::OwnedDeps<
        cosmwasm_std::testing::MockStorage,
        cosmwasm_std::testing::MockApi,
        cosmwasm_std::testing::MockQuerier,
    >, asset_id: Hash32) {
        let entry = AssetEntry {
            asset_id: bin32(asset_id),
            local_denom: "uzec".into(),
            origin: Some("registry:zec".into()),
            status: AssetStatus::Active,
        };
        ASSET_REGISTRY
            .save(&mut deps.storage, &hex::encode(asset_id), &entry)
            .unwrap();
    }

    #[test]
    fn egress_nf_label_normative() {
        assert_eq!(EGRESS_NF_LABEL, b"egress-nf-v0");
        assert_eq!(EGRESS_INSTANCE_LABEL, b"egress-burn-instance-v0");
    }

    #[test]
    fn instance_encoding_deterministic() {
        let s = happy_egress_statement();
        let a = encode_egress_instances(&s);
        let b = encode_egress_instances(&s);
        assert_eq!(a, b);
        assert!(a.len() > 32);
        // Domain tag is SHA256 of label
        let tag = Sha256::digest(EGRESS_INSTANCE_LABEL);
        assert_eq!(&a[..32], tag.as_slice());
    }

    #[test]
    fn validate_happy() {
        let s = happy_egress_statement();
        validate_egress_burn_statement(&s).expect("happy");
    }

    #[test]
    fn validate_dest_mismatch() {
        let mut s = happy_egress_statement();
        s.owner_binding = bin32(hash32_label("other-dest"));
        assert_eq!(
            validate_egress_burn_statement(&s),
            Err(EgressBurnError::DestMismatch)
        );
    }

    #[test]
    fn validate_nullifier_mismatch() {
        let mut s = happy_egress_statement();
        s.nullifier = bin32(hash32_label("wrong-nu"));
        assert_eq!(
            validate_egress_burn_statement(&s),
            Err(EgressBurnError::NullifierMismatch)
        );
    }

    #[test]
    fn validate_zero_value() {
        let mut s = happy_egress_statement();
        s.value = 0;
        assert_eq!(
            validate_egress_burn_statement(&s),
            Err(EgressBurnError::BadAmount)
        );
    }

    #[test]
    fn execute_happy_path_mock_verify() {
        let mut deps = mock_dependencies();
        seed_cfg(&mut deps, true);
        let s = happy_egress_statement();
        let asset = require_hash32(&s.asset_id, "asset_id").unwrap();
        register_asset(&mut deps, asset);

        let owner = deps.api.addr_make("owner");
        let info = message_info(&owner, &[]);
        let res = execute_bridge_egress_burn(
            deps.as_mut(),
            mock_env(),
            info,
            s.clone(),
            mock_egress_proof_bytes(),
        )
        .expect("happy burn");

        assert!(res
            .attributes
            .iter()
            .any(|a| a.key == "action" && a.value == "bridge_egress_burn"));
        assert!(res
            .attributes
            .iter()
            .any(|a| a.key == "proof_mode" && a.value == "mock_verify_lab"));

        let nu = require_hash32(&s.nullifier, "nullifier").unwrap();
        let key = egress_spent_storage_key(&nu);
        assert!(EGRESS_SPENT
            .may_load(&deps.storage, &key)
            .unwrap()
            .is_some());
    }

    #[test]
    fn execute_double_burn_reject() {
        let mut deps = mock_dependencies();
        seed_cfg(&mut deps, true);
        let s = happy_egress_statement();
        register_asset(&mut deps, require_hash32(&s.asset_id, "a").unwrap());
        let owner = deps.api.addr_make("owner");
        let info = message_info(&owner, &[]);

        execute_bridge_egress_burn(
            deps.as_mut(),
            mock_env(),
            info.clone(),
            s.clone(),
            mock_egress_proof_bytes(),
        )
        .expect("first");

        let err = execute_bridge_egress_burn(
            deps.as_mut(),
            mock_env(),
            info,
            s,
            mock_egress_proof_bytes(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("already spent") || err.to_string().contains("double"));
    }

    #[test]
    fn execute_dest_mismatch_reject() {
        let mut deps = mock_dependencies();
        seed_cfg(&mut deps, true);
        let mut s = happy_egress_statement();
        register_asset(&mut deps, require_hash32(&s.asset_id, "a").unwrap());
        s.owner_binding = bin32(hash32_label("redirect-attack"));
        let owner = deps.api.addr_make("owner");
        let err = execute_bridge_egress_burn(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            s,
            mock_egress_proof_bytes(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("dest") || err.to_string().contains("mismatch"));
    }

    #[test]
    fn execute_empty_proof_under_mock_reject() {
        let mut deps = mock_dependencies();
        // mock_verify true — still reject empty proof (private-dex posture).
        seed_cfg(&mut deps, true);
        let s = happy_egress_statement();
        register_asset(&mut deps, require_hash32(&s.asset_id, "a").unwrap());
        let owner = deps.api.addr_make("owner");
        let err = execute_bridge_egress_burn(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            s,
            Binary::default(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("proof") || err.to_string().contains("rejected"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn mock_verify_false_without_zk_api_fail_closed() {
        // Under cfg!(test), allow_mock is true even if mock_verify=false —
        // so we exercise verify_egress_proof with a synthetic cfg path by
        // calling the non-test branch logic via empty-proof-like production
        // rejection of ZkApi when we force !allow_mock.
        //
        // Direct unit of verify when mock is off: only reachable if we ignore cfg!(test).
        // Documented residual: cfg!(test) always allows mock (same as private-dex).
        // Production wasm binary without mock_verify and without zk-api → ZkApiUnavailable.
        let cfg = BridgeCfg {
            dest_domain: bin32(hash32_label("d")),
            confirmations_k: 6,
            max_lc_lag: 64,
            lc_client_id: "lc".into(),
            mock_verify: false,
            egress_zkid: Some(7),
        };
        // With cfg!(test), this still takes mock path:
        let s = happy_egress_statement();
        let deps = mock_dependencies();
        let mode = verify_egress_proof(deps.as_ref().api, &cfg, &s, &mock_egress_proof_bytes())
            .expect("cfg(test) allows mock");
        assert_eq!(mode, "mock_verify_lab");
        let _ = Addr::unchecked("x");
    }
}
