//! IndexerSuite — Argus state-diff indexer for Cosmos blockchain data.
//!
//! Argus is a Node.js application that indexes CosmWasm state changes,
//! not just events. It consists of 7+ services:
//! - Server (API)
//! - Listener (event watcher)
//! - Workers (background processing)
//! - Workers-bg (background queue)
//! - Account webhooks
//! - Accounts server
//!
//! Plus 4 infrastructure containers:
//! - Postgres accounts DB
//! - TimescaleDB data DB
//! - Redis (queue)
//! - MeiliSearch (full-text search)
//!
//! This suite manages the full fleet or a minimal subset.

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use tracing::info;

use ict_rs::runtime::{ContainerId, ContainerOptions, DockerImage, PortBinding, RuntimeBackend};

use crate::suite::sidecar::{EndpointMap, HealthStatus, SidecarId, SidecarRegistry, TerpSidecar};

/// Configuration for the Argus indexer.
#[derive(Clone, Debug)]
pub struct IndexerConfig {
    pub local_rpc: String,
    pub bech32_prefix: String,
    pub db_user: String,
    pub db_password: String,
    pub meili_master_key: String,
    pub api_port: u16,
    pub debug_port: u16,
}

impl Default for IndexerConfig {
    fn default() -> Self {
        Self {
            local_rpc: "http://127.0.0.1:26657".into(),
            bech32_prefix: "terp".into(),
            db_user: "dev".into(),
            db_password: "dev".into(),
            meili_master_key: "masterKey".into(),
            api_port: 3420,
            debug_port: 9227,
        }
    }
}

/// Argus indexer fleet state.
struct IndexerFleet {
    data_db: Option<ContainerId>,
    accounts_db: Option<ContainerId>,
    redis: Option<ContainerId>,
    meilisearch: Option<ContainerId>,
}

/// IndexerSuite — manages the Argus indexer fleet.
pub struct IndexerSuite {
    id: SidecarId,
    config: IndexerConfig,
    pub(crate) runtime: Option<Arc<dyn RuntimeBackend>>,
    endpoints: EndpointMap,
    fleet: Option<IndexerFleet>,
    started: bool,
    minimal: bool,
}

impl IndexerSuite {
    /// Create a new indexer suite with the given config.
    pub fn new(id: impl Into<String>, config: IndexerConfig) -> Self {
        Self {
            id: id.into(),
            config,
            runtime: None,
            endpoints: EndpointMap::new(),
            fleet: None,
            started: false,
            minimal: false,
        }
    }

    /// Create with default configuration for a local test.
    pub fn with_defaults(id: impl Into<String>) -> Self {
        Self::new(id, IndexerConfig::default())
    }

    /// Use minimal infrastructure (server + data backend only).
    /// Eliminates Redis and MeiliSearch requirements for quick tests.
    pub fn with_minimal(mut self) -> Self {
        self.minimal = true;
        self
    }

    fn container_name(&self, suffix: &str) -> String {
        format!("{}-{}", self.id, suffix)
    }

