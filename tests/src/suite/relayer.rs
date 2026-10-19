//! RelayerSuite — IBC relayer for multi-chain test environments.
//!
//! Wraps `ict_rs::relayer::HermesRelayer` into the `TerpSidecar` trait.

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use tracing::info;

use ict_rs::relayer::Relayer;
use ict_rs::runtime::RuntimeBackend;

use crate::suite::sidecar::{EndpointMap, HealthStatus, SidecarId, SidecarRegistry, TerpSidecar};

/// Configuration for the IBC relayer.
#[derive(Clone, Debug)]
pub struct RelayerConfig {
    pub relayer_type: ict_rs::relayer::RelayerType,
    pub chain_a_id: String,
    pub chain_b_id: String,
    pub chain_a_rpc: String,
    pub chain_b_rpc: String,
    pub chain_a_grpc: String,
    pub chain_b_grpc: String,
    pub mnemonic: String,
}

/// IBC relayer sidecar for linking two chains.
pub struct RelayerSuite {
    id: SidecarId,
    config: RelayerConfig,
    inner: Option<Box<dyn Relayer>>,
    pub(crate) runtime: Option<Arc<dyn RuntimeBackend>>,
    endpoints: EndpointMap,
    started: bool,
}

impl RelayerSuite {
    /// Create a new relayer suite.
    pub fn new(id: impl Into<String>, config: RelayerConfig) -> Self {
        Self {
            id: id.into(),
            config,
            inner: None,
            runtime: None,
            endpoints: EndpointMap::new(),
            started: false,
        }
    }

    /// Create a simple Hermes relayer with defaults.
    pub fn hermes(
        id: impl Into<String>,
        chain_a_id: &str,
        chain_b_id: &str,
        mnemonic: &str,
    ) -> Self {
        Self::new(
            id,
            RelayerConfig {
                relayer_type: ict_rs::relayer::RelayerType::Hermes,
                chain_a_id: chain_a_id.to_string(),
                chain_b_id: chain_b_id.to_string(),
                chain_a_rpc: format!("http://{}:26657", chain_a_id),
                chain_b_rpc: format!("http://{}:26657", chain_b_id),
                chain_a_grpc: format!("http://{}:9090", chain_a_id),
                chain_b_grpc: format!("http://{}:9090", chain_b_id),
                mnemonic: mnemonic.to_string(),
            },
        )
    }
}

#[async_trait]
impl TerpSidecar for RelayerSuite {
    fn id(&self) -> &SidecarId {
        &self.id
    }

    async fn start(&mut self, _deps: &SidecarRegistry) -> Result<EndpointMap> {
        if self.started {
            return Ok(self.endpoints.clone());
        }

        let runtime = self
            .runtime
            .clone()
            .context("no RuntimeBackend available for RelayerSuite")?;

        let relayer: Box<dyn Relayer> = match self.config.relayer_type {
            ict_rs::relayer::RelayerType::Hermes => {
                let hermes =
                    ict_rs::relayer::HermesRelayer::new(runtime, &self.id, "local").await?;
                Box::new(hermes)
            }
            ict_rs::relayer::RelayerType::CosmosRly => {
                // CosmosRly not fully wired yet — could be added later
                anyhow::bail!("CosmosRly not yet supported in RelayerSuite")
            }
            ict_rs::relayer::RelayerType::Hyperspace => {
                anyhow::bail!("Hyperspace not yet supported in RelayerSuite")
            }
        };

        self.inner = Some(relayer);

        let mut endpoints = EndpointMap::new();
        endpoints.insert("name".to_string(), self.id.clone());
        self.endpoints = endpoints.clone();
        self.started = true;

        info!(sidecar = %self.id, "Relayer started");
        Ok(endpoints)
    }

    async fn stop(&mut self) -> Result<()> {
        if let Some(ref mut relayer) = self.inner {
            relayer.stop().await?;
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
        self.endpoints.get(name).cloned()
    }
}
