//! Core sidecar traits for the TerpNetworkSuite.
//!
//! Three runtime modes unified under one abstraction:
//! - `DockerSidecar` — full process isolation (relayer, minio-ipfs, Postgres)
//! - `SubprocessSidecar<C>` — local binary spawned from the test host
//! - `InProcessSidecar` — tokio task embedded in the test process

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::process::Command;
use tracing::{debug, info, warn};

// Spinner type removed — subprocess health-check pattern uses polling in SubprocessSidecar

// Re-export for convenience
pub use ict_rs::runtime::ContainerId;
pub use ict_rs::runtime::RuntimeBackend;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Named endpoints a sidecar exposes (e.g. `{ "http": "http://127.0.0.1:9090" }`).
pub type EndpointMap = HashMap<String, String>;

/// Sidecar health status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    Unhealthy(String),
    Unknown,
}

/// Unique identifier for a sidecar within a suite.
pub type SidecarId = String;

// ---------------------------------------------------------------------------
// TerpSidecar — the core abstraction
// ---------------------------------------------------------------------------

/// A sidecar that can be started, stopped, and queried.
///
/// Every sidecar in a `TerpNetworkSuite` implements this. The `start()` method
/// receives a reference to any already-running sidecars for dependency resolution.
#[async_trait]
pub trait TerpSidecar: Send + Sync {
    /// Unique name within the suite.
    fn id(&self) -> &SidecarId;

    /// Start the sidecar. `deps` provides access to already-running sidecars
    /// so the implementation can resolve dependency endpoints (e.g. relayer
    /// needs chain RPC URLs).
    async fn start(&mut self, deps: &SidecarRegistry) -> Result<EndpointMap>;

    /// Stop the sidecar. Idempotent.
    async fn stop(&mut self) -> Result<()>;

    /// Current health status.
    async fn health(&self) -> Result<HealthStatus>;

    /// Get a named endpoint URL. Returns `None` if the sidecar hasn't been
    /// started or doesn't expose that endpoint.
    fn endpoint(&self, name: &str) -> Option<String>;
}

// ---------------------------------------------------------------------------
// SidecarRegistry — map from sidecar name to running endpoints
// ---------------------------------------------------------------------------

/// Registry of running sidecars. Populated during suite startup.
/// Passed as `deps` to each `TerpSidecar::start()` call.
#[derive(Default, Clone)]
pub struct SidecarRegistry {
    endpoints: HashMap<SidecarId, EndpointMap>,
}

impl SidecarRegistry {
    pub fn new() -> Self {
        Self {
            endpoints: HashMap::new(),
        }
    }

    /// Register a sidecar's endpoints after it starts.
    pub fn register(&mut self, id: SidecarId, endpoints: EndpointMap) {
        self.endpoints.insert(id, endpoints);
    }

    /// Get a specific endpoint from a specific sidecar.
    pub fn get_endpoint(&self, sidecar: &str, endpoint: &str) -> Option<String> {
        self.endpoints
            .get(sidecar)
            .and_then(|m| m.get(endpoint))
            .cloned()
    }

    /// Get the full endpoint map for a sidecar.
    pub fn get_all(&self, sidecar: &str) -> Option<&EndpointMap> {
        self.endpoints.get(sidecar)
    }
}

// ---------------------------------------------------------------------------
// SubprocessConfig — config for subprocess-based sidecars
// ---------------------------------------------------------------------------

/// Configuration for a sidecar that runs as a local binary process.
pub trait SubprocessConfig: Clone + Send + Sync + 'static {
    /// Resolve the binary path. Default implementation checks:
    /// 1. `CARGO_MANIFEST_DIR`-relative paths
    /// 2. `PATH`
    fn binary_path(&self) -> Result<PathBuf>;

    /// CLI arguments passed to the binary.
    fn args(&self, config_path: &Path) -> Vec<String>;

    /// Environment variables.
    fn env(&self) -> Vec<(String, String)>;

    /// Write the configuration file(s) into `config_dir` and return the
    /// path to the primary config file (passed as last arg).
    fn write_config(&self, config_dir: &Path) -> Result<PathBuf>;

    /// Optional health check: (method, path) e.g. `("GET", "/health")`.
    /// When set, the startup loop polls `http://127.0.0.1:{port}{path}`.
    fn health_check(&self) -> Option<(String, String)>;

    /// Port the sidecar's HTTP endpoint binds to (for health checks and
    /// endpoint registration).
    fn bind_port(&self) -> u16;

    /// Timeout in seconds to wait for the health endpoint. Default 30.
    fn startup_timeout_secs(&self) -> u64 {
        30
    }
}

