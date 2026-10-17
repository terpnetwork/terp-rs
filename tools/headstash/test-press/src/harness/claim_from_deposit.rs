//! Continuous deposit identity → bridge mint claim (G1 IDENTITY-CLAIM).
//!
//! Pure builder: observation + watch + lab policy → claim fields.
//! Nullifier domain is **provisional** (`terp-btc-deposit-nu-v0`) — not Tacit burn ν.
//! Membership flags under lab policy are labeled mock (not production IMT/LC).
//!
//! SSOT: `docs/plans/spectrum/agents/connect-private-swap-design-prep-2026-07-22/DESIGN-IDENTITY-CLAIM.md`

use sha2::{Digest, Sha256};
use thiserror::Error;

use terp_seams::bridge::{
    authorize_bridge_mint, derive_claim_id_with_dest, derive_domain_binding, label_hash,
    terp_asset_id_from_tacit, AssetRegistry, BridgeMintClaim, BridgeMintPublic, Hash32, MintedSet,
    ReflectionSnapshot, DEFAULT_CONFIRMATIONS_K, DEFAULT_MAX_LC_LAG,
};

/// Provisional corridor deposit nullifier domain (not Tacit LC burn ν).
/// Label: lab / continuous-identity until true BTC confidential burn ν exists.
pub const TERP_BTC_DEPOSIT_NU_V0: &[u8] = b"terp-btc-deposit-nu-v0";

/// Observation fields required by the claim builder.
#[derive(Clone, Debug)]
pub struct DepositObservationView {
    pub intent_id: String,
    pub btc_deposit_addr: String,
    /// 64-char hex (reporter / hub).
    pub txid: String,
    /// Required for continuous identity when `policy.require_vout`.
    /// `None` = missing (fail-closed under require_vout). `Some(0)` is a valid outpoint index.
    pub vout: Option<u32>,
    pub amount_sats: u64,
    pub confirmations: u32,
    pub domain_bind: Option<String>,
    pub reporter: String,
}

/// Watch / preauth fields required by the claim builder.
#[derive(Clone, Debug)]
pub struct DepositWatchView {
    pub intent_id: String,
    pub btc_deposit_addr: String,
    /// Domain C intent bind string (display / optional echo check only).
    /// **Not** claim `domain_binding` (Domain B — D2).
    pub domain_bind: String,
    /// 64-char hex (32B). Source of truth: G4 DEST-SEAL (golden/live).
    pub dest_owner_binding: String,
    pub min_amount_sats: u64,
    pub client_proof_digest: String,
}

/// Corridor lab policy for non-deposit claim fields.
/// `lab_mock_membership = true` ⇒ claim flags `in_burn_set`/`in_pool_root` true.
/// Does **not** imply production membership (D7).
#[derive(Clone, Debug)]
pub struct CorridorLabMintPolicy {
    pub source_chain_tag: String,
    pub tacit_asset_id: Hash32,
    /// Default 1; `value_u64 = amount_sats * unit_scale` (checked no overflow).
    pub unit_scale: u64,
    pub dest_domain: Hash32,
    pub pool_domain: Hash32,
    pub source_pool_root: Hash32,
    pub source_burn_root: Hash32,
    pub source_height: u64,
    pub src_chain_id: Hash32,
    pub dst_chain_id: Hash32,
    pub lc_client_id_hash: Hash32,
    /// true on ict_local_funded default — **labeled lab**, not production IMT.
    pub lab_mock_membership: bool,
    /// Client-held opening; non-zero → rcm_flag=1 on NoteOutResult.
    pub rcm: Option<Hash32>,
    /// Dest leaf / cm public (client constructs; lab may seed label).
    pub cm_public: Hash32,
    /// Builder-side conf floor (optional; 0 = skip). Independent of contract K.
    pub min_confirmations: u32,
    /// When true (default for deposit-backed path), missing vout is error.
    pub require_vout: bool,
    /// When true, reject known placeholder dest hex (G4 product list).
    pub reject_placeholder_dest: bool,
    /// When true and both obs/watch domain_bind present, require equality (echo only).
    pub check_domain_bind_echo: bool,
}

