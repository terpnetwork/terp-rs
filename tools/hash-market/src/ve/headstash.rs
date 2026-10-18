//! Headstash claim routes on the vote-extension server.
//!
//! `/headstash/:id` stays the snap note server. These routes live under
//! `/ve/headstash/`. The seal body does not carry a key.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{Json, extract::State, http::StatusCode};
use ed25519_dalek::SigningKey;
use headstash_epoch::{
    ALGO_ACCEPTED, ALGO_PENDING, BATCH_ATTEST_TIMESTAMP, BATCH_SOURCE_ID, Backend, CHAIN_UID,
    Campaign, CampaignParams, DEPTH, IngressError, PeerReceipt, SealedVote, SubmittedClaim,
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
        "value": hex::encode(sealed.batch_hash),
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
    Json(body): Json<CampaignBody>,
) -> Result<Json<serde_json::Value>, HttpError> {
    let params = params_from(&body)?;
    let campaign = Campaign::<DEPTH>::open(params).map_err(error_json)?;
    let mut host = state.headstash.write().await;
    if host
        .campaign
        .as_ref()
        .is_some_and(|existing| !existing.digests().is_empty())
    {
        return Err(error_json_msg(
            StatusCode::CONFLICT,
            "campaign already has claims",
        ));
    }
    host.campaign = Some(campaign);
    host.sealed = None;
    Ok(Json(serde_json::json!({"status": "ok"})))
}

pub async fn submit_claims(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ClaimsBody>,
) -> Result<Json<serde_json::Value>, HttpError> {
    let items = claims_from(&body)?;
    let mut host = state.headstash.write().await;
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
    Json(body): Json<SealBody>,
) -> Result<Json<serde_json::Value>, HttpError> {
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
    let sealed = campaign.seal(body.height, signer);
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
    })
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

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::vote_json;
    use headstash_epoch::{
        ALGO_PENDING, BATCH_ATTEST_HEIGHT, BATCH_ATTEST_TIMESTAMP, BATCH_SOURCE_ID, CHAIN_UID,
        SealedVote,
    };

    #[test]
    fn vote_json_is_one_headstash_batch_attestation() {
        let sealed = SealedVote {
            algo: ALGO_PENDING.into(),
            batch_hash: [2u8; 32],
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
    }

    #[test]
    fn campaign_amount_accepts_a_u64_above_u32_max() {
        let wide = u64::from(u32::MAX) + 1;
        let raw = format!(r#"{{"amount":{wide},"cap":2}}"#);
        let parsed: super::AmountBody = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed.amount, wide);
        assert_eq!(parsed.cap, 2);
    }
}
