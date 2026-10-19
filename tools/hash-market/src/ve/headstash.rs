//! Headstash claim routes on the vote-extension server.
//!
//! `/headstash/:id` stays the snap note server. These routes live under
//! `/ve/headstash/`. The seal body does not carry a key.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use axum::{Json, extract::State};
use ed25519_dalek::SigningKey;
use headstash_epoch::{
    ALGO_ACCEPTED, ALGO_PENDING, BATCH_ATTEST_TIMESTAMP, BATCH_SOURCE_ID, Backend, CHAIN_UID,
    Campaign, CampaignParams, DEPTH, IngressError, PeerReceipt, SealedVote, SubmittedClaim,
    halo2_vk_hash,
};
use serde::Deserialize;

use crate::msg::VoteExtensionHashData;
use crate::server::AppState;

use super::server::{ProviderData, ProviderKey};

/// Campaign plus the last sealed vote. The OracleSource key sits beside
/// them and is not part of the campaign.
pub struct HeadstashHost {
    campaign: Option<Campaign<DEPTH>>,
    sealed: Option<SealedVote>,
    signer: Option<SigningKey>,
}

impl HeadstashHost {
    pub fn new() -> Self {
        Self {
            campaign: None,
            sealed: None,
            signer: None,
        }
    }

    pub fn install_signer(&mut self, key: SigningKey) {
        self.signer = Some(key);
    }

    /// In-memory only. The loopback driver uses this between installs.
    /// It is not an HTTP route.
    pub fn clear_campaign(&mut self) {
        self.campaign = None;
        self.sealed = None;
    }

    pub fn sealed(&self) -> Option<&SealedVote> {
        self.sealed.as_ref()
    }
}

impl Default for HeadstashHost {
    fn default() -> Self {
        Self::new()
    }
}

pub fn env_chain_is_headstash() -> bool {
    std::env::var("HASHMERCHANT_CHAIN_UID").ok().as_deref() == Some(CHAIN_UID)
}