impl CorridorLabMintPolicy {
    /// Seed roots/labels aligned with happy hinge **except** ν/value/dest
    /// which come from deposit. Used when lab reflection is pre-set to match.
    pub fn corridor_lab_default() -> Self {
        let dest = label_hash("terp-chain-1");
        Self {
            // Lab default matches hinge fixture asset map pin (not mainnet money).
            source_chain_tag: "bitcoin-mainnet".into(),
            tacit_asset_id: label_hash("tacit-btc-etch-1"),
            unit_scale: 1,
            dest_domain: dest,
            pool_domain: label_hash("terp-pool-0"),
            source_pool_root: label_hash("pool-root-1"),
            source_burn_root: label_hash("burn-root-1"),
            source_height: 100,
            src_chain_id: label_hash("src-bitcoin-mainnet"),
            dst_chain_id: dest,
            lc_client_id_hash: label_hash("lc-client-reflection-0"),
            lab_mock_membership: true,
            rcm: None,
            cm_public: [0u8; 32], // filled after dest known if zero
            min_confirmations: 0,
            require_vout: true,
            reject_placeholder_dest: false,
            check_domain_bind_echo: false,
        }
    }

    /// Lab rcm: `sha256("rcm-corridor-deposit-v0" ‖ intent_id)`.
    pub fn lab_rcm_for_intent(intent_id: &str) -> Hash32 {
        let mut h = Sha256::new();
        h.update(b"rcm-corridor-deposit-v0");
        h.update(intent_id.as_bytes());
        let d = h.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&d);
        out
    }

    /// Lab cm_public: `sha256("cm-leaf-deposit-v0" ‖ dest_commitment)`.
    pub fn lab_cm_public_for_dest(dest: &Hash32) -> Hash32 {
        let mut h = Sha256::new();
        h.update(b"cm-leaf-deposit-v0");
        h.update(dest);
        let d = h.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&d);
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ClaimFromDepositError {
    #[error("missing txid")]
    MissingTxid,
    #[error("invalid txid hex")]
    InvalidTxidHex,
    #[error("missing vout (require_vout)")]
    MissingVout,
    #[error("missing amount (amount_sats == 0)")]
    MissingAmount,
    #[error("amount below watch min_amount_sats")]
    AmountBelowWatchMin,
    #[error("missing dest_owner_binding")]
    MissingDest,
    #[error("invalid dest_owner_binding hex")]
    InvalidDestHex,
    #[error("placeholder dest rejected")]
    PlaceholderDest,
    #[error("intent_id mismatch obs vs watch")]
    IntentMismatch,
    #[error("btc_deposit_addr mismatch obs vs watch")]
    AddrMismatch,
    #[error("confirmations below policy min")]
    ConfirmationsBelowMin,
    #[error("value scale overflow")]
    ValueScaleOverflow,
    #[error("domain_bind echo mismatch")]
    DomainBindEchoMismatch,
}

/// Pure claim fields (Hash32) — mirrors `BridgeMintClaimPublic` without CosmWasm Binary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeMintClaimPure {
    pub source_chain_tag: String,
    pub tacit_asset_id: Hash32,
    pub value_u64: u64,
    pub nullifier: Hash32,
    pub dest_commitment: Hash32,
    pub dest_domain: Hash32,
    pub claim_id: Hash32,
    pub source_pool_root: Hash32,
    pub source_burn_root: Hash32,
    pub source_height: u64,
    pub domain_binding: Hash32,
    pub unit_scale: u64,
    pub pool_domain: Hash32,
    pub cm_public: Hash32,
    pub rcm: Option<Hash32>,
    pub in_burn_set: bool,
    pub in_pool_root: bool,
    pub spent_only: bool,
    pub src_chain_id: Hash32,
    pub dst_chain_id: Hash32,
    pub lc_client_id: Hash32,
    pub burn_dest_commitment: Hash32,
}

