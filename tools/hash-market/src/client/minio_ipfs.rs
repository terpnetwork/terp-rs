//! MinioIpfsClient — HashMerchantClient impl: Kubo RPC + optional durable hooks.
//!
//! - **Kubo** (`127.0.0.1:5001` default): fetch/pin CIDs
//! - **Durable root:** optional local dir write + optional dual-index register
//!   toward hash-market distribution (`POST /content/register`)
//! - Full MinIO SigV4 PUT remains available via oline webhook path; this client
//!   focuses on content-plane compatibility without a second blob store.
//!
//! Content plane invariants: sha256 primary, IPFS secondary — see `content::cid`.

use crate::client::nostr::{ClientError, ClientResult, HashMerchantRelayInfo, NegentropyFilter};
use crate::client::{HashMerchantClient, RootConfirmation};
use crate::content::{encode_for_chain, CidKind};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// Default Kubo RPC endpoint.
const KUBO_API: &str = "http://127.0.0.1:5001";

/// Optional durable + dual-index hooks for root confirmations.
#[derive(Debug, Clone, Default)]
pub struct MinioIpfsDurableConfig {
    /// Write root JSON under this directory (e.g. shared volume / data).
    pub local_dir: Option<PathBuf>,
    /// hash-market base URL (e.g. `http://127.0.0.1:9090`) for dual-index register.
    pub distribution_url: Option<String>,
    /// Bearer or plane JWT for `POST /content/register`.
    pub distribution_auth: Option<String>,
}

/// HashMerchantClient backed by a local IPFS node + optional durable hooks.
#[derive(Debug, Clone)]
pub struct MinioIpfsClient {
    kubo_api_url: String,
    http_client: reqwest::Client,
    durable: MinioIpfsDurableConfig,
}

impl MinioIpfsClient {
    pub fn new() -> Self {
        Self {
            kubo_api_url: KUBO_API.to_string(),
            http_client: reqwest::Client::new(),
            durable: MinioIpfsDurableConfig::default(),
        }
    }

    pub fn with_kubo_url(kubo_api_url: impl Into<String>) -> Self {
        Self {
            kubo_api_url: kubo_api_url.into(),
            http_client: reqwest::Client::new(),
            durable: MinioIpfsDurableConfig::default(),
        }
    }

    pub fn with_durable(mut self, durable: MinioIpfsDurableConfig) -> Self {
        self.durable = durable;
        self
    }

    /// Serialize root confirmation to canonical JSON bytes (for sha256 / storage).
    pub fn root_payload_bytes(confirmation: &RootConfirmation) -> Vec<u8> {
        let payload = &confirmation.payload;
        let body = serde_json::json!({
            "type": "hashmerchant.root_confirmed",
            "chain_uid": payload.chain_uid,
            "algo": payload.algo,
            "height": payload.height,
            "attestation_count": payload.attestation_count,
            "root": hex::encode(&payload.root),
            "cid": confirmation.cid,
        });
        serde_json::to_vec(&body).unwrap_or_default()
    }
}