/// JSON `GET /vote-extension` for a sealed headstash vote.
/// One attestation. The env fixture is not an input.
pub fn vote_json(sealed: &SealedVote) -> serde_json::Value {
    let attestation = serde_json::json!({
        "source_id": BATCH_SOURCE_ID,
        "value": hex::encode(sealed.attestation_value()),
        "height": sealed.attestation_height,
        "timestamp": sealed.attestation_timestamp,
        "custody_signature": hex::encode(sealed.custody_signature),
    });
    serde_json::json!({
        "runtime_id": "headstash",
        "chain_uid": CHAIN_UID,
        "algo": sealed.algo,
        "root": hex::encode(sealed.attested),
        "foreign_height": sealed.foreign_height,
        "foreign_block_time": sealed.attestation_timestamp,
        "attestations": [attestation.clone()],
        "present_attestations": [attestation],
        "present_reversed": false,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignBody {
    pub backend: String,
    pub ctx_hex: String,
    pub allocation_root_hex: String,
    pub bech32_hrp: String,
    pub denom_hex: String,
    pub proof_len: usize,
    pub cutoff_height: u64,
    pub amounts: Vec<AmountBody>,
    pub trusted_hex: Vec<String>,
    pub trusted_verify_threshold: usize,
    pub trusted_sign_threshold: usize,
    pub max_rows: usize,
    #[serde(default)]
    pub reserved_hex: Vec<String>,
    /// Halo2 registry id. Required to be non-zero when `backend` is `halo2`.
    /// Flock may omit it.
    #[serde(default)]
    pub vk_id: u64,
    /// 64 lowercase hex characters. SHA-256 of the whole circuit blob.
    /// Flock may omit it.
    #[serde(default)]
    pub vk_hash_hex: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AmountBody {
    pub amount: u64,
    pub cap: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimsBody {
    pub claims: Vec<ClaimBody>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimBody {
    pub proof_hex: String,
    pub message: String,
    pub signature_hex: String,
    pub leaf_x_hex: String,
    pub denom_hex: String,
    #[serde(default)]
    pub index: Option<u32>,
    #[serde(default)]
    pub receipts: Vec<ReceiptBody>,
}

/// Halo2 submissions have no signature and no leaf key. Unknown fields are rejected.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Halo2ClaimsBody {
    claims: Vec<Halo2ClaimBody>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Halo2ClaimBody {
    proof_hex: String,
    message: String,
    denom_hex: String,
    vk_id: u64,
    #[serde(default)]
    index: Option<u32>,
    #[serde(default)]
    receipts: Vec<ReceiptBody>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptBody {
    pub signer_hex: String,
    pub signature_hex: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SealBody {
    pub height: u64,
}

type HttpError = (StatusCode, Json<serde_json::Value>);

pub async fn install_campaign(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, HttpError> {
    require_operator(&state, "POST", CAMPAIGN_PATH, &headers, &body)?;
    let body: CampaignBody = serde_json::from_slice(&body)
        .map_err(|_| error_json_msg(StatusCode::BAD_REQUEST, "campaign body"))?;
    let params = params_from(&body)?;
    if params.backend == Backend::Halo2 && params.vk_hash == [0u8; 32] {
        return Err(error_json_msg(StatusCode::BAD_REQUEST, "vk hash"));
    }
    if params.backend == Backend::Halo2 {
        match halo2_vk_hash(params.vk_id) {
            Some(loaded) if loaded == params.vk_hash => {}
            Some(_) => {
                return Err(error_json_msg(StatusCode::BAD_REQUEST, "vk hash"));
            }
            None => {
                return Err(error_json_msg(StatusCode::BAD_REQUEST, "vk is not loaded"));
            }
        }
    }
    let campaign = Campaign::<DEPTH>::open(params).map_err(error_json)?;
    let mut host = state.headstash.write().await;
    if host.campaign.is_some() {
        return Err(error_json_msg(
            StatusCode::CONFLICT,
            "campaign already exists",
        ));
    }
    host.campaign = Some(campaign);
    host.sealed = None;
    Ok(Json(serde_json::json!({"status": "ok"})))
}

pub async fn submit_claims(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, HttpError> {
    let mut host = state.headstash.write().await;
    let backend = host.campaign.as_ref().map(|campaign| campaign.backend());
    let Some(backend) = backend else {
        return Err(error_json_msg(
            StatusCode::SERVICE_UNAVAILABLE,
            "no campaign",
        ));
    };
    let items = match backend {
        Backend::Flock => {
            let parsed: ClaimsBody = serde_json::from_value(body).map_err(|_| {
                error_json_msg(StatusCode::BAD_REQUEST, "claim body")
            })?;
            claims_from(&parsed)?
        }
        Backend::Halo2 => {
            let parsed: Halo2ClaimsBody = serde_json::from_value(body).map_err(|_| {
                error_json_msg(StatusCode::BAD_REQUEST, "claim body")
            })?;
            claims_from_halo2(&parsed)?
        }
    };
    let signer = host.signer.take();
    let Some(campaign) = host.campaign.as_mut() else {
        host.signer = signer;
        return Err(error_json_msg(
            StatusCode::SERVICE_UNAVAILABLE,
            "no campaign",
        ));
    };
    let report = campaign.accept_batch(&items, signer.as_ref());
    host.signer = signer;
    let rejected: Vec<_> = report
        .rejected
        .iter()
        .map(|(index, err)| serde_json::json!({"index": index, "error": err.to_string()}))
        .collect();
    Ok(Json(serde_json::json!({
        "accepted": report.accepted,
        "rejected": rejected,
    })))
}

pub async fn seal(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, HttpError> {
    require_operator(&state, "POST", SEAL_PATH, &headers, &body)?;
    let body: SealBody = serde_json::from_slice(&body)
        .map_err(|_| error_json_msg(StatusCode::BAD_REQUEST, "seal body"))?;
    let mut host = state.headstash.write().await;
    let Some(campaign) = host.campaign.as_ref() else {
        return Err(error_json_msg(
            StatusCode::SERVICE_UNAVAILABLE,
            "no campaign",
        ));
    };
    let Some(signer) = host.signer.as_ref() else {
        return Err(error_json_msg(
            StatusCode::SERVICE_UNAVAILABLE,
            "no custody key",
        ));
    };
    let mut sealed = campaign.seal(body.height, signer);
    if lab_lie_root() {
        sealed.batch_hash[0] ^= 0x5a;
        let value = sealed.attestation_value();
        sealed.attested = headstash_epoch::aggregate_root(&value);
        sealed.custody_signature = ed25519_dalek::Signer::sign(
            signer,
            &headstash_epoch::custody_message(&value),
        )
        .to_bytes();
    }
    let json = vote_json(&sealed);
    let algo = sealed.algo.clone();
    let root = sealed.attested.to_vec();
    let foreign_height = sealed.foreign_height;
    host.sealed = Some(sealed);
    drop(host);

    let data = VoteExtensionHashData {
        runtime_id: "headstash".into(),
        chain_uid: CHAIN_UID.into(),
        algo: algo.clone(),
        root,
        foreign_height,
        foreign_block_time: BATCH_ATTEST_TIMESTAMP,
        ics23_proof: Vec::new(),
    };
    let mut store = state.provider_data.write().await;
    for old in [ALGO_PENDING, ALGO_ACCEPTED] {
        store.remove(&ProviderKey {
            chain_uid: CHAIN_UID.into(),
            algo: old.into(),
        });
    }
    store.insert(
        ProviderKey {
            chain_uid: CHAIN_UID.into(),
            algo,
        },
        ProviderData {
            data,
            received_at: now_secs(),
        },
    );
    Ok(Json(json))
}

fn params_from(body: &CampaignBody) -> Result<CampaignParams, HttpError> {
    let backend = match body.backend.as_str() {
        "flock" => Backend::Flock,
        "halo2" => Backend::Halo2,
        _ => return Err(error_json_msg(StatusCode::BAD_REQUEST, "unknown backend")),
    };
    let mut amounts = BTreeMap::new();
    for class in &body.amounts {
        if amounts.insert(class.amount, class.cap).is_some() {
            return Err(error_json_msg(StatusCode::BAD_REQUEST, "duplicate amount"));
        }
    }
    let (vk_id, vk_hash) = vk_from_body(body, backend)?;
    Ok(CampaignParams {
        backend,
        ctx: decode_32(&body.ctx_hex)?,
        allocation_root: decode_32(&body.allocation_root_hex)?,
        bech32_hrp: body.bech32_hrp.clone(),
        denom: decode_32(&body.denom_hex)?,
        proof_len: body.proof_len,
        cutoff_height: body.cutoff_height,
        amounts,
        trusted: body
            .trusted_hex
            .iter()
            .map(|hex| decode_32(hex))
            .collect::<Result<Vec<_>, _>>()?,
        trusted_verify_threshold: body.trusted_verify_threshold,
        trusted_sign_threshold: body.trusted_sign_threshold,
        max_rows: body.max_rows,
        reserved: body
            .reserved_hex
            .iter()
            .map(|hex| decode_32(hex))
            .collect::<Result<Vec<_>, _>>()?,
        vk_id,
        vk_hash,
    })
}

fn vk_from_body(body: &CampaignBody, backend: Backend) -> Result<(u64, [u8; 32]), HttpError> {
    match backend {
        Backend::Halo2 => {
            if body.vk_id == 0 {
                return Err(error_json_msg(StatusCode::BAD_REQUEST, "vk id"));
            }
            if body.vk_hash_hex.len() != 64 {
                return Err(error_json_msg(StatusCode::BAD_REQUEST, "vk hash"));
            }
            let hash = decode_32(&body.vk_hash_hex)?;
            if hash == [0u8; 32] {
                return Err(error_json_msg(StatusCode::BAD_REQUEST, "vk hash"));
            }
            Ok((body.vk_id, hash))
        }
        Backend::Flock => {
            if body.vk_hash_hex.is_empty() {
                Ok((body.vk_id, [0u8; 32]))
            } else {
                Ok((body.vk_id, decode_32(&body.vk_hash_hex)?))
            }
        }
    }
}

fn claims_from(body: &ClaimsBody) -> Result<Vec<SubmittedClaim>, HttpError> {
    let mut out = Vec::with_capacity(body.claims.len());
    for claim in &body.claims {
        let mut receipts = Vec::with_capacity(claim.receipts.len());
        for receipt in &claim.receipts {
            let signature = decode_hex(&receipt.signature_hex)?;
            let signature: [u8; 64] = signature
                .try_into()
                .map_err(|_| error_json_msg(StatusCode::BAD_REQUEST, "receipt signature"))?;
            receipts.push(PeerReceipt {
                signer: decode_32(&receipt.signer_hex)?,
                signature,
            });
        }
        out.push(SubmittedClaim {
            proof: decode_hex(&claim.proof_hex)?,
            message: claim.message.as_bytes().to_vec(),
            signature: decode_hex(&claim.signature_hex)?,
            leaf_x: decode_32(&claim.leaf_x_hex)?,
            denom: decode_32(&claim.denom_hex)?,
            index: claim.index,
            receipts,
            vk_id: 0,
        });
    }
    Ok(out)
}

fn claims_from_halo2(body: &Halo2ClaimsBody) -> Result<Vec<SubmittedClaim>, HttpError> {
    let mut out = Vec::with_capacity(body.claims.len());
    for claim in &body.claims {
        let mut receipts = Vec::with_capacity(claim.receipts.len());
        for receipt in &claim.receipts {
            let signature = decode_hex(&receipt.signature_hex)?;
            let signature: [u8; 64] = signature
                .try_into()
                .map_err(|_| error_json_msg(StatusCode::BAD_REQUEST, "receipt signature"))?;
            receipts.push(PeerReceipt {
                signer: decode_32(&receipt.signer_hex)?,
                signature,
            });
        }
        out.push(SubmittedClaim {
            proof: decode_hex(&claim.proof_hex)?,
            message: claim.message.as_bytes().to_vec(),
            signature: Vec::new(),
            leaf_x: [0u8; 32],
            denom: decode_32(&claim.denom_hex)?,
            index: claim.index,
            receipts,
            vk_id: claim.vk_id,
        });
    }
    Ok(out)
}

fn decode_32(hex_str: &str) -> Result<[u8; 32], HttpError> {
    let bytes = decode_hex(hex_str)?;
    bytes
        .try_into()
        .map_err(|_| error_json_msg(StatusCode::BAD_REQUEST, "expected 32 bytes"))
}

fn decode_hex(hex_str: &str) -> Result<Vec<u8>, HttpError> {
    hex::decode(hex_str.trim()).map_err(|_| error_json_msg(StatusCode::BAD_REQUEST, "bad hex"))
}

fn error_json(err: IngressError) -> HttpError {
    error_json_msg(StatusCode::BAD_REQUEST, &err.to_string())
}

fn error_json_msg(code: StatusCode, msg: &str) -> HttpError {
    (code, Json(serde_json::json!({"error": msg})))
}

/// Lab hook for the wrong-root failure mode. Both variables must be
/// exactly `1`. A production server leaves them unset.
fn lab_lie_root() -> bool {
    lab_lie_root_from(
        std::env::var("HEADSTASH_LIE_ROOT").ok().as_deref(),
        std::env::var("HEADSTASH_LAB_LIE").ok().as_deref(),
    )
}

fn lab_lie_root_from(lie: Option<&str>, lab: Option<&str>) -> bool {
    lie == Some("1") && lab == Some("1")
}

const CAMPAIGN_PATH: &str = "/ve/headstash/campaign";
const SEAL_PATH: &str = "/ve/headstash/seal";

fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .filter(|text| !text.is_empty())
}

fn require_operator(
    state: &AppState,
    method: &str,
    path: &str,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<(), HttpError> {
    let pubkey = header_value(headers, "x-hash-market-pubkey").unwrap_or("");
    let signature = header_value(headers, "x-hash-market-signature").unwrap_or("");
    let expiry = header_value(headers, "x-hash-market-expiry");
    let nonce = header_value(headers, "x-hash-market-nonce");
    crate::tree_operator::authorize_operator(
        &state.tree_operator,
        &state.operator_pubkeys,
        method,
        path,
        body,
        pubkey,
        signature,
        expiry,
        nonce,
        now_secs(),
    )
    .map_err(|deny| match deny {
        crate::tree_operator::TreeWriteDeny::Forbidden => {
            error_json_msg(StatusCode::FORBIDDEN, "operator")
        }
        crate::tree_operator::TreeWriteDeny::Unauthorized => {
            error_json_msg(StatusCode::UNAUTHORIZED, "operator")
        }
    })?;
    if !state.remember_admin(signature, nonce) {
        return Err(error_json_msg(StatusCode::UNAUTHORIZED, "operator"));
    }
    Ok(())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::vote_json;
    use std::sync::Arc;
    use axum::body::Bytes;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use headstash_epoch::{
        ALGO_PENDING, BATCH_ATTEST_HEIGHT, BATCH_ATTEST_TIMESTAMP, BATCH_SOURCE_ID, CHAIN_UID,
        SealedVote,
    };

    #[test]
    fn vote_json_is_one_headstash_batch_attestation() {
        let sealed = SealedVote {
            algo: ALGO_PENDING.into(),
            batch_hash: [2u8; 32],
            vk_hash: [0u8; 32],
            attested: [3u8; 32],
            custody_signature: [4u8; 64],
            custody_public: [5u8; 32],
            attestation_height: BATCH_ATTEST_HEIGHT,
            attestation_timestamp: BATCH_ATTEST_TIMESTAMP,
            foreign_height: 49,
        };
        let value = vote_json(&sealed);
        assert_eq!(value["chain_uid"], CHAIN_UID);
        assert_eq!(value["algo"], ALGO_PENDING);
        assert_eq!(value["attestations"][0]["source_id"], BATCH_SOURCE_ID);
        assert_eq!(value["attestations"][0]["height"], 1);
        assert_eq!(value["attestations"][0]["timestamp"], 1);
        assert_eq!(value["attestations"].as_array().unwrap().len(), 1);
        assert_eq!(value["root"], hex::encode([3u8; 32]));
        assert_eq!(value["foreign_height"], 49);
        assert_eq!(value["attestations"][0]["value"], hex::encode([2u8; 32]));
        let mut wide = sealed;
        wide.vk_hash = [9u8; 32];
        let wide_json = vote_json(&wide);
        assert_eq!(
            wide_json["attestations"][0]["value"],
            format!("{}{}", hex::encode([2u8; 32]), hex::encode([9u8; 32]))
        );
    }

    #[test]
    fn epoch_claim_halo2_body_rejects_a_signature() {
        let raw = r#"{"claims":[{"proof_hex":"ab","message":"headstash-claim-v1:","denom_hex":"0000000000000000000000000000000000000000000000000000000000000000","signature_hex":"cc","leaf_x_hex":"0000000000000000000000000000000000000000000000000000000000000000"}]}"#;
        assert!(serde_json::from_str::<super::Halo2ClaimsBody>(raw).is_err());
        let bare = r#"{"claims":[{"proof_hex":"ab","message":"headstash-claim-v1:aa","denom_hex":"0000000000000000000000000000000000000000000000000000000000000000","vk_id":1}]}"#;
        let parsed: super::Halo2ClaimsBody = serde_json::from_str(bare).unwrap();
        let items = super::claims_from_halo2(&parsed).unwrap();
        assert!(items[0].signature.is_empty());
        assert_eq!(items[0].leaf_x, [0u8; 32]);
        assert_eq!(items[0].vk_id, 1);
    }

    #[test]
    fn a_stray_lie_root_does_not_flip_the_seal() {
        assert!(!super::lab_lie_root_from(None, None));
        assert!(!super::lab_lie_root_from(Some("1"), None));
        assert!(!super::lab_lie_root_from(Some("1"), Some("0")));
        assert!(!super::lab_lie_root_from(None, Some("1")));
        assert!(super::lab_lie_root_from(Some("1"), Some("1")));
    }

    #[test]
    fn campaign_amount_accepts_a_u64_above_u32_max() {
        let wide = u64::from(u32::MAX) + 1;
        let raw = format!(r#"{{"amount":{wide},"cap":2}}"#);
        let parsed: super::AmountBody = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed.amount, wide);
        assert_eq!(parsed.cap, 2);
    }

    #[test]
    fn halo2_campaign_refuses_a_zero_vk_hash() {
        let zeros = "00".repeat(32);
        let nonzero = "11".repeat(32);
        let halo = campaign_json("halo2", 1, &zeros);
        let err = super::vk_from_body(&halo, headstash_epoch::Backend::Halo2).unwrap_err();
        assert_eq!(err.0, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!((err.1).0["error"], "vk hash");
        let ok = campaign_json("halo2", 1, &nonzero);
        let (id, hash) =
            super::vk_from_body(&ok, headstash_epoch::Backend::Halo2).unwrap();
        assert_eq!(id, 1);
        assert_eq!(hash[0], 0x11);
        let flock = campaign_json("flock", 0, "");
        let (id, hash) = super::vk_from_body(&flock, headstash_epoch::Backend::Flock).unwrap();
        assert_eq!(id, 0);
        assert_eq!(hash, [0u8; 32]);
    }

    fn campaign_json(backend: &str, vk_id: u64, vk_hash_hex: &str) -> super::CampaignBody {
        let z = "00".repeat(32);
        let raw = format!(
            r#"{{"backend":"{backend}","ctx_hex":"{z}","allocation_root_hex":"{z}","bech32_hrp":"terp","denom_hex":"{z}","proof_len":4,"cutoff_height":50,"amounts":[{{"amount":15,"cap":1}}],"trusted_hex":["{z}"],"trusted_verify_threshold":1,"trusted_sign_threshold":1,"max_rows":4,"vk_id":{vk_id},"vk_hash_hex":"{vk_hash_hex}"}}"#
        );
        serde_json::from_str(&raw).unwrap()
    }

    fn flock_bytes(cutoff: u64) -> Vec<u8> {
        let pk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32])
            .verifying_key()
            .to_bytes();
        let body = serde_json::json!({
            "backend": "flock",
            "ctx_hex": "11".repeat(32),
            "allocation_root_hex": "22".repeat(32),
            "bech32_hrp": "terp",
            "denom_hex": "33".repeat(32),
            "proof_len": 8,
            "cutoff_height": cutoff,
            "amounts": [{"amount": 15, "cap": 1}],
            "trusted_hex": [hex::encode(pk)],
            "trusted_verify_threshold": 1,
            "trusted_sign_threshold": 1,
            "max_rows": 4
        });
        serde_json::to_vec(&body).unwrap()
    }

    fn signed_headers(sk: &k256::ecdsa::SigningKey, path: &str, body: &[u8]) -> HeaderMap {
        let (pk, sig) = crate::tree_operator::sign_operator(sk, "POST", path, body, None, None);
        let mut headers = HeaderMap::new();
        headers.insert("x-hash-market-pubkey", pk.parse().unwrap());
        headers.insert("x-hash-market-signature", sig.parse().unwrap());
        headers
    }

    fn block_on<T>(fut: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(fut)
    }

    fn fresh_state(operator: &str, pubkeys: Vec<String>) -> (Arc<crate::server::AppState>, std::path::PathBuf) {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hs-op-auth-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = crate::server::AppState::new(
            None::<crate::VoteExtensionHandler>,
            "headstash-auth-test".into(),
            dir.clone(),
            false,
        )
        .unwrap();
        state.set_tree_operator(operator.to_string());
        state.set_operator_pubkeys(pubkeys);
        (Arc::new(state), dir)
    }

    #[test]
    fn admin_routes_reject_unsigned_wrong_key_replay_and_reinstall() {
        let (sk, addr) = crate::tree_operator::ephemeral_operator();
        let (state, dir) = fresh_state(&addr, Vec::new());
        let body = flock_bytes(50);
        let missing = block_on(super::install_campaign(
            State(state.clone()),
            HeaderMap::new(),
            Bytes::from(body.clone()),
        ));
        let err = missing.unwrap_err();
        assert_eq!(err.0, StatusCode::UNAUTHORIZED);

        let (other, _) = crate::tree_operator::ephemeral_operator();
        let wrong = block_on(super::install_campaign(
            State(state.clone()),
            signed_headers(&other, super::CAMPAIGN_PATH, &body),
            Bytes::from(body.clone()),
        ));
        assert_eq!(wrong.unwrap_err().0, StatusCode::FORBIDDEN);

        let ok = block_on(super::install_campaign(
            State(state.clone()),
            signed_headers(&sk, super::CAMPAIGN_PATH, &body),
            Bytes::from(body.clone()),
        ));
        assert!(ok.is_ok());

        let replay = block_on(super::install_campaign(
            State(state.clone()),
            signed_headers(&sk, super::CAMPAIGN_PATH, &body),
            Bytes::from(body.clone()),
        ));
        assert_eq!(replay.unwrap_err().0, StatusCode::UNAUTHORIZED);

        let second = flock_bytes(51);
        let again = block_on(super::install_campaign(
            State(state.clone()),
            signed_headers(&sk, super::CAMPAIGN_PATH, &second),
            Bytes::from(second),
        ));
        let conflict = again.unwrap_err();
        assert_eq!(conflict.0, StatusCode::CONFLICT);
        assert_eq!(conflict.1 .0["error"], "campaign already exists");

        let seal_body = br#"{"height":1}"#.to_vec();
        let unsigned_seal = block_on(super::seal(
            State(state.clone()),
            HeaderMap::new(),
            Bytes::from(seal_body.clone()),
        ));
        assert_eq!(unsigned_seal.unwrap_err().0, StatusCode::UNAUTHORIZED);

        block_on(async {
            state
                .headstash
                .write()
                .await
                .install_signer(ed25519_dalek::SigningKey::from_bytes(&[3u8; 32]));
        });
        let sealed = block_on(super::seal(
            State(state.clone()),
            signed_headers(&sk, super::SEAL_PATH, &seal_body),
            Bytes::from(seal_body.clone()),
        ));
        assert!(sealed.is_ok(), "{sealed:?}");
        let replay_seal = block_on(super::seal(
            State(state.clone()),
            signed_headers(&sk, super::SEAL_PATH, &seal_body),
            Bytes::from(seal_body),
        ));
        assert_eq!(replay_seal.unwrap_err().0, StatusCode::UNAUTHORIZED);

        let esk = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
        let epk = hex::encode(esk.verifying_key().to_bytes());
        let (ed_state, ed_dir) = fresh_state("", vec![epk.clone()]);
        let ed_body = flock_bytes(40);
        let challenge =
            crate::tree_operator::operator_challenge("POST", super::CAMPAIGN_PATH, &ed_body, None, None);
        let esig = crate::tree_operator::sign_operator_ed25519(&esk, &challenge);
        let mut ed_headers = HeaderMap::new();
        ed_headers.insert("x-hash-market-pubkey", epk.parse().unwrap());
        ed_headers.insert("x-hash-market-signature", esig.parse().unwrap());
        let ed_ok = block_on(super::install_campaign(
            State(ed_state),
            ed_headers,
            Bytes::from(ed_body),
        ));
        assert!(ed_ok.is_ok(), "{ed_ok:?}");

        let (exp_state, exp_dir) = fresh_state(&addr, Vec::new());
        let exp_body = flock_bytes(12);
        let (exp_pk, exp_sig) = crate::tree_operator::sign_operator(
            &sk,
            "POST",
            super::CAMPAIGN_PATH,
            &exp_body,
            Some("1"),
            None,
        );
        let mut exp_headers = HeaderMap::new();
        exp_headers.insert("x-hash-market-pubkey", exp_pk.parse().unwrap());
        exp_headers.insert("x-hash-market-signature", exp_sig.parse().unwrap());
        exp_headers.insert("x-hash-market-expiry", "1".parse().unwrap());
        let expired = block_on(super::install_campaign(
            State(exp_state),
            exp_headers,
            Bytes::from(exp_body),
        ));
        assert_eq!(expired.unwrap_err().0, StatusCode::UNAUTHORIZED);

        drop(state);
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(ed_dir);
        let _ = std::fs::remove_dir_all(exp_dir);
    }
}