impl BridgeMintClaimPure {
    /// Convert to pure-auth hinge claim for `authorize_bridge_mint`.
    pub fn to_auth_claim(&self) -> BridgeMintClaim {
        BridgeMintClaim {
            public: BridgeMintPublic {
                source_chain_tag: self.source_chain_tag.clone(),
                tacit_asset_id: self.tacit_asset_id,
                value_u64: self.value_u64,
                nullifier: self.nullifier,
                dest_commitment: self.dest_commitment,
                dest_domain: self.dest_domain,
                claim_id: self.claim_id,
                source_pool_root: self.source_pool_root,
                source_burn_root: self.source_burn_root,
                source_height: self.source_height,
                domain_binding: self.domain_binding,
                unit_scale: self.unit_scale,
                pool_domain: self.pool_domain,
                cm_public: self.cm_public,
                rcm: self.rcm.unwrap_or([0u8; 32]),
            },
            mint_value: self.value_u64,
            expected_dest_domain: self.dest_domain,
            src_chain_id: self.src_chain_id,
            dst_chain_id: self.dst_chain_id,
            lc_client_id: self.lc_client_id,
            burn_dest_commitment: self.burn_dest_commitment,
            in_burn_set: self.in_burn_set,
            in_pool_root: self.in_pool_root,
            spent_only: self.spent_only,
        }
    }

    pub fn nullifier_hex(&self) -> String {
        hex::encode(self.nullifier)
    }
}

