//! Structured RunReport for agentic automation (text or JSON).

use crate::ibc::{DiffItem, DiffReport};
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::path::Path;

/// Capability / execution mode recorded on a report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityMode {
    Offline,
    LiveQuery,
    LiveTx,
    DockerHarness,
}

impl CapabilityMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::LiveQuery => "live-query",
            Self::LiveTx => "live-tx",
            Self::DockerHarness => "docker-harness",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "offline" => Some(Self::Offline),
            "live-query" | "live_query" => Some(Self::LiveQuery),
            "live-tx" | "live_tx" => Some(Self::LiveTx),
            "docker-harness" | "docker_harness" => Some(Self::DockerHarness),
            _ => None,
        }
    }
}

/// Env snapshot for agents (no secrets).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunEnv {
    pub mode: String,
    pub docker: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out: Option<String>,
}

/// One published or referenced artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub role: String,
}

/// Machine-readable result for every automation bin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReport {
    pub tool: String,
    pub verb: String,
    pub ok: bool,
    pub exit_code: i32,
    pub artifacts: Vec<ArtifactRef>,
    pub findings: Vec<DiffItem>,
    pub env: RunEnv,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vars: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// Exit codes for agent automation.
pub mod exit {
    pub const OK: i32 = 0;
    pub const DOMAIN: i32 = 1;
    pub const PREFLIGHT: i32 = 2;
}

impl RunReport {
    pub fn from_diff(
        tool: impl Into<String>,
        verb: impl Into<String>,
        report: &DiffReport,
        env: RunEnv,
        duration_ms: u64,
    ) -> Self {
        let ok = !report.has_errors();
        Self {
            tool: tool.into(),
            verb: verb.into(),
            ok,
            exit_code: if ok { exit::OK } else { exit::DOMAIN },
            artifacts: vec![],
            findings: report.items.clone(),
            env,
            duration_ms,
            error: if ok {
                None
            } else {
                Some("domain_failure".into())
            },
            vars: None,
            hint: None,
        }
    }

    pub fn preflight_ok(tool: impl Into<String>, mode: &CapabilityMode, duration_ms: u64) -> Self {
        Self {
            tool: tool.into(),
            verb: "preflight".into(),
            ok: true,
            exit_code: exit::OK,
            artifacts: vec![],
            findings: vec![],
            env: RunEnv {
                mode: mode.as_str().into(),
                docker: matches!(mode, CapabilityMode::DockerHarness),
                out: None,
            },
            duration_ms,
            error: None,
            vars: None,
            hint: None,
        }
    }

    pub fn missing_env(
        tool: impl Into<String>,
        verb: impl Into<String>,
        mode: &CapabilityMode,
        vars: Vec<String>,
        hint: impl Into<String>,
        duration_ms: u64,
    ) -> Self {
        Self {
            tool: tool.into(),
            verb: verb.into(),
            ok: false,
            exit_code: exit::PREFLIGHT,
            artifacts: vec![],
            findings: vec![],
            env: RunEnv {
                mode: mode.as_str().into(),
                docker: matches!(mode, CapabilityMode::DockerHarness),
                out: None,
            },
            duration_ms,
            error: Some("missing_env".into()),
            vars: Some(vars),
            hint: Some(hint.into()),
        }
    }

    pub fn with_artifacts(mut self, artifacts: Vec<ArtifactRef>) -> Self {
        self.artifacts = artifacts;
        self
    }

    pub fn emit(&self, format: OutputFormat) -> io::Result<()> {
        match format {
            OutputFormat::Json => {
                let mut out = io::stdout().lock();
                serde_json::to_writer_pretty(&mut out, self)?;
                writeln!(out)?;
            }
            OutputFormat::Text => {
                let mut out = io::stdout().lock();
                if self.ok {
                    writeln!(
                        out,
                        "✓ {} {} ok (exit {}, {} findings, {}ms)",
                        self.tool,
                        self.verb,
                        self.exit_code,
                        self.findings.len(),
                        self.duration_ms
                    )?;
                } else {
                    writeln!(
                        out,
                        "✗ {} {} failed (exit {}, error={:?})",
                        self.tool, self.verb, self.exit_code, self.error
                    )?;
                    if let Some(vars) = &self.vars {
                        writeln!(out, "  missing env: {}", vars.join(", "))?;
                    }
                    if let Some(h) = &self.hint {
                        writeln!(out, "  hint: {h}")?;
                    }
                }
                for item in &self.findings {
                    writeln!(
                        out,
                        "  [{:?}] {} {} {:?}",
                        item.severity, item.code, item.message, item.path
                    )?;
                }
                for a in &self.artifacts {
                    writeln!(out, "  artifact[{}] {}", a.role, a.path)?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Text,
    Json,
}

impl OutputFormat {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "text" => Some(Self::Text),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

/// Env vars required for a capability mode (no network probe).
pub fn required_env_for_mode(mode: &CapabilityMode) -> Vec<&'static str> {
    match mode {
        CapabilityMode::Offline => vec![],
        // Daemon path currently needs a mnemonic even for queries.
        CapabilityMode::LiveQuery => vec!["MAIN_MNEMONIC"],
        CapabilityMode::LiveTx => vec!["MAIN_MNEMONIC"],
        CapabilityMode::DockerHarness => vec![],
    }
}

/// Preflight: return missing env var names (empty = ok).
pub fn preflight_missing(mode: &CapabilityMode, require_env_file: bool) -> Vec<String> {
    let mut missing = Vec::new();
    if require_env_file {
        let dotenv_path = std::env::var("DOTENV_PATH").unwrap_or_else(|_| ".env".into());
        if !Path::new(&dotenv_path).exists()
            && !Path::new("tests/.env").exists()
            && std::env::var("MAIN_MNEMONIC").is_err()
        {
            // Soft: only require file if mode needs mnemonic and none present
            if !required_env_for_mode(mode).is_empty() {
                missing.push(format!("env_file:{dotenv_path}"));
            }
        }
    }
    for v in required_env_for_mode(mode) {
        if std::env::var(v)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .is_none()
        {
            missing.push(v.to_string());
        }
    }
    missing
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ibc::{DiffReport, DiffSeverity};

    #[test]
    fn run_report_json_shape() {
        let mut d = DiffReport::new();
        d.push(DiffSeverity::Error, "prefer_direct", "msg");
        let r = RunReport::from_diff(
            "ibc",
            "validate",
            &d,
            RunEnv {
                mode: "offline".into(),
                docker: false,
                out: Some("public".into()),
            },
            120,
        );
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["tool"], "ibc");
        assert_eq!(v["verb"], "validate");
        assert_eq!(v["ok"], false);
        assert_eq!(v["exit_code"], 1);
        assert_eq!(v["findings"][0]["code"], "prefer_direct");
        assert_eq!(v["env"]["mode"], "offline");
        assert_eq!(v["duration_ms"], 120);
    }

    #[test]
    fn missing_env_exit_2() {
        let r = RunReport::missing_env(
            "ibc",
            "preflight",
            &CapabilityMode::LiveTx,
            vec!["MAIN_MNEMONIC".into()],
            "export MAIN_MNEMONIC",
            1,
        );
        assert!(!r.ok);
        assert_eq!(r.exit_code, exit::PREFLIGHT);
        assert_eq!(r.error.as_deref(), Some("missing_env"));
    }
}
