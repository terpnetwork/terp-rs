//! HttpWebhookClient — HashMerchantClient impl that POSTs roots to HTTP endpoints.
//!
//! Useful for connecting hashmerchant root confirmations to external systems
//! like CI/CD pipelines, monitoring, or archive storage.

use crate::client::{HashMerchantClient, RootConfirmation};
use crate::nostr::{
    ClientError, ClientResult, HashMerchantPayload, HashMerchantRelayInfo, NegentropyFilter,
};

/// HashMerchantClient that forwards root confirmations to webhook URLs.
///
/// On `on_root_confirmed`, POSTs the confirmation JSON to the configured URL.
/// Supports multiple webhook endpoints.
///
/// The webhook URL can be set at construction time. If no URL is configured,
/// roots are logged to stderr only.
#[derive(Debug, Clone)]
pub struct HttpWebhookClient {
    /// Webhook URLs to POST root confirmations to.
    webhook_urls: Vec<String>,
    http_client: reqwest::Client,
}

impl HttpWebhookClient {
    /// Create a new client with a single webhook URL.
    pub fn new(webhook_url: impl Into<String>) -> Self {
        Self {
            webhook_urls: vec![webhook_url.into()],
            http_client: reqwest::Client::new(),
        }
    }

    /// Create a new client with multiple webhook URLs.
    pub fn with_urls(urls: Vec<String>) -> Self {
        Self {
            webhook_urls: urls,
            http_client: reqwest::Client::new(),
        }
    }

    /// Add a webhook URL at runtime.
    pub fn add_url(&mut self, url: impl Into<String>) {
        self.webhook_urls.push(url.into());
    }
}

#[async_trait::async_trait]
impl HashMerchantClient for HttpWebhookClient {
    async fn on_root_confirmed(&self, confirmation: RootConfirmation) -> ClientResult<()> {
        let body = serde_json::to_value(&confirmation).map_err(|e| ClientError::Serde(e))?;

        for url in &self.webhook_urls {
            let resp = self.http_client.post(url).json(&body).send().await;

            match resp {
                Ok(r) if r.status().is_success() => {
                    eprintln!("[hashmerchant/webhook] sent to {url}: OK");
                }
                Ok(r) => {
                    eprintln!(
                        "[hashmerchant/webhook] sent to {url}: {} {}",
                        r.status(),
                        r.text().await.unwrap_or_default().trim(),
                    );
                }
                Err(e) => {
                    eprintln!("[hashmerchant/webhook] failed to POST {url}: {e}");
                }
            }
        }

        Ok(())
    }

    async fn negentropy_sync(
        &self,
        _relay: &str,
        _filter: &NegentropyFilter,
    ) -> ClientResult<Vec<RootConfirmation>> {
        // HttpWebhookClient doesn't handle negentropy — it receives roots
        // via HTTP POST, not Nostr sync.
        Ok(vec![])
    }

    async fn discover_relays(&self, _chain_uid: &str) -> ClientResult<Vec<HashMerchantRelayInfo>> {
        Ok(vec![])
    }

    async fn fetch_cid(&self, _cid: &str) -> ClientResult<Vec<u8>> {
        Err(ClientError::Other(
            "HttpWebhookClient does not support CID fetch. Use MinioIpfsClient.".into(),
        ))
    }

    async fn pin_cid(&self, cid: &str) -> ClientResult<()> {
        // Forward pin request to webhook as a control message
        let pin_msg = serde_json::json!({"action": "pin", "cid": cid});
        for url in &self.webhook_urls {
            self.http_client.post(url).json(&pin_msg).send().await.ok();
        }
        Ok(())
    }
}
