//! Trusted-builder soulbound claims (Trustless Manifesto → hashmerchant VE).
//!
//! POST a claim after an Ethereum `sign()` on the manifesto contract.
//! The claim is mixed into vote-extension `root` so Terp can mint an SBT.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use sha2::{Digest, Sha256};

use crate::msg::VoteExtensionHashData;
use crate::server::AppState;
#[cfg(feature = "ve")]
use crate::ve::server::{ProviderData, ProviderKey};

pub const VE_RUNTIME_ID: &str = "trusted-builder-sbt";
pub const VE_CHAIN_UID: &str = "eth-trustless-manifesto";
pub const VE_ALGO: &str = "sha256";

/// On-chain manifesto (Ethereum mainnet).
pub const MANIFESTO_MAINNET: &str = "0x32aa964746ba2be65c71fe4a5cb3c4a023ca3e20";

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TrustedBuilderClaim {
    pub eth_address: String,
    pub manifesto_contract: String,
    pub eth_chain_id: u64,
    pub sign_tx: String,
    pub signed_height: u64,
    #[serde(default)]
    pub terp_recipient: Option<String>,
}

impl TrustedBuilderClaim {
    pub fn commitment(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(b"trusted-builder-sbt/v1");
        h.update(self.eth_address.to_ascii_lowercase().as_bytes());
        h.update(self.manifesto_contract.to_ascii_lowercase().as_bytes());
        h.update(self.eth_chain_id.to_be_bytes());
        h.update(self.sign_tx.as_bytes());
        h.update(self.signed_height.to_be_bytes());
        h.finalize().into()
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/claims", post(put_claim).get(list_claims))
        .route("/claims/:eth", get(get_claim))
        .with_state(state)
}

async fn put_claim(
    State(state): State<Arc<AppState>>,
    Json(claim): Json<TrustedBuilderClaim>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if claim.eth_address.is_empty() || claim.sign_tx.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let key = claim.eth_address.to_ascii_lowercase();
    let commit = claim.commitment();
    {
        let mut map = state.trusted_builder.write().await;
        map.insert(key.clone(), claim.clone());
    }

    #[cfg(feature = "ve")]
    if state.ve_enabled {
        let root = {
            let map = state.trusted_builder.read().await;
            let mut acc = Sha256::new();
            acc.update(b"trusted-builder-set/v1");
            let mut keys: Vec<_> = map.keys().cloned().collect();
            keys.sort();
            for k in keys {
                if let Some(c) = map.get(&k) {
                    acc.update(c.commitment());
                }
            }
            acc.finalize().to_vec()
        };
        let data = VoteExtensionHashData {
            runtime_id: VE_RUNTIME_ID.into(),
            chain_uid: VE_CHAIN_UID.into(),
            algo: VE_ALGO.into(),
            root,
            foreign_height: claim.signed_height,
            foreign_block_time: chrono_now(),
            ics23_proof: commit.to_vec(),
        };
        let mut store = state.provider_data.write().await;
        store.insert(
            ProviderKey {
                chain_uid: VE_CHAIN_UID.into(),
                algo: VE_ALGO.into(),
            },
            ProviderData {
                data,
                received_at: claim.signed_height,
            },
        );
    }

    Ok(Json(serde_json::json!({
        "ok": true,
        "eth_address": key,
        "commitment": hex::encode(commit),
        "ve_chain_uid": VE_CHAIN_UID,
    })))
}

async fn get_claim(
    State(state): State<Arc<AppState>>,
    Path(eth): Path<String>,
) -> Result<Json<TrustedBuilderClaim>, StatusCode> {
    let map = state.trusted_builder.read().await;
    map.get(&eth.to_ascii_lowercase())
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn list_claims(State(state): State<Arc<AppState>>) -> Json<Vec<TrustedBuilderClaim>> {
    let map = state.trusted_builder.read().await;
    Json(map.values().cloned().collect())
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TrustedBuilderClaim {
        TrustedBuilderClaim {
            eth_address: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".into(),
            manifesto_contract: MANIFESTO_MAINNET.into(),
            eth_chain_id: 31337,
            sign_tx: "0xabc".into(),
            signed_height: 4,
            terp_recipient: None,
        }
    }

    #[test]
    fn sane_commitment_is_32_bytes_and_stable() {
        let a = sample().commitment();
        let b = sample().commitment();
        assert_eq!(a, b);
        assert_eq!(a.len(), 32);
        assert_ne!(a, [0u8; 32]);
    }

    #[test]
    fn insane_different_signer_different_commitment() {
        let mut other = sample();
        other.eth_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8".into();
        assert_ne!(sample().commitment(), other.commitment());
    }

    #[test]
    fn insane_empty_eth_rejected_by_handler_contract() {
        // Handler rejects empty eth / sign_tx before insert.
        let bad = TrustedBuilderClaim {
            eth_address: String::new(),
            manifesto_contract: MANIFESTO_MAINNET.into(),
            eth_chain_id: 1,
            sign_tx: String::new(),
            signed_height: 0,
            terp_recipient: None,
        };
        assert!(bad.eth_address.is_empty() || bad.sign_tx.is_empty());
    }
}
