//! NIP-77 Negentropy sync for HashMerchant root events.
//!
//! Negentropy is a set reconciliation protocol. This module provides
//! the hashmerchant-specific sync logic: maintaining a local set of
//! root IDs, computing the delta with a remote relay, and fetching
//! only the new root events.

use crate::client::nostr::{ClientError, ClientResult, NegentropyFilter};
use std::collections::HashSet;

/// A local store of known root event IDs, indexed by chain.
///
/// Negentropy works by exchanging ID sets between the client and relay.
/// Each root event has a unique Nostr event ID (SHA-256 of the canonical
/// JSON). The relay holds ALL root IDs for a chain; the client holds the
/// IDs it has already seen. Negentropy reconciles the difference.
#[derive(Debug, Clone, Default)]
pub struct RootIdStore {
    /// Known root event IDs per chain_uid.
    chains: std::collections::HashMap<String, HashSet<String>>,
}

impl RootIdStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register that we've seen a root event for a chain.
    pub fn insert(&mut self, chain_uid: &str, event_id: String) {
        self.chains
            .entry(chain_uid.to_string())
            .or_default()
            .insert(event_id);
    }

    /// Get all known root IDs for a chain.
    pub fn ids_for(&self, chain_uid: &str) -> Vec<String> {
        self.chains
            .get(chain_uid)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Compute the delta: given a set of remote IDs, return which
    /// remote IDs we don't have locally.
    ///
    /// This is the core negentropy operation — the relay sends its
    /// known IDs, the client returns the subset it's missing.
    pub fn missing_from(&self, chain_uid: &str, remote_ids: &[String]) -> Vec<String> {
        let local = self.chains.get(chain_uid);
        remote_ids
            .iter()
            .filter(|id| !local.map_or(false, |s| s.contains(*id)))
            .cloned()
            .collect()
    }

    /// Number of known root IDs for a chain.
    pub fn count(&self, chain_uid: &str) -> usize {
        self.chains.get(chain_uid).map(|s| s.len()).unwrap_or(0)
    }

    /// Insert many IDs at once (batch insert from negentropy response).
    pub fn insert_batch(&mut self, chain_uid: &str, ids: Vec<String>) {
        let entry = self.chains.entry(chain_uid.to_string()).or_default();
        for id in ids {
            entry.insert(id);
        }
    }
}

/// Build a NIP-77-style NEG-OPEN message filter.
///
/// Returns the JSON-encoded filter to send in a NEG-OPEN message.
pub fn build_negentropy_filter(filter: &NegentropyFilter) -> serde_json::Value {
    let mut f = serde_json::json!({
        "kinds": filter.kinds,
    });
    if !filter.chains.is_empty() {
        f["#chain"] = serde_json::json!(filter.chains);
    }
    if let Some(since) = filter.since {
        f["since"] = serde_json::json!(since);
    }
    if let Some(until) = filter.until {
        f["until"] = serde_json::json!(until);
    }
    f
}

/// Perform a negentropy sync against a remote Nostr relay via its WS endpoint.
///
/// This is a simplified negentropy handshake:
/// 1. Client sends `["NEG-OPEN", <sub_id>, <filter>, <initial_ids>]`
/// 2. Relay responds with missing IDs
/// 3. Client REQs those IDs to fetch the full events
///
/// Returns the IDs of new root events that need to be fetched.
pub async fn negentropy_sync(
    relay_ws_url: &str,
    filter: &NegentropyFilter,
    known_ids: &[String],
) -> ClientResult<Vec<String>> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    let (mut ws_stream, _) = connect_async(relay_ws_url)
        .await
        .map_err(|e| ClientError::Ws(format!("connection failed: {e}")))?;

    // Step 1: Send NEG-OPEN
    let sub_id = format!("hm-sync-{}", filter.chains.join("-"));
    let neg_open = serde_json::json!([
        "NEG-OPEN",
        sub_id,
        build_negentropy_filter(filter),
        known_ids,
    ]);

    ws_stream
        .send(Message::Text(neg_open.to_string()))
        .await
        .map_err(|e| ClientError::Ws(format!("send NEG-OPEN failed: {e}")))?;

    // Step 2: Expect NEG-MSG with missing IDs
    let new_ids = loop {
        match ws_stream.next().await {
            Some(Ok(Message::Text(text))) => {
                let parsed: serde_json::Value = serde_json::from_str(&text)
                    .map_err(|e| ClientError::Nostr(format!("invalid relay response: {e}")))?;

                if let Some(arr) = parsed.as_array() {
                    if arr.len() >= 3 && arr[0] == "NEG-MSG" {
                        // arr[1] = sub_id, arr[2] = missing_ids
                        if let Some(ids) = arr[2].as_array() {
                            let ids: Vec<String> = ids
                                .iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect();
                            break ids;
                        }
                    }
                }
            }
            Some(Ok(Message::Close(_))) => break vec![],
            Some(Ok(_)) => continue, // ping/pong/binary — ignore
            Some(Err(e)) => {
                return Err(ClientError::Ws(format!("WS error: {e}")));
            }
            None => break vec![],
        }
    };

    // Close cleanly
    ws_stream.close(None).await.ok();

    Ok(new_ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_id_store_basic() {
        let mut store = RootIdStore::new();

        store.insert("ethereum-mainnet", "id1".into());
        store.insert("ethereum-mainnet", "id2".into());
        store.insert("cosmoshub-4", "id3".into());

        assert_eq!(store.count("ethereum-mainnet"), 2);
        assert_eq!(store.count("cosmoshub-4"), 1);
        assert_eq!(store.count("unknown"), 0);
    }

    #[test]
    fn root_id_store_missing_from() {
        let mut store = RootIdStore::new();
        store.insert("ethereum-mainnet", "id1".into());

        let remote = vec!["id1".into(), "id2".into(), "id3".into()];
        let missing = store.missing_from("ethereum-mainnet", &remote);
        assert_eq!(missing.len(), 2);
        assert!(missing.contains(&"id2".to_string()));
        assert!(missing.contains(&"id3".to_string()));
    }

    #[test]
    fn root_id_store_batch_insert() {
        let mut store = RootIdStore::new();
        store.insert_batch("ethereum-mainnet", vec!["a".into(), "b".into(), "c".into()]);
        assert_eq!(store.count("ethereum-mainnet"), 3);
    }

    #[test]
    fn build_negentropy_filter_chain_only() {
        let filter = NegentropyFilter::for_chain("ethereum-mainnet");
        let json = build_negentropy_filter(&filter);
        assert_eq!(json["kinds"][0], 30070);
        assert_eq!(json["#chain"][0], "ethereum-mainnet");
    }

    #[test]
    fn build_negentropy_filter_with_time() {
        let filter = NegentropyFilter {
            kinds: vec![30070],
            chains: vec!["cosmoshub-4".into()],
            since: Some(1700000000),
            until: None,
        };
        let json = build_negentropy_filter(&filter);
        assert_eq!(json["since"], 1700000000);
        assert!(json.get("until").is_none());
    }
}