// ---------------------------------------------------------------------------
// SubprocessSidecar — manages a local binary subprocess
// ---------------------------------------------------------------------------

/// A sidecar that runs as a local binary process on the test host.
///
/// Handles: binary resolution, config file generation in a temp dir,
/// process spawning, health polling, and graceful shutdown.
pub struct SubprocessSidecar<C: SubprocessConfig> {
    id: SidecarId,
    config: C,
    config_dir: Option<tempfile::TempDir>,
    config_path: Option<PathBuf>,
    child: Mutex<Option<tokio::process::Child>>,
    endpoints: EndpointMap,
    started: bool,
}

impl<C: SubprocessConfig> SubprocessSidecar<C> {
    pub fn new(id: impl Into<String>, config: C) -> Self {
        Self {
            id: id.into(),
            config,
            config_dir: None,
            config_path: None,
            child: Mutex::new(None),
            endpoints: EndpointMap::new(),
            started: false,
        }
    }

    pub fn config(&self) -> &C {
        &self.config
    }

    /// Tail the subprocess stderr until we see a "listening" pattern or timeout.
    /// Returns the collected stderr text (for troubleshooting).
    async fn wait_for_listening(
        child: &mut tokio::process::Child,
        timeout: std::time::Duration,
    ) -> Result<String> {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt};

        let stderr = child.stderr.take().context("no stderr captured")?;
        let mut reader = tokio::io::BufReader::new(stderr);
        let mut buf = String::new();
        let mut collected = String::new();

        // Read lines until we see "listening" or "Listening" or timeout
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow::anyhow!(
                    "subprocess '{}' did not start within {}s. Stderr:\n{}",
                    child.id().unwrap_or(0),
                    timeout.as_secs(),
                    collected
                ));
            }

            let n = tokio::time::timeout(
                std::time::Duration::from_millis(500),
                reader.read_line(&mut buf),
            )
            .await
            .map_err(|_| ()) // timeout on line read — just check deadline
            .unwrap_or(Ok(0))?;

            if n == 0 {
                // EOF — process exited
                break;
            }

            collected.push_str(&buf);
            let lower = buf.to_lowercase();
            if lower.contains("listening") || lower.contains("ready") {
                info!(sidecar = %collected.lines().next().unwrap_or("?"), "subprocess ready");
                return Ok(collected);
            }
            buf.clear();
        }

        Ok(collected)
    }
}

