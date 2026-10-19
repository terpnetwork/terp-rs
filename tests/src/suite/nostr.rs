//! NostrRelaySuite — local NIP-01 Nostr relay for integration testing.
//!
//! Wraps `ict_rs::nostr::NostrRelayerManager` into the `TerpSidecar` trait.
//! Uses `mattn/nostr-relay:latest` Docker image.

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use tracing::info;

use ict_rs::nostr::NostrRelayerManager;
use ict_rs::runtime::RuntimeBackend;

use crate::suite::sidecar::{EndpointMap, HealthStatus, SidecarId, SidecarRegistry, TerpSidecar};

/// Local Nostr relay for integration testing.
pub struct NostrRelaySuite {
    id: SidecarId,
    inner: Option<NostrRelayerManager>,
    pub(crate) runtime: Option<Arc<dyn RuntimeBackend>>,
    endpoint_url: Option<String>,
    started: bool,
}

impl NostrRelaySuite {
    /// Create a new Nostr relay sidecar.
    /// Call `start()` with a runtime to launch.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            inner: None,
            runtime: None,
            endpoint_url: None,
            started: false,
        }
    }

    /// The WebSocket URL for connecting Nostr clients.
    pub fn ws_url(&self) -> Option<&str> {
        self.endpoint_url.as_deref()
    }

    /// Connect a NostrClient to the local relay.
    #[cfg(feature = "nostr")]
    pub async fn nostr_client(&self) -> Result<ict_rs::nostr::NostrClient> {
        let url = self.endpoint_url.as_deref().context("relay not started")?;
        ict_rs::nostr::NostrClient::connect(url)
            .await
            .context("Failed to connect NostrClient to local relay")
    }
}

#[async_trait]
impl TerpSidecar for NostrRelaySuite {
    fn id(&self) -> &SidecarId {
        &self.id
    }

    async fn start(&mut self, _deps: &SidecarRegistry) -> Result<EndpointMap> {
        if self.started {
            return Ok(EndpointMap::new());
        }

        let runtime = self
            .runtime
            .clone()
            .context("no RuntimeBackend available for NostrRelaySuite")?;

        let mut relay = NostrRelayerManager::new(runtime, &self.id);
        let ws_url = relay.start().await?;

        info!(sidecar = %self.id, ws_url = %ws_url, "Nostr relay started");

        let mut endpoints = EndpointMap::new();
        endpoints.insert("ws".to_string(), ws_url.clone());

        self.inner = Some(relay);
        self.endpoint_url = Some(ws_url);
        self.started = true;
        Ok(endpoints)
    }

    async fn stop(&mut self) -> Result<()> {
        if let Some(ref mut relay) = self.inner {
            relay.stop().await?;
        }
        self.started = false;
        Ok(())
    }

    async fn health(&self) -> Result<HealthStatus> {
        match &self.inner {
            Some(_) => Ok(HealthStatus::Healthy),
            None => Ok(HealthStatus::Unknown),
        }
    }

    fn endpoint(&self, name: &str) -> Option<String> {
        if name == "ws" {
            self.endpoint_url.clone()
        } else {
            None
        }
    }
}
