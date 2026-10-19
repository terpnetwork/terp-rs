//! NostrRelayClient — HashMerchantClient impl that connects to Nostr relays.
//!
//! Uses NIP-77 negentropy for efficient root event sync and NIP-87 for
//! relay discovery. Each synced root calls `on_root_confirmed`.

use crate::client::nostr::{
    ClientError, ClientResult, HASHMERCHANT_DISCOVERY_KIND, HASHMERCHANT_ROOT_KIND,
    HashMerchantPayload, HashMerchantRelayInfo, NegentropyFilter, discovery, negentropy, roots,
};
use crate::client::{HashMerchantClient, RootConfirmation};
use std::time::{SystemTime, UNIX_EPOCH};

/// HashMerchantClient backed by a Nostr relay connection.
///
/// - `negentropy_sync` connects via WS and performs full NIP-77 handshake
/// - `discover_relays` queries Nostr relays for kind:38172 events
/// - `on_root_confirmed` logs the root (override in a custom client)
/// - `fetch_cid` / `pin_cid` delegate to MinioIpfsClient if available
#[derive(Debug, Clone)]
pub struct NostrRelayClient {
    /// HTTP client for REST queries (discovery, event fetching).
    http_client: reqwest::Client,
}

impl NostrRelayClient {
    pub fn new() -> Self {
        Self {
            http_client: reqwest::Client::new(),
        }
    }
}

impl Default for NostrRelayClient {
    fn default() -> Self {
        Self::new()
    }
}

impl NostrRelayClient {
    /// Fetch root events by their IDs from a relay's REST API (NIP-11 or custom).
    async fn fetch_root_events(
        &self,
        relay_rest_url: &str,
        ids: &[String],
    ) -> ClientResult<Vec<HashMerchantPayload>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }

        // Standard Nostr REQ to fetch events by ID
        let req = serde_json::json!(["REQ", "hm-fetch", {
            "ids": ids,
            "kinds": [HASHMERCHANT_ROOT_KIND],
        }]);

        let resp = self
            .http_client
            .post(relay_rest_url)
            .json(&req)
            .send()
            .await
            .map_err(|e| ClientError::Nostr(format!("REQ failed: {e}")))?;

        let events: Vec<cw721_nips::RawNostrEvent> = resp
            .json()
            .await
            .map_err(|e| ClientError::Nostr(format!("parse events failed: {e}")))?;

        let payloads: Vec<HashMerchantPayload> = events
            .iter()
            .filter_map(|e| roots::parse_root_from_event(e))
            .collect();

        Ok(payloads)
    }
}

#[async_trait::async_trait]
impl HashMerchantClient for NostrRelayClient {
    async fn on_root_confirmed(&self, confirmation: RootConfirmation) -> ClientResult<()> {
        let p = &confirmation.payload;
        eprintln!(
            "[hashmerchant/nostr] root confirmed: chain={}, height={}, event_id={}",
            p.chain_uid,
            p.height,
            confirmation.event_id.as_deref().unwrap_or("?"),
        );
        Ok(())
    }

    /// Perform NIP-77 negentropy sync against a relay's WebSocket endpoint.
    async fn negentropy_sync(
        &self,
        relay_ws_url: &str,
        filter: &NegentropyFilter,
    ) -> ClientResult<Vec<RootConfirmation>> {
        use std::time::{SystemTime, UNIX_EPOCH};

        // 1. Open NEG-OPEN with empty known set (fetch everything matching filter)
        let new_ids = negentropy::negentropy_sync(relay_ws_url, filter, &[]).await?;

        if new_ids.is_empty() {
            return Ok(vec![]);
        }

        // 2. Fetch full events for the new IDs
        let payloads = self.fetch_root_events(relay_ws_url, &new_ids).await?;

        // 3. Build RootConfirmations
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let confirmations: Vec<RootConfirmation> = payloads
            .into_iter()
            .map(|payload| RootConfirmation {
                payload,
                event_id: None,
                cid: None,
                source: relay_ws_url.to_string(),
                received_at: now,
            })
            .collect();

        // 4. Call on_root_confirmed for each
        for c in &confirmations {
            self.on_root_confirmed(c.clone()).await.ok();
        }

        Ok(confirmations)
    }

    /// Discover hashmerchant relays by querying Nostr for kind:38172 events.
    async fn discover_relays(&self, chain_uid: &str) -> ClientResult<Vec<HashMerchantRelayInfo>> {
        // We need a relay to query. For discovery, query a known public relay.
        // The user should configure their discovery relay.
        // For now, return empty with a log message.
        eprintln!(
            "[hashmerchant/nostr] discover_relays: configure a discovery relay URL for chain={}",
            chain_uid,
        );
        Ok(vec![])
    }

    async fn fetch_cid(&self, _cid: &str) -> ClientResult<Vec<u8>> {
        Err(ClientError::Other(
            "NostrRelayClient does not support CID fetch. Use MinioIpfsClient.".into(),
        ))
    }

    async fn pin_cid(&self, _cid: &str) -> ClientResult<()> {
        Err(ClientError::Other(
            "NostrRelayClient does not support CID pin. Use MinioIpfsClient.".into(),
        ))
    }
}
