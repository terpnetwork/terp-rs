//! Minimal Kubo HTTP API client (add bytes → CID).

use anyhow::{Context, Result};

#[derive(Clone, Debug)]
pub struct IpfsClient {
    api_base: String,
    pub gateway_public: String,
}

impl IpfsClient {
    pub fn new(api_base: String, gateway_public: String) -> Self {
        Self {
            api_base: api_base.trim_end_matches('/').to_string(),
            gateway_public,
        }
    }

    /// `ipfs add` via multipart to `/api/v0/add?pin=true`.
    pub async fn add_bytes(&self, bytes: &[u8]) -> Result<String> {
        // reqwest is available under server feature (hash-market server).
        let url = format!("{}/api/v0/add?pin=true&cid-version=1", self.api_base);
        let part = reqwest::multipart::Part::bytes(bytes.to_vec())
            .file_name("blob")
            .mime_str("application/octet-stream")
            .unwrap_or_else(|_| reqwest::multipart::Part::bytes(bytes.to_vec()));
        let form = reqwest::multipart::Form::new().part("file", part);
        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .multipart(form)
            .send()
            .await
            .context("ipfs add request")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("ipfs add HTTP {status}: {body}");
        }
        let text = resp.text().await.context("ipfs add body")?;
        // Kubo returns one JSON object per line; take first Hash field
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let v: serde_json::Value = serde_json::from_str(line).context("ipfs add json")?;
            if let Some(h) = v.get("Hash").and_then(|x| x.as_str()) {
                return Ok(h.to_string());
            }
            if let Some(h) = v.get("Cid").and_then(|x| x.as_str()) {
                return Ok(h.to_string());
            }
        }
        anyhow::bail!("ipfs add: no Hash in response: {text}");
    }

    /// Best-effort fetch via gateway (for resolve proxy).
    pub async fn cat_gateway(&self, cid: &str) -> Result<Vec<u8>> {
        let url = if self.gateway_public.starts_with("http") {
            format!("{}/{}", self.gateway_public.trim_end_matches('/'), cid)
        } else {
            // relative gateway — use API cat instead
            let api = format!("{}/api/v0/cat?arg={cid}", self.api_base);
            let client = reqwest::Client::new();
            let resp = client.post(&api).send().await.context("ipfs cat")?;
            if !resp.status().is_success() {
                anyhow::bail!("ipfs cat HTTP {}", resp.status());
            }
            return Ok(resp.bytes().await?.to_vec());
        };
        let client = reqwest::Client::new();
        let resp = client.get(&url).send().await.context("ipfs gateway get")?;
        if !resp.status().is_success() {
            anyhow::bail!("ipfs gateway HTTP {}", resp.status());
        }
        Ok(resp.bytes().await?.to_vec())
    }
}
