//! Ethereum JSON-RPC client for `eth_getProof` and `eth_getBlockByNumber`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Minimal Ethereum RPC client.
pub struct EthClient {
    rpc_url: String,
    client: reqwest::Client,
}

impl EthClient {
    pub fn new(rpc_url: &str) -> Self {
        Self {
            rpc_url: rpc_url.to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Call `eth_getProof` for an account at a given block.
    pub async fn get_proof(
        &self,
        address: &str,
        storage_keys: &[String],
        block: &str,
    ) -> Result<EthProofResponse> {
        let params = serde_json::json!([address, storage_keys, block]);
        let resp: JsonRpcResponse<EthProofResponse> = self.rpc_call("eth_getProof", params).await?;
        resp.result.context("eth_getProof returned null")
    }

    /// Call `eth_getBlockByNumber`.
    pub async fn get_block_by_number(&self, block: &str, full_txs: bool) -> Result<EthBlock> {
        let params = serde_json::json!([block, full_txs]);
        let resp: JsonRpcResponse<EthBlock> = self.rpc_call("eth_getBlockByNumber", params).await?;
        resp.result.context("eth_getBlockByNumber returned null")
    }

    /// Get the latest block number (hex string).
    pub async fn block_number(&self) -> Result<String> {
        let resp: JsonRpcResponse<String> = self
            .rpc_call("eth_blockNumber", serde_json::json!([]))
            .await?;
        resp.result.context("eth_blockNumber returned null")
    }

    async fn rpc_call<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<JsonRpcResponse<T>> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1,
        });

        let resp = self
            .client
            .post(&self.rpc_url)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("RPC request to {} failed", self.rpc_url))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("RPC HTTP {status}: {text}");
        }

        resp.json().await.context("failed to parse RPC response")
    }
}

// ---------------------------------------------------------------------------
// JSON-RPC types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct JsonRpcResponse<T> {
    result: Option<T>,
    #[allow(dead_code)]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    #[allow(dead_code)]
    code: i64,
    #[allow(dead_code)]
    message: String,
}

/// Response from `eth_getProof`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EthProofResponse {
    pub address: String,
    pub balance: String,
    pub code_hash: String,
    pub nonce: String,
    pub storage_hash: String,
    pub account_proof: Vec<String>,
    pub storage_proof: Vec<StorageProof>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageProof {
    pub key: String,
    pub value: String,
    pub proof: Vec<String>,
}

/// Minimal block header from `eth_getBlockByNumber`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EthBlock {
    pub number: String,
    pub hash: String,
    pub state_root: String,
    pub timestamp: String,
    pub parent_hash: String,
}

impl EthBlock {
    /// Parse the hex block number to u64.
    pub fn block_number(&self) -> Result<u64> {
        let s = self.number.strip_prefix("0x").unwrap_or(&self.number);
        u64::from_str_radix(s, 16).context("invalid block number hex")
    }

    /// Parse the hex timestamp to i64 (unix seconds).
    pub fn block_time(&self) -> Result<i64> {
        let s = self.timestamp.strip_prefix("0x").unwrap_or(&self.timestamp);
        let ts = u64::from_str_radix(s, 16).context("invalid timestamp hex")?;
        Ok(ts as i64)
    }

    /// Decode the state root from hex to bytes.
    pub fn state_root_bytes(&self) -> Result<Vec<u8>> {
        let s = self
            .state_root
            .strip_prefix("0x")
            .unwrap_or(&self.state_root);
        hex::decode(s).context("invalid state_root hex")
    }
}