    // Helper to create infrastructure containers
    async fn start_infra(&mut self, runtime: &Arc<dyn RuntimeBackend>) -> Result<IndexerFleet> {
        let mut fleet = IndexerFleet {
            data_db: None,
            accounts_db: None,
            redis: None,
            meilisearch: None,
        };

        // 1. TimescaleDB (data backend)
        let data_opts = ContainerOptions {
            image: DockerImage {
                repository: "timescale/timescaledb".into(),
                version: "2.18.1-pg17".into(),
                uid_gid: None,
            },
            name: self.container_name("db-data"),
            network_id: None,
            env: vec![
                ("POSTGRES_DB".into(), "dev_data".into()),
                ("POSTGRES_USER".into(), self.config.db_user.clone()),
                ("POSTGRES_PASSWORD".into(), self.config.db_password.clone()),
            ],
            cmd: vec![],
            entrypoint: None,
            ports: vec![PortBinding {
                host_port: 0,
                container_port: 5432,
                protocol: "tcp".into(),
            }],
            volumes: vec![],
            labels: vec![],
            hostname: None,
        };
        let data_id = runtime.create_container(&data_opts).await?;
        runtime.start_container(&data_id).await?;
        fleet.data_db = Some(data_id);
        info!("TimescaleDB container started");

        // 2. Postgres (accounts backend)
        let acct_opts = ContainerOptions {
            image: DockerImage {
                repository: "postgres".into(),
                version: "17-alpine".into(),
                uid_gid: None,
            },
            name: self.container_name("db-accounts"),
            network_id: None,
            env: vec![
                ("POSTGRES_DB".into(), "dev_accounts".into()),
                ("POSTGRES_USER".into(), self.config.db_user.clone()),
                ("POSTGRES_PASSWORD".into(), self.config.db_password.clone()),
            ],
            cmd: vec![],
            entrypoint: None,
            ports: vec![PortBinding {
                host_port: 0,
                container_port: 5432,
                protocol: "tcp".into(),
            }],
            volumes: vec![],
            labels: vec![],
            hostname: None,
        };
        let acct_id = runtime.create_container(&acct_opts).await?;
        runtime.start_container(&acct_id).await?;
        fleet.accounts_db = Some(acct_id);
        info!("Postgres accounts container started");

        if !self.minimal {
            // 3. Redis
            let redis_opts = ContainerOptions {
                image: DockerImage {
                    repository: "redis".into(),
                    version: "7-alpine".into(),
                    uid_gid: None,
                },
                name: self.container_name("redis"),
                network_id: None,
                env: vec![],
                cmd: vec![],
                entrypoint: None,
                ports: vec![PortBinding {
                    host_port: 0,
                    container_port: 6379,
                    protocol: "tcp".into(),
                }],
                volumes: vec![],
                labels: vec![],
                hostname: None,
            };
            let redis_id = runtime.create_container(&redis_opts).await?;
            runtime.start_container(&redis_id).await?;
            fleet.redis = Some(redis_id);
            info!("Redis container started");

            // 4. MeiliSearch
            let meili_opts = ContainerOptions {
                image: DockerImage {
                    repository: "getmeili/meilisearch".into(),
                    version: "latest".into(),
                    uid_gid: None,
                },
                name: self.container_name("meilisearch"),
                network_id: None,
                env: vec![(
                    "MEILI_MASTER_KEY".into(),
                    self.config.meili_master_key.clone(),
                )],
                cmd: vec![],
                entrypoint: None,
                ports: vec![PortBinding {
                    host_port: 0,
                    container_port: 7700,
                    protocol: "tcp".into(),
                }],
                volumes: vec![],
                labels: vec![],
                hostname: None,
            };
            let meili_id = runtime.create_container(&meili_opts).await?;
            runtime.start_container(&meili_id).await?;
            fleet.meilisearch = Some(meili_id);
            info!("MeiliSearch container started");
        }

        Ok(fleet)
    }

    /// The API URL for querying indexed chain data.
    pub fn api_url(&self) -> Option<String> {
        self.endpoints.get("api").cloned()
    }
}

#[async_trait]
impl TerpSidecar for IndexerSuite {
    fn id(&self) -> &SidecarId {
        &self.id
    }

    async fn start(&mut self, deps: &SidecarRegistry) -> Result<EndpointMap> {
        if self.started {
            return Ok(self.endpoints.clone());
        }

        let runtime = self
            .runtime
            .clone()
            .context("no RuntimeBackend available for IndexerSuite")?;

        // Start infrastructure containers (Postgres, TimescaleDB, Redis, MeiliSearch)
        let fleet = self.start_infra(&runtime).await?;

        // Wait for DBs to be healthy (short sleep — containers start fast)
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // Register endpoints
        let mut endpoints = EndpointMap::new();
        endpoints.insert(
            "api".to_string(),
            format!("http://127.0.0.1:{}", self.config.api_port),
        );

        self.fleet = Some(fleet);
        self.endpoints = endpoints.clone();
        self.started = true;

        info!(
            sidecar = %self.id,
            minimal = %self.minimal,
            "Indexer suite started"
        );

        Ok(endpoints)
    }

    async fn stop(&mut self) -> Result<()> {
        if let Some(fleet) = self.fleet.take() {
            let runtime = self.runtime.as_ref().context("no runtime")?;

            async fn stop_container(runtime: &Arc<dyn RuntimeBackend>, id: &ContainerId) {
                let _ = runtime.stop_container(id).await;
                let _ = runtime.remove_container(id).await;
            }

            if let Some(ref id) = fleet.meilisearch {
                stop_container(runtime, id).await;
            }
            if let Some(ref id) = fleet.redis {
                stop_container(runtime, id).await;
            }
            if let Some(ref id) = fleet.accounts_db {
                stop_container(runtime, id).await;
            }
            if let Some(ref id) = fleet.data_db {
                stop_container(runtime, id).await;
            }
        }

        self.started = false;
        Ok(())
    }

    async fn health(&self) -> Result<HealthStatus> {
        match &self.fleet {
            Some(_) => Ok(HealthStatus::Healthy),
            None => Ok(HealthStatus::Unknown),
        }
    }

    fn endpoint(&self, name: &str) -> Option<String> {
        self.endpoints.get(name).cloned()
    }
}