/// Pure: provisional deposit nullifier.
///
/// `SHA256(TERP_BTC_DEPOSIT_NU_V0 ‖ txid[32] ‖ vout_be_u32 ‖ intent_id_utf8)`
pub fn deposit_nullifier_v0(txid: &Hash32, vout: u32, intent_id: &str) -> Hash32 {
    let mut h = Sha256::new();
    h.update(TERP_BTC_DEPOSIT_NU_V0);
    h.update(txid);
    h.update(vout.to_be_bytes());
    h.update(intent_id.as_bytes());
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

/// Parse 64-hex (optional 0x) → Hash32; fail-closed on length/charset.
pub fn parse_txid32(txid_hex: &str) -> Result<Hash32, ClaimFromDepositError> {
    parse_hash32_hex(txid_hex).map_err(|_| {
        if txid_hex.trim().is_empty() {
            ClaimFromDepositError::MissingTxid
        } else {
            ClaimFromDepositError::InvalidTxidHex
        }
    })
}

fn parse_hash32_hex(s: &str) -> Result<Hash32, ()> {
    let h = s.trim().strip_prefix("0x").unwrap_or(s.trim());
    if h.is_empty() {
        return Err(());
    }
    let bytes = hex::decode(h).map_err(|_| ())?;
    if bytes.len() != 32 {
        return Err(());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn is_placeholder_dest_hex(hex_s: &str) -> bool {
    let h = hex_s.trim().strip_prefix("0x").unwrap_or(hex_s.trim()).to_ascii_lowercase();
    if h.len() != 64 {
        return false;
    }
    h.chars().all(|c| c == '0')
        || h.chars().all(|c| c == 'b')
        || h.chars().all(|c| c == 'a')
        || h.chars().all(|c| c == 'c')
}

/// Continuous identity builder. Pure: no chain, no HTTP.
///
/// - nullifier ← deposit_nullifier_v0(txid, vout, intent_id)
/// - value_u64 ← amount_sats * unit_scale
/// - dest_commitment = burn_dest_commitment ← decode(watch.dest_owner_binding)
/// - claim_id ← derive_claim_id_with_dest(dest_domain, dest_cm, nu, tacit, value)
/// - domain_binding ← derive_domain_binding(src, dst, lc, tacit, nu, height, burn_root)
/// - membership flags ← policy.lab_mock_membership (**labeled lab**)
pub fn claim_from_deposit_watch(
    obs: &DepositObservationView,
    watch: &DepositWatchView,
    policy: &CorridorLabMintPolicy,
) -> Result<BridgeMintClaimPure, ClaimFromDepositError> {
    // Intent / addr gates
    if obs.intent_id != watch.intent_id {
        return Err(ClaimFromDepositError::IntentMismatch);
    }
    if obs.btc_deposit_addr != watch.btc_deposit_addr {
        return Err(ClaimFromDepositError::AddrMismatch);
    }
    if policy.check_domain_bind_echo {
        if let Some(ref db) = obs.domain_bind {
            if db != &watch.domain_bind {
                return Err(ClaimFromDepositError::DomainBindEchoMismatch);
            }
        }
    }

    // Txid
    let txid_trim = obs.txid.trim();
    if txid_trim.is_empty() {
        return Err(ClaimFromDepositError::MissingTxid);
    }
    let txid = parse_txid32(txid_trim)?;

    // Vout
    let vout = match obs.vout {
        Some(v) => v,
        None if policy.require_vout => return Err(ClaimFromDepositError::MissingVout),
        None => 0,
    };

    // Amount
    if obs.amount_sats == 0 {
        return Err(ClaimFromDepositError::MissingAmount);
    }
    if watch.min_amount_sats > 0 && obs.amount_sats < watch.min_amount_sats {
        return Err(ClaimFromDepositError::AmountBelowWatchMin);
    }
    if policy.min_confirmations > 0 && obs.confirmations < policy.min_confirmations {
        return Err(ClaimFromDepositError::ConfirmationsBelowMin);
    }
    let value_u64 = obs
        .amount_sats
        .checked_mul(policy.unit_scale)
        .ok_or(ClaimFromDepositError::ValueScaleOverflow)?;

    // Dest (G4 source; copy bytes only — do not re-hash)
    let dest_hex = watch.dest_owner_binding.trim();
    if dest_hex.is_empty() {
        return Err(ClaimFromDepositError::MissingDest);
    }
    if policy.reject_placeholder_dest && is_placeholder_dest_hex(dest_hex) {
        return Err(ClaimFromDepositError::PlaceholderDest);
    }
    let dest_cm = parse_hash32_hex(dest_hex).map_err(|_| ClaimFromDepositError::InvalidDestHex)?;

    let nu = deposit_nullifier_v0(&txid, vout, &obs.intent_id);
    let claim_id = derive_claim_id_with_dest(
        &policy.dest_domain,
        &dest_cm,
        &nu,
        &policy.tacit_asset_id,
        value_u64,
    );
    let domain_binding = derive_domain_binding(
        &policy.src_chain_id,
        &policy.dst_chain_id,
        &policy.lc_client_id_hash,
        &policy.tacit_asset_id,
        &nu,
        policy.source_height,
        &policy.source_burn_root,
    );

    let rcm = policy
        .rcm
        .unwrap_or_else(|| CorridorLabMintPolicy::lab_rcm_for_intent(&obs.intent_id));
    let cm_public = if policy.cm_public != [0u8; 32] {
        policy.cm_public
    } else {
        CorridorLabMintPolicy::lab_cm_public_for_dest(&dest_cm)
    };

    let (in_burn_set, in_pool_root, spent_only) = if policy.lab_mock_membership {
        (true, true, false)
    } else {
        (false, false, false)
    };

    Ok(BridgeMintClaimPure {
        source_chain_tag: policy.source_chain_tag.clone(),
        tacit_asset_id: policy.tacit_asset_id,
        value_u64,
        nullifier: nu,
        dest_commitment: dest_cm,
        dest_domain: policy.dest_domain,
        claim_id,
        source_pool_root: policy.source_pool_root,
        source_burn_root: policy.source_burn_root,
        source_height: policy.source_height,
        domain_binding,
        unit_scale: policy.unit_scale,
        pool_domain: policy.pool_domain,
        cm_public,
        rcm: Some(rcm),
        in_burn_set,
        in_pool_root,
        spent_only,
        src_chain_id: policy.src_chain_id,
        dst_chain_id: policy.dst_chain_id,
        lc_client_id: policy.lc_client_id_hash,
        burn_dest_commitment: dest_cm,
    })
}

/// Snapshot + registry pin matching [`CorridorLabMintPolicy::corridor_lab_default`].
pub fn lab_snapshot_for_policy(policy: &CorridorLabMintPolicy) -> ReflectionSnapshot {
    ReflectionSnapshot {
        pool_root: policy.source_pool_root,
        spent_root: label_hash("spent-root-1"),
        burn_root: policy.source_burn_root,
        source_height: policy.source_height,
        tip_height: policy.source_height + DEFAULT_CONFIRMATIONS_K,
        confirmations_k: DEFAULT_CONFIRMATIONS_K,
        max_lc_lag: DEFAULT_MAX_LC_LAG,
        frozen: false,
    }
}

/// Terp asset id for policy (must match contract re-derive with tacit_asset_id).
pub fn lab_terp_asset_for_policy(policy: &CorridorLabMintPolicy) -> Hash32 {
    terp_asset_id_from_tacit(
        &policy.source_chain_tag,
        &policy.tacit_asset_id,
        policy.unit_scale,
    )
}

/// Env: true when harness may fall back to hinge happy fixture.
pub fn corridor_allow_happy_fixture() -> bool {
    matches!(
        std::env::var("CORRIDOR_ALLOW_HAPPY_FIXTURE").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    )
}

/// Resolve deposit-backed claim from env views when present.
///
/// Env (lab):
/// - `CORRIDOR_INTENT_ID`, `CORRIDOR_DEPOSIT_TXID`, `CORRIDOR_DEPOSIT_AMOUNT_SATS`
/// - `CORRIDOR_DEPOSIT_VOUT` (optional; missing → fail if require_vout)
/// - `CORRIDOR_DEPOSIT_ADDR`, `CORRIDOR_DEST_OWNER_BINDING`
/// - `CORRIDOR_WATCH_MIN_AMOUNT_SATS` (optional, default 0)
/// - `CORRIDOR_DOMAIN_BIND` (optional Domain C echo only)
///
/// Returns `None` if deposit identity env is incomplete (caller may happy-fixture
/// only when [`corridor_allow_happy_fixture`]).
pub fn try_claim_from_env(
    policy: &CorridorLabMintPolicy,
) -> Result<Option<BridgeMintClaimPure>, ClaimFromDepositError> {
    let intent = match std::env::var("CORRIDOR_INTENT_ID") {
        Ok(s) if !s.trim().is_empty() => s,
        _ => return Ok(None),
    };
    let txid = match std::env::var("CORRIDOR_DEPOSIT_TXID") {
        Ok(s) if !s.trim().is_empty() => s,
        _ => return Ok(None),
    };
    let amount_sats = match std::env::var("CORRIDOR_DEPOSIT_AMOUNT_SATS") {
        Ok(s) => match s.parse::<u64>() {
            Ok(v) if v > 0 => v,
            _ => return Ok(None),
        },
        Err(_) => return Ok(None),
    };
    let dest = match std::env::var("CORRIDOR_DEST_OWNER_BINDING") {
        Ok(s) if !s.trim().is_empty() => s,
        _ => return Ok(None),
    };
    let addr = std::env::var("CORRIDOR_DEPOSIT_ADDR").unwrap_or_else(|_| "lab-deposit-addr".into());
    let vout = match std::env::var("CORRIDOR_DEPOSIT_VOUT") {
        Ok(s) if !s.trim().is_empty() => Some(s.parse::<u32>().map_err(|_| ClaimFromDepositError::MissingVout)?),
        _ => None,
    };
    let min_amount = std::env::var("CORRIDOR_WATCH_MIN_AMOUNT_SATS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let domain_bind = std::env::var("CORRIDOR_DOMAIN_BIND").unwrap_or_default();
    let confs = std::env::var("CORRIDOR_DEPOSIT_CONFS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    let obs = DepositObservationView {
        intent_id: intent.clone(),
        btc_deposit_addr: addr.clone(),
        txid,
        vout,
        amount_sats,
        confirmations: confs,
        domain_bind: if domain_bind.is_empty() {
            None
        } else {
            Some(domain_bind.clone())
        },
        reporter: "env".into(),
    };
    let watch = DepositWatchView {
        intent_id: intent,
        btc_deposit_addr: addr,
        domain_bind,
        dest_owner_binding: dest,
        min_amount_sats: min_amount,
        client_proof_digest: "lab".into(),
    };
    Ok(Some(claim_from_deposit_watch(&obs, &watch, policy)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_txid_hex() -> String {
        // 32 raw bytes as hex (not reversed)
        "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff".into()
    }

    fn sample_dest_hex() -> String {
        // G4 golden primary owner_binding
        "8b5cac11e39905d56126a0c538b84ff8daa379d8009d4e8b121112479607f09b".into()
    }

    fn sample_obs() -> DepositObservationView {
        DepositObservationView {
            intent_id: "ict-demo-001".into(),
            btc_deposit_addr: "bcrt1qtest".into(),
            txid: sample_txid_hex(),
            vout: Some(0),
            amount_sats: 20_000,
            confirmations: 1,
            domain_bind: Some("domain-c-echo".into()),
            reporter: "lab".into(),
        }
    }

    fn sample_watch() -> DepositWatchView {
        DepositWatchView {
            intent_id: "ict-demo-001".into(),
            btc_deposit_addr: "bcrt1qtest".into(),
            domain_bind: "domain-c-echo".into(),
            dest_owner_binding: sample_dest_hex(),
            min_amount_sats: 10_000,
            client_proof_digest: "c".repeat(64),
        }
    }

    #[test]
    fn t2_domain_tag_exact_bytes() {
        assert_eq!(TERP_BTC_DEPOSIT_NU_V0, b"terp-btc-deposit-nu-v0");
        assert_eq!(TERP_BTC_DEPOSIT_NU_V0.len(), b"terp-btc-deposit-nu-v0".len());
    }

    #[test]
    fn t1_deposit_nullifier_v0_deterministic_and_sensitive() {
        let txid = parse_txid32(&sample_txid_hex()).unwrap();
        let a = deposit_nullifier_v0(&txid, 0, "ict-demo-001");
        let b = deposit_nullifier_v0(&txid, 0, "ict-demo-001");
        assert_eq!(a, b);
        // golden fixed hex for regression
        let expect = {
            let mut h = Sha256::new();
            h.update(b"terp-btc-deposit-nu-v0");
            h.update(txid);
            h.update(0u32.to_be_bytes());
            h.update(b"ict-demo-001");
            let d = h.finalize();
            let mut out = [0u8; 32];
            out.copy_from_slice(&d);
            out
        };
        assert_eq!(a, expect);
        assert_ne!(deposit_nullifier_v0(&txid, 1, "ict-demo-001"), a);
        assert_ne!(deposit_nullifier_v0(&txid, 0, "ict-demo-002"), a);
        let mut tx2 = txid;
        tx2[0] ^= 1;
        assert_ne!(deposit_nullifier_v0(&tx2, 0, "ict-demo-001"), a);
    }

    #[test]
    fn t3_claim_from_deposit_happy_authorize() {
        let obs = sample_obs();
        let watch = sample_watch();
        let policy = CorridorLabMintPolicy::corridor_lab_default();
        let claim = claim_from_deposit_watch(&obs, &watch, &policy).expect("claim");
        assert_eq!(claim.value_u64, 20_000);
        assert_eq!(claim.dest_commitment, parse_hash32_hex(&sample_dest_hex()).unwrap());
        assert_eq!(claim.burn_dest_commitment, claim.dest_commitment);
        assert_eq!(
            claim.nullifier,
            deposit_nullifier_v0(
                &parse_txid32(&sample_txid_hex()).unwrap(),
                0,
                "ict-demo-001"
            )
        );
        assert!(claim.in_burn_set && claim.in_pool_root && !claim.spent_only);

        // re-derive claim_id / domain_binding
        let expect_cid = derive_claim_id_with_dest(
            &policy.dest_domain,
            &claim.dest_commitment,
            &claim.nullifier,
            &policy.tacit_asset_id,
            claim.value_u64,
        );
        assert_eq!(claim.claim_id, expect_cid);
        let expect_db = derive_domain_binding(
            &policy.src_chain_id,
            &policy.dst_chain_id,
            &policy.lc_client_id_hash,
            &policy.tacit_asset_id,
            &claim.nullifier,
            policy.source_height,
            &policy.source_burn_root,
        );
        assert_eq!(claim.domain_binding, expect_db);

        // Domain C domain_bind must NOT equal Domain B domain_binding as source
        // (builder re-derived Domain B independently)
        assert_ne!(
            hex::encode(claim.domain_binding),
            watch.domain_bind,
            "must not paste Domain C into Domain B"
        );

        let snap = lab_snapshot_for_policy(&policy);
        let auth = claim.to_auth_claim();
        let mut registry = AssetRegistry::new();
        registry.register(lab_terp_asset_for_policy(&policy));
        let minted = MintedSet::new();
        authorize_bridge_mint(&snap, &auth, &minted, &registry).expect("authorize");
    }

    #[test]
    fn t4_missing_txid() {
        let mut obs = sample_obs();
        obs.txid = "".into();
        let err = claim_from_deposit_watch(&obs, &sample_watch(), &CorridorLabMintPolicy::corridor_lab_default())
            .unwrap_err();
        assert_eq!(err, ClaimFromDepositError::MissingTxid);
    }

    #[test]
    fn t4_invalid_txid_hex() {
        let mut obs = sample_obs();
        obs.txid = "deadbeef".into();
        let err = claim_from_deposit_watch(&obs, &sample_watch(), &CorridorLabMintPolicy::corridor_lab_default())
            .unwrap_err();
        assert_eq!(err, ClaimFromDepositError::InvalidTxidHex);
    }

    #[test]
    fn t5_missing_amount() {
        let mut obs = sample_obs();
        obs.amount_sats = 0;
        let err = claim_from_deposit_watch(&obs, &sample_watch(), &CorridorLabMintPolicy::corridor_lab_default())
            .unwrap_err();
        assert_eq!(err, ClaimFromDepositError::MissingAmount);
    }

    #[test]
    fn t6_missing_dest() {
        let mut watch = sample_watch();
        watch.dest_owner_binding = "".into();
        let err = claim_from_deposit_watch(&sample_obs(), &watch, &CorridorLabMintPolicy::corridor_lab_default())
            .unwrap_err();
        assert_eq!(err, ClaimFromDepositError::MissingDest);
    }

    #[test]
    fn t6_invalid_dest_hex() {
        let mut watch = sample_watch();
        watch.dest_owner_binding = "zz".into();
        let err = claim_from_deposit_watch(&sample_obs(), &watch, &CorridorLabMintPolicy::corridor_lab_default())
            .unwrap_err();
        assert_eq!(err, ClaimFromDepositError::InvalidDestHex);
    }

    #[test]
    fn t7_amount_below_watch_min() {
        let mut obs = sample_obs();
        obs.amount_sats = 100;
        let err = claim_from_deposit_watch(&obs, &sample_watch(), &CorridorLabMintPolicy::corridor_lab_default())
            .unwrap_err();
        assert_eq!(err, ClaimFromDepositError::AmountBelowWatchMin);
    }

    #[test]
    fn t8_intent_mismatch() {
        let mut obs = sample_obs();
        obs.intent_id = "other".into();
        let err = claim_from_deposit_watch(&obs, &sample_watch(), &CorridorLabMintPolicy::corridor_lab_default())
            .unwrap_err();
        assert_eq!(err, ClaimFromDepositError::IntentMismatch);
    }

    #[test]
    fn t8_addr_mismatch() {
        let mut obs = sample_obs();
        obs.btc_deposit_addr = "other".into();
        let err = claim_from_deposit_watch(&obs, &sample_watch(), &CorridorLabMintPolicy::corridor_lab_default())
            .unwrap_err();
        assert_eq!(err, ClaimFromDepositError::AddrMismatch);
    }

    #[test]
    fn t9_require_vout_missing() {
        let mut obs = sample_obs();
        obs.vout = None;
        let policy = CorridorLabMintPolicy::corridor_lab_default();
        assert!(policy.require_vout);
        let err = claim_from_deposit_watch(&obs, &sample_watch(), &policy).unwrap_err();
        assert_eq!(err, ClaimFromDepositError::MissingVout);
    }

    #[test]
    fn t9_vout_zero_allowed_when_explicit() {
        let mut obs = sample_obs();
        obs.vout = Some(0);
        claim_from_deposit_watch(
            &obs,
            &sample_watch(),
            &CorridorLabMintPolicy::corridor_lab_default(),
        )
        .expect("vout 0 is valid UTXO index");
    }

    #[test]
    fn t10_same_inputs_same_nu() {
        let policy = CorridorLabMintPolicy::corridor_lab_default();
        let c1 = claim_from_deposit_watch(&sample_obs(), &sample_watch(), &policy).unwrap();
        let c2 = claim_from_deposit_watch(&sample_obs(), &sample_watch(), &policy).unwrap();
        assert_eq!(c1.nullifier, c2.nullifier);
        assert_eq!(c1.claim_id, c2.claim_id);
    }

    #[test]
    fn parse_txid_accepts_0x_prefix() {
        let h = format!("0x{}", sample_txid_hex());
        let a = parse_txid32(&h).unwrap();
        let b = parse_txid32(&sample_txid_hex()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn placeholder_dest_optional_reject() {
        let mut policy = CorridorLabMintPolicy::corridor_lab_default();
        policy.reject_placeholder_dest = true;
        let mut watch = sample_watch();
        watch.dest_owner_binding = "b".repeat(64);
        let err = claim_from_deposit_watch(&sample_obs(), &watch, &policy).unwrap_err();
        assert_eq!(err, ClaimFromDepositError::PlaceholderDest);
    }
}