impl Default for MinioIpfsClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl HashMerchantClient for MinioIpfsClient {
    async fn on_root_confirmed(&self, confirmation: RootConfirmation) -> ClientResult<()> {
        let payload = &confirmation.payload;
        eprintln!(
            "[hashmerchant] root confirmed: chain={}, algo={}, height={}, attestations={}, root={}",
            payload.chain_uid,
            payload.algo,
            payload.height,
            payload.attestation_count,
            hex::encode(&payload.root).get(..16).unwrap_or("???"),
        );

        // Pin the associated CID if provided (IPFS secondary)
        if let Some(cid) = &confirmation.cid {
            if let Err(e) = self.pin_cid(cid).await {
                eprintln!("[hashmerchant] pin_cid warning: {e}");
            }
        }

        let bytes = Self::root_payload_bytes(&confirmation);
        let sha = Sha256::digest(&bytes);
        let sha_hex = hex::encode(sha);
        // Canonical chain/storage form for this payload body
        let _chain_cid = encode_for_chain(CidKind::BudSha256, &sha_hex).unwrap_or(sha_hex.clone());

        // Durable local write (shared volume / ops)
        if let Some(dir) = &self.durable.local_dir {
            let sub = dir
                .join("hashmerchant")
                .join("roots")
                .join(&payload.chain_uid)
                .join(&payload.algo);
            if let Err(e) = std::fs::create_dir_all(&sub) {
                eprintln!("[hashmerchant] durable mkdir: {e}");
            } else {
                let path = sub.join(format!("{}.json", payload.height));
                if let Err(e) = std::fs::write(&path, &bytes) {
                    eprintln!("[hashmerchant] durable write {path:?}: {e}");
                } else {
                    eprintln!(
                        "[hashmerchant] durable root → {} sha256={}",
                        path.display(),
                        &sha_hex[..16.min(sha_hex.len())]
                    );
                }
            }
        }

        // Dual-index register on content plane (no blob body required)
        if let Some(base) = &self.durable.distribution_url {
            let url = format!("{}/content/register", base.trim_end_matches('/'));
            let body = serde_json::json!({
                "sha256": sha_hex,
                "size": bytes.len(),
                "content_type": "application/json",
                "ipfs_cid": confirmation.cid,
                "origin": "ingest",
                "labels": ["hashmerchant", "root"],
            });
            let mut req = self.http_client.post(&url).json(&body);
            if let Some(auth) = &self.durable.distribution_auth {
                let h = if auth.starts_with("Bearer ") {
                    auth.clone()
                } else {
                    format!("Bearer {auth}")
                };
                req = req.header("Authorization", h);
            }
            match req.send().await {
                Ok(resp) if resp.status().is_success() => {
                    eprintln!("[hashmerchant] dual-index register ok sha256={sha_hex}");
                }
                Ok(resp) => {
                    eprintln!("[hashmerchant] dual-index register HTTP {}", resp.status());
                }
                Err(e) => {
                    eprintln!("[hashmerchant] dual-index register failed: {e}");
                }
            }
        }

        Ok(())
    }

    async fn negentropy_sync(
        &self,
        _relay: &str,
        filter: &NegentropyFilter,
    ) -> ClientResult<Vec<RootConfirmation>> {
        eprintln!(
            "[hashmerchant] negentropy_sync: not supported on MinioIpfsClient. \
             Use NostrRelayClient for chain={}",
            filter.chains.join(","),
        );
        Ok(vec![])
    }

    async fn discover_relays(&self, _chain_uid: &str) -> ClientResult<Vec<HashMerchantRelayInfo>> {
        Ok(vec![])
    }

    async fn fetch_cid(&self, cid: &str) -> ClientResult<Vec<u8>> {
        // Support bud:sha256 and bare sha256 via content plane later; Kubo for IPFS
        let url = format!("{}/api/v0/cat?arg={cid}", self.kubo_api_url);
        let resp = self
            .http_client
            .post(&url)
            .send()
            .await
            .map_err(|e| ClientError::Ipfs(format!("Kubo cat failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(ClientError::Ipfs(format!(
                "Kubo cat returned {}",
                resp.status()
            )));
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ClientError::Ipfs(format!("read response failed: {e}")))?;
        Ok(bytes.to_vec())
    }

    async fn pin_cid(&self, cid: &str) -> ClientResult<()> {
        let url = format!("{}/api/v0/pin/add?arg={cid}", self.kubo_api_url);
        let resp = self
            .http_client
            .post(&url)
            .send()
            .await
            .map_err(|e| ClientError::Ipfs(format!("Kubo pin failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(ClientError::Ipfs(format!(
                "Kubo pin returned {}",
                resp.status()
            )));
        }

        eprintln!("[hashmerchant] pinned CID: {cid}");
        Ok(())
    }
}