#[async_trait]
impl<C: SubprocessConfig + 'static> TerpSidecar for SubprocessSidecar<C> {
    fn id(&self) -> &SidecarId {
        &self.id
    }

    async fn start(&mut self, _deps: &SidecarRegistry) -> Result<EndpointMap> {
        if self.started {
            return Ok(self.endpoints.clone());
        }

        // 1. Create temp dir for config
        let config_dir = tempfile::tempdir().context("failed to create temp dir")?;
        let config_path = self.config.write_config(config_dir.path())?;

        // 2. Resolve binary
        let binary = self.config.binary_path()?;

        // 3. Build args
        let args = self.config.args(&config_path);

        // 4. Build env
        let env = self.config.env();

        info!(
            sidecar = %self.id,
            binary = %binary.display(),
            "starting subprocess sidecar"
        );

        // 5. Spawn process
        let mut cmd = Command::new(&binary);
        cmd.args(&args);
        for (k, v) in &env {
            cmd.env(k, v);
        }
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().context("failed to spawn subprocess sidecar")?;

        // 6. Wait for health or listening signal
        let port = self.config.bind_port();
        let health = self.config.health_check();

        if let Some((_method, path)) = &health {
            let health_url = format!("http://127.0.0.1:{}{}", port, path);
            let timeout = self.config.startup_timeout_secs();

            // Try health endpoint in a loop
            let start = std::time::Instant::now();
            let mut last_err = String::new();
            loop {
                if start.elapsed().as_secs() > timeout {
                    // Kill the hung process
                    let _ = child.kill().await;
                    return Err(anyhow::anyhow!(
                        "sidecar '{}' health endpoint {health_url} not ready within {timeout}s. Last error: {last_err}",
                        self.id,
                    ));
                }

                match reqwest::get(&health_url).await {
                    Ok(resp) if resp.status().is_success() => {
                        info!(sidecar = %self.id, url = %health_url, "health check passed");
                        break;
                    }
                    Ok(resp) => {
                        last_err = format!("HTTP {}", resp.status());
                    }
                    Err(e) => {
                        last_err = e.to_string();
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        } else {
            // No health endpoint — wait for "listening" in stderr
            let _stderr = Self::wait_for_listening(
                &mut child,
                std::time::Duration::from_secs(self.config.startup_timeout_secs()),
            )
            .await
            .map_err(|e| {
                let _ = child.kill();
                e
            })?;
        }

        // 7. Register endpoints
        let mut endpoints = EndpointMap::new();
        endpoints.insert("http".to_string(), format!("http://127.0.0.1:{}", port));

        self.config_dir = Some(config_dir);
        self.config_path = Some(config_path);
        *self.child.lock().unwrap() = Some(child);
        self.endpoints = endpoints.clone();
        self.started = true;

        Ok(endpoints)
    }

    async fn stop(&mut self) -> Result<()> {
        // Extract child inside a block so the MutexGuard drops before .await
        let child_opt = self.child.lock().unwrap().take();
        if let Some(mut child) = child_opt {
            info!(sidecar = %self.id, "stopping subprocess sidecar");
            // Send SIGTERM then SIGKILL after 5s
            let _ = child.start_kill(); // SIGKILL on Unix
            match tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await {
                Ok(Ok(status)) => debug!(sidecar = %self.id, exit = %status, "process exited"),
                Ok(Err(e)) => warn!(sidecar = %self.id, error = %e, "wait failed"),
                Err(_) => {
                    warn!(sidecar = %self.id, "process did not exit in 5s, killing");
                    let _ = child.kill().await;
                }
            }
        }

        // Clean up temp dir
        if let Some(dir) = self.config_dir.take() {
            let _ = dir.close();
        }

        self.started = false;
        Ok(())
    }

    async fn health(&self) -> Result<HealthStatus> {
        // First, check child status synchronously (drop the lock before any .await)
        let has_child = {
            let guard = self.child.lock().unwrap();
            guard.is_some()
        };
        if !has_child {
            return Ok(HealthStatus::Unknown);
        }

        // Check if process has exited (synchronous)
        let exit_status = {
            let mut guard = self.child.lock().unwrap();
            match &mut *guard {
                Some(child) => child.try_wait().ok().flatten(),
                None => None,
            }
        };
        if let Some(status) = exit_status {
            return Ok(HealthStatus::Unhealthy(format!("exited with {}", status)));
        }

        // If there's an HTTP health endpoint and we got here (process alive), check it
        if let Some(http_url) = self.endpoints.get("http") {
            let health_url = format!("{}/health", http_url.trim_end_matches('/'));
            match reqwest::get(&health_url).await {
                Ok(resp) if resp.status().is_success() => Ok(HealthStatus::Healthy),
                Ok(resp) => Ok(HealthStatus::Unhealthy(format!("HTTP {}", resp.status()))),
                Err(e) => Ok(HealthStatus::Unhealthy(e.to_string())),
            }
        } else {
            Ok(HealthStatus::Healthy)
        }
    }

    fn endpoint(&self, name: &str) -> Option<String> {
        self.endpoints.get(name).cloned()
    }
}

impl<C: SubprocessConfig> Drop for SubprocessSidecar<C> {
    fn drop(&mut self) {
        if self.started && self.child.lock().unwrap().is_some() {
            warn!(
                sidecar = %self.id,
                "SubprocessSidecar dropped without calling stop()"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// DockerSidecar — wraps ict-rs SidecarProcess
// ---------------------------------------------------------------------------

/// A sidecar that runs as a Docker container via ict-rs `SidecarProcess`.
///
/// Delegates to the existing Docker lifecycle and adds endpoint registration
/// from the container's host-mapped ports.
pub struct DockerSidecar {
    id: SidecarId,
    sidecar_process: Option<ict_rs::sidecar::SidecarProcess>,
    runtime: Arc<dyn RuntimeBackend>,
    endpoints: EndpointMap,
    started: bool,
    network_id: Option<ict_rs::runtime::NetworkId>,
}

impl DockerSidecar {
    /// Create from a pre-built `SidecarProcess` (must be created but NOT started).
    /// The caller provides the `SidecarProcess` already configured; this wrapper
    /// handles the lifecycle via the TerpSidecar trait.
    pub fn new(
        id: impl Into<String>,
        sidecar_process: ict_rs::sidecar::SidecarProcess,
        runtime: Arc<dyn RuntimeBackend>,
    ) -> Self {
        Self {
            id: id.into(),
            sidecar_process: Some(sidecar_process),
            runtime,
            endpoints: EndpointMap::new(),
            started: false,
            network_id: None,
        }
    }

    /// Create from an existing container ID (e.g. an already-running Postgres).
    /// `host_ports` maps container_port -> host_port for endpoint registration.
    pub fn from_container(
        id: impl Into<String>,
        container_id: ContainerId,
        host_ports: &[(u16, u16)],
        runtime: Arc<dyn RuntimeBackend>,
    ) -> Self {
        let mut endpoints = EndpointMap::new();
        for (container_port, host_port) in host_ports {
            endpoints.insert(
                format!("port{}", container_port),
                format!("http://127.0.0.1:{}", host_port),
            );
        }
        Self {
            id: id.into(),
            sidecar_process: None,
            runtime,
            endpoints,
            started: true,
            network_id: None,
        }
    }
}

#[async_trait]
impl TerpSidecar for DockerSidecar {
    fn id(&self) -> &SidecarId {
        &self.id
    }

    async fn start(&mut self, _deps: &SidecarRegistry) -> Result<EndpointMap> {
        if self.started {
            return Ok(self.endpoints.clone());
        }

        let sp = self
            .sidecar_process
            .as_mut()
            .context("no sidecar process configured")?;

        info!(sidecar = %self.id, "starting Docker sidecar");
        sp.create_container().await?;
        sp.start_container().await?;

        // Register endpoints from container ports
        let mut endpoints = EndpointMap::new();
        if let Some(ref cid) = sp.container_id() {
            for port_str in &sp.config.ports {
                if let Ok(port) = port_str.parse::<u16>() {
                    if let Ok(Some(host_port)) = self.runtime.get_host_port(cid, port, "tcp").await
                    {
                        endpoints.insert(
                            format!("port{}", port),
                            format!("http://127.0.0.1:{}", host_port),
                        );
                    }
                }
            }
        }
        endpoints.insert("name".to_string(), sp.name());

        self.endpoints = endpoints.clone();
        self.started = true;
        Ok(endpoints)
    }

    async fn stop(&mut self) -> Result<()> {
        if let Some(mut sp) = self.sidecar_process.take() {
            info!(sidecar = %self.id, "stopping Docker sidecar");
            sp.stop_container().await?;
        }
        self.started = false;
        Ok(())
    }

    async fn health(&self) -> Result<HealthStatus> {
        match &self.sidecar_process {
            None => Ok(HealthStatus::Unknown),
            Some(sp) => {
                if let Some(ref cid) = sp.container_id() {
                    match self.runtime.exec_in_container(cid, &["true"], &[]).await {
                        Ok(_) => Ok(HealthStatus::Healthy),
                        Err(e) => Ok(HealthStatus::Unhealthy(e.to_string())),
                    }
                } else {
                    Ok(HealthStatus::Unknown)
                }
            }
        }
    }

    fn endpoint(&self, name: &str) -> Option<String> {
        self.endpoints.get(name).cloned()
    }
}

impl Drop for DockerSidecar {
    fn drop(&mut self) {
        if self.started {
            warn!(
                sidecar = %self.id,
                "DockerSidecar dropped without calling stop()"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// InProcessSidecar — wraps a tokio task handle
// ---------------------------------------------------------------------------

/// A sidecar that runs as a tokio task in the same process.
///
/// Useful for mock services, lightweight HTTP servers, or relay logic
/// that doesn't need process isolation.
pub struct InProcessSidecar {
    id: SidecarId,
    handle: Option<tokio::task::JoinHandle<()>>,
    endpoints: EndpointMap,
    started: bool,
}

impl InProcessSidecar {
    pub fn new(
        id: impl Into<String>,
        handle: tokio::task::JoinHandle<()>,
        endpoints: EndpointMap,
    ) -> Self {
        Self {
            id: id.into(),
            handle: Some(handle),
            endpoints,
            started: true,
        }
    }
}

#[async_trait]
impl TerpSidecar for InProcessSidecar {
    fn id(&self) -> &SidecarId {
        &self.id
    }

    async fn start(&mut self, _deps: &SidecarRegistry) -> Result<EndpointMap> {
        Ok(self.endpoints.clone())
    }

    async fn stop(&mut self) -> Result<()> {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
        self.started = false;
        Ok(())
    }

    async fn health(&self) -> Result<HealthStatus> {
        match &self.handle {
            Some(h) if !h.is_finished() => Ok(HealthStatus::Healthy),
            Some(_) => Ok(HealthStatus::Unhealthy("task finished".into())),
            None => Ok(HealthStatus::Unknown),
        }
    }

    fn endpoint(&self, name: &str) -> Option<String> {
        self.endpoints.get(name).cloned()
    }
}

impl Drop for InProcessSidecar {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.as_ref() {
            if !handle.is_finished() {
                handle.abort();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SidecarBinaryResolver — helper for finding binaries
// ---------------------------------------------------------------------------

/// Resolve a sidecar binary path by checking well-known locations.
pub struct SidecarBinaryResolver {
    /// Names/relative paths under CARGO_MANIFEST_DIR
    candidates: Vec<PathBuf>,
    /// Fallback to PATH lookup
    fallback_name: Option<String>,
}

impl SidecarBinaryResolver {
    /// Create a resolver that checks the given relative paths under
    /// `CARGO_MANIFEST_DIR`, then falls back to `PATH` for the given name.
    pub fn new(candidates: Vec<PathBuf>, fallback_name: Option<String>) -> Self {
        Self {
            candidates,
            fallback_name,
        }
    }

    /// Create a resolver that only looks up a name in PATH.
    pub fn path_only(name: &str) -> Self {
        Self {
            candidates: Vec::new(),
            fallback_name: Some(name.to_string()),
        }
    }

    /// Create a resolver for a cargo-built binary, checking
    /// `CARGO_MANIFEST_DIR` relative paths.
    pub fn cargo_binary(manifest_relative_paths: Vec<&str>, fallback_name: &str) -> Self {
        let candidates: Vec<PathBuf> = manifest_relative_paths
            .iter()
            .map(|p| {
                let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
                manifest.join(p)
            })
            .collect();
        Self {
            candidates,
            fallback_name: Some(fallback_name.to_string()),
        }
    }

    /// Resolve the binary path.
    pub fn resolve(&self) -> Result<PathBuf> {
        // Check candidate paths first
        for c in &self.candidates {
            if c.exists() {
                return Ok(c.canonicalize()?);
            }
        }

        // Fall back to PATH
        if let Some(ref name) = self.fallback_name {
            if let Ok(path) = which::which(name) {
                return Ok(path);
            }
        }

        Err(anyhow::anyhow!(
            "binary not found. Checked paths: {:?}, fallback: {:?}",
            self.candidates,
            self.fallback_name,
        ))
    }
}

/// Internal helper used by stderr line reading.
pub(super) mod sidecar_internal {
    pub struct Spinner;
}
