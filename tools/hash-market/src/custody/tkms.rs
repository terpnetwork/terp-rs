//! TKMS (Tendermint Key Management System) remote custody.
//!
//! Sends signing requests over a TCP JSON protocol to an external KMS process.

use super::Custody;
use anyhow::{Context, Result};
use async_trait::async_trait;

/// Remote custody that forwards signing to a TKMS instance over TCP.
pub struct TkmsCustody {
    addr: String,
    pk_bytes: Vec<u8>,
}

/// JSON request sent to the TKMS TCP socket.
#[derive(serde::Serialize)]
struct TkmsRequest {
    method: String,
    payload: String, // hex-encoded message
}

/// JSON response from the TKMS TCP socket.
#[derive(serde::Deserialize)]
struct TkmsResponse {
    signature: Option<String>, // hex-encoded compact signature
    public_key: Option<String>,
    error: Option<String>,
}

impl TkmsCustody {
    /// Connect to a TKMS instance and fetch the public key.
    pub async fn connect(addr: &str) -> Result<Self> {
        let resp = Self::rpc(addr, "public_key", "").await?;
        let pk_hex = resp.public_key.context("TKMS did not return public_key")?;
        let pk_bytes = hex::decode(&pk_hex).context("invalid hex from TKMS public_key")?;
        Ok(Self {
            addr: addr.to_string(),
            pk_bytes,
        })
    }

    async fn rpc(addr: &str, method: &str, payload: &str) -> Result<TkmsResponse> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpStream;

        let mut stream = TcpStream::connect(addr)
            .await
            .with_context(|| format!("failed to connect to TKMS at {addr}"))?;

        let req = TkmsRequest {
            method: method.to_string(),
            payload: payload.to_string(),
        };
        let req_bytes = serde_json::to_vec(&req)?;

        // Length-prefixed write: 4-byte big-endian length + JSON payload
        let len = (req_bytes.len() as u32).to_be_bytes();
        stream.write_all(&len).await?;
        stream.write_all(&req_bytes).await?;
        stream.flush().await?;

        // Length-prefixed read
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await?;
        let resp_len = u32::from_be_bytes(len_buf) as usize;
        anyhow::ensure!(resp_len <= 1_048_576, "TKMS response too large");

        let mut resp_buf = vec![0u8; resp_len];
        stream.read_exact(&mut resp_buf).await?;

        let resp: TkmsResponse = serde_json::from_slice(&resp_buf)?;
        if let Some(err) = &resp.error {
            anyhow::bail!("TKMS error: {err}");
        }
        Ok(resp)
    }
}

#[async_trait]
impl Custody for TkmsCustody {
    async fn sign(&self, msg: &[u8]) -> Result<Vec<u8>> {
        let payload_hex = hex::encode(msg);
        let resp = Self::rpc(&self.addr, "sign", &payload_hex).await?;
        let sig_hex = resp.signature.context("TKMS did not return signature")?;
        hex::decode(&sig_hex).context("invalid hex from TKMS signature")
    }

    fn public_key(&self) -> &[u8] {
        &self.pk_bytes
    }

    fn label(&self) -> &str {
        "tkms"
    }
}
