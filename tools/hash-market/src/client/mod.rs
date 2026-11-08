//! HashMerchantClient trait — uniform interface for consuming hashmerchant roots.
//!
//! Three default implementations:
//! - `MinioIpfsClient` — local Kubo RPC + MinIO for CID fetch/pin
//! - `NostrRelayClient` — connects to Nostr relay via NEG-OPEN + fetches roots
//! - `HttpWebhookClient` — POSTs root confirmations to HTTP endpoints
//!
//! Client polling loop: eth_getProof → Pallas transform → encode → POST to sidecar.

pub mod eth;
pub mod hashmerchant;
pub mod minio_ipfs;
pub mod nostr;
pub mod ve;

pub use hashmerchant::{HashMerchantClient, RootConfirmation};
pub use minio_ipfs::{MinioIpfsClient, MinioIpfsDurableConfig};

use crate::fields::pasta;

use crate::EthClient;
use crate::msg::VoteExtensionHashData;
use anyhow::{Context, Result};

/// Run the ETH proof polling loop.
pub async fn run_poll_loop(
    eth: &EthClient,
    sidecar_url: &str,
    runtime_id: &str,
    chain_uid: &str,
    interval_secs: u64,
    storage_keys: &[String],
    account_address: &str,
) -> Result<()> {
    let http = reqwest::Client::new();
    let mut last_height: u64 = 0;

    loop {
        match poll_once(
            eth,
            &http,
            PollOnceParams {
                sidecar_url,
                runtime_id,
                chain_uid,
                storage_keys,
                account_address,
                last_height,
            },
        )
        .await
        {
            Ok(Some(height)) => {
                last_height = height;
                tracing::info!(height, "submitted vote extension data");
            }
            Ok(None) => {
                tracing::debug!("no new block");
            }
            Err(e) => {
                tracing::warn!(error = %e, "poll cycle failed");
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
    }
}

/// Arguments for [`poll_once`].
struct PollOnceParams<'a> {
    pub sidecar_url: &'a str,
    pub runtime_id: &'a str,
    pub chain_uid: &'a str,
    pub storage_keys: &'a [String],
    pub account_address: &'a str,
    pub last_height: u64,
}

async fn poll_once(
    eth: &EthClient,
    http: &reqwest::Client,
    args: PollOnceParams<'_>,
) -> Result<Option<u64>> {
    let PollOnceParams {
        sidecar_url,
        runtime_id,
        chain_uid,
        storage_keys,
        account_address,
        last_height,
    } = args;
    // Get latest block
    let block_hex = eth.block_number().await?;
    let block = eth.get_block_by_number(&block_hex, false).await?;
    let height = block.block_number()?;

    if height <= last_height {
        return Ok(None);
    }

    // Get proof
    let proof = eth
        .get_proof(account_address, storage_keys, &block_hex)
        .await?;

    // Transform storage hash to Pallas
    let state_root = block.state_root_bytes()?;
    let pallas_leaves = pasta::transform_proofs(std::slice::from_ref(&state_root));
    let root = if let Some(leaf) = pallas_leaves.first() {
        leaf.0.to_vec()
    } else {
        state_root
    };

    let block_time = block.block_time()?;

    let data = VoteExtensionHashData {
        runtime_id: runtime_id.to_string(),
        chain_uid: chain_uid.to_string(),
        algo: "keccak256".to_string(),
        root,
        foreign_height: height,
        foreign_block_time: block_time,
        ics23_proof: serde_json::to_vec(&proof.account_proof)
            .context("failed to serialize account_proof")?,
    };

    // POST to sidecar
    let encoded = data.encode();
    http.post(format!("{sidecar_url}/extend-vote"))
        .json(&serde_json::json!({
            "height": height,
            "data": hex::encode(&encoded),
        }))
        .send()
        .await
        .context("POST to sidecar failed")?;

    Ok(Some(height))
}
