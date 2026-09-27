//! MinioIpfsSuite — MinIO object storage with IPFS pinning via webhook collector.
//!
//! Ports: 9000 (MinIO API), 9002 (Console), 8081 (IPFS Gateway),
//!        9100 (Webhook collector), 9443 (Push daemon).
//!
//! Config pattern from `ict-rs/src/cosmos/docker_sidecar.rs` (minio_ipfs_config function).
//! Deployment pattern from `../../o-line/plays/instant-replay/`.

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;


use ict_rs::chain::SidecarConfig;
use ict_rs::runtime::{DockerImage, RuntimeBackend};

use crate::suite::sidecar::{
    DockerSidecar, EndpointMap, HealthStatus, SidecarId, SidecarRegistry, TerpSidecar,
};

// ---------------------------------------------------------------------------
// MinioIpfsSuite
// ---------------------------------------------------------------------------

/// MinIO object storage with IPFS pinning sidecar.
///
/// Manages a Docker container running minio-ipfs with webhook-based
/// auto-pinning. Provides S3-compatible API and IPFS gateway.
pub struct MinioIpfsSuite {
    id: SidecarId,
    docker: Option<DockerSidecar>,
    pub(crate) runtime: Option<Arc<dyn RuntimeBackend>>,
    config: MinioIpfsConfig,
    endpoints: EndpointMap,
    started: bool,
}

/// Configuration for MinIO-IPFS container.
#[derive(Clone, Debug)]
pub struct MinioIpfsConfig {
    pub root_user: String,
    pub root_password: String,
    pub webhook_auth_token: String,
    pub autopin_buckets: String,
}

impl Default for MinioIpfsConfig {
    fn default() -> Self {
        // LOCAL DUMMY ONLY — MinIO docker-dev defaults, not a production identity.
        Self {
            root_user: "minioadmin".into(),
            root_password: "minioadmin".into(),
            webhook_auth_token: "test-webhook-token".into(),
            autopin_buckets: "test".into(),
        }
    }
}

impl MinioIpfsSuite {
    /// Create a new MinIO-IPFS sidecar. Call `start()` with a runtime to launch.
    pub fn new(id: impl Into<String>, config: MinioIpfsConfig) -> Self {
        Self {
            id: id.into(),
            docker: None,
            runtime: None,
            config,
            endpoints: EndpointMap::new(),
            started: false,
        }
    }

    /// Create with default local credentials.
    pub fn with_defaults(id: impl Into<String>) -> Self {
        Self::new(id, MinioIpfsConfig::default())
    }

    /// Build an ict-rs SidecarConfig from our typed config.
    fn build_sidecar_config(&self) -> SidecarConfig {
        SidecarConfig {
            name: self.id.clone(),
            image: DockerImage {
                repository: "minio-ipfs".into(),
                version: "latest".into(),
                uid_gid: None,
            },
            home_dir: "/data".into(),
            ports: vec![
                "9000".into(),
                "9002".into(),
                "8081".into(),
                "9100".into(),
                "9443".into(),
            ],
            env: vec![
                ("MINIO_ROOT_USER".into(), self.config.root_user.clone()),
                ("MINIO_ROOT_PASSWORD".into(), self.config.root_password.clone()),
                ("WEBHOOK_AUTH_TOKEN".into(), self.config.webhook_auth_token.clone()),
                ("AUTOPIN_BUCKETS".into(), self.config.autopin_buckets.clone()),
            ],
            cmd: vec![],
            pre_start: false,
            validator_process: false,
            health_endpoint: Some("/minio/health/live".into()),
            ready_timeout_secs: 30,
            volumes: vec![],
        }
    }

    /// The S3 API endpoint URL.
    pub fn s3_url(&self) -> Option<String> {
        self.endpoints.get("port9000").cloned()
    }

    /// The MinIO Console URL.
    pub fn console_url(&self) -> Option<String> {
        self.endpoints.get("port9002").cloned()
    }

    /// The IPFS Gateway URL.
    pub fn ipfs_gateway_url(&self) -> Option<String> {
        self.endpoints.get("port8081").cloned()
    }
}

#[async_trait]
impl TerpSidecar for MinioIpfsSuite {
    fn id(&self) -> &SidecarId {
        &self.id
    }

    async fn start(&mut self, deps: &SidecarRegistry) -> Result<EndpointMap> {
        if self.started {
            return Ok(self.endpoints.clone());
        }

        // We need a runtime to create the container — get it from deps or
        // from the runtime field if set externally.
        let runtime = self
            .runtime
            .clone()
            .or_else(|| {
                // Try to get runtime from deps (registered by a parent suite)
                None
            })
            .context("no RuntimeBackend available for MinioIpfsSuite")?;

        let sidecar_config = self.build_sidecar_config();

        let sp = ict_rs::sidecar::SidecarProcess::new(
            sidecar_config,
            0,
            "minio",
            &self.id,
            runtime.clone(),
            ict_rs::runtime::NetworkId("local".into()),
        );

        let mut docker = DockerSidecar::new(&self.id, sp, runtime);
        let ep = docker.start(deps).await?;
        self.endpoints = ep.clone();
        self.docker = Some(docker);
        self.started = true;
        Ok(ep)
    }

    async fn stop(&mut self) -> Result<()> {
        if let Some(mut docker) = self.docker.take() {
            docker.stop().await?;
        }
        self.started = false;
        Ok(())
    }

    async fn health(&self) -> Result<HealthStatus> {
        match &self.docker {
            Some(d) => d.health().await,
            None => Ok(HealthStatus::Unknown),
        }
    }

    fn endpoint(&self, name: &str) -> Option<String> {
        self.endpoints.get(name).cloned()
    }
}