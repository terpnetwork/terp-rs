//! Host-only LLM router. Profiles never leak into the WASIX guest.
//!
//! Kinds:
//! - `openai`: OpenAI-compatible `POST {base}/chat/completions`
//! - `opencode`: OpenCode serve session API (`POST /session` + `/session/{id}/message`)
//! - `acp`: one-shot ACP subprocess (`opencode acp`, `grok agent stdio`)
//!
//! Each profile has a `timeout_ms` budget (default 45s). Timeout, transport
//! errors, empty completions, and unknown models fail over to the next profile.
//! If every profile returns 401/403, fail closed.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// Sane per-turn budget when `timeout_ms` is omitted or zero.
pub const DEFAULT_TIMEOUT_MS: u64 = 45_000;

fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProfileKind {
    Openai,
    Opencode,
    Acp,
}

impl Default for ProfileKind {
    fn default() -> Self {
        Self::Openai
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmProfile {
    pub name: String,
    #[serde(default)]
    pub kind: ProfileKind,
    /// OpenAI: origin including optional `/v1`. Unused for `acp`.
    #[serde(default)]
    pub base_url: String,
    /// ACP: executable (e.g. `opencode`, `grok`).
    #[serde(default)]
    pub command: Option<String>,
    /// ACP: argv after the executable (`["acp","--pure"]` or `["agent","stdio"]`).
    #[serde(default)]
    pub args: Option<Vec<String>>,
    /// ACP working directory (jail).
    #[serde(default)]
    pub cwd: Option<String>,
    pub model: String,
    #[serde(default)]
    pub sub_model: Option<String>,
    /// OpenCode provider id (default `opencode`).
    #[serde(default)]
    pub provider_id: Option<String>,
    /// Env var that holds the API key. Empty/missing = no Authorization header.
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// Per-turn budget. Timeout fails over to the next profile.
    /// `0` or omitted uses [`DEFAULT_TIMEOUT_MS`].
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

impl LlmProfile {
    fn request_timeout(&self) -> Duration {
        let ms = if self.timeout_ms == 0 {
            DEFAULT_TIMEOUT_MS
        } else {
            self.timeout_ms
        };
        Duration::from_millis(ms)
    }
}

/// Startup view of one profile. No secrets, no env values, no key material.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProfileHealth {
    name: String,
    kind: &'static str,
    model: String,
    usable: bool,
    detail: String,
}

#[derive(Debug, Deserialize)]
struct ProfilesFile {
    #[serde(default)]
    profiles: Vec<LlmProfile>,
}

/// Disable OpenCode coding tools so `/session/{id}/message` is one LLM turn
/// (RLM already has its own WASIX REPL). Keys are OpenCode tool ids.
const OPENCODE_TOOLS_OFF: &[(&str, bool)] = &[
    ("bash", false),
    ("edit", false),
    ("write", false),
    ("read", false),
    ("glob", false),
    ("grep", false),
    ("list", false),
    ("webfetch", false),
    ("patch", false),
    ("task", false),
    ("todowrite", false),
    ("skill", false),
];

pub struct LlmClient {
    client: reqwest::Client,
    profiles: Vec<LlmProfile>,
}

impl LlmClient {
    pub fn from_env() -> Result<Self> {
        let loaded = load_profiles()?;
        log_profile_health(&loaded.health);
        anyhow::ensure!(!loaded.profiles.is_empty(), "no LLM profiles configured");
        info!(
            count = loaded.profiles.len(),
            first = %loaded.profiles[0].name,
            kind = ?loaded.profiles[0].kind,
            "LLM router initialized"
        );
        Self::with_profiles(loaded.profiles)
    }

    fn with_profiles(profiles: Vec<LlmProfile>) -> Result<Self> {
        // Client timeout is a ceiling. Each request also sets the profile budget,
        // which replaces this value, and `chat_one` enforces the same budget.
        let cap_ms = profiles
            .iter()
            .map(|p| p.request_timeout().as_millis() as u64)
            .max()
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .max(1);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(cap_ms))
            .build()
            .context("Failed to create HTTP client")?;
        Ok(Self { client, profiles })
    }

    pub fn profile_names(&self) -> Vec<String> {
        self.profiles.iter().map(|p| p.name.clone()).collect()
    }

    pub async fn chat(&self, messages: &[Message], model_override: Option<&str>) -> Result<String> {
        let mut last_err: Option<anyhow::Error> = None;
        let mut auth_fails = 0usize;
        for profile in &self.profiles {
            info!(profile = %profile.name, kind = ?profile.kind, "LLM turn");
            match self.chat_one(profile, messages, model_override).await {
                Ok(s) if !s.trim().is_empty() => return Ok(s),
                Ok(_) => {
                    warn!(profile = %profile.name, "LLM empty completion; trying next profile");
                    last_err = Some(anyhow::anyhow!("empty completion from {}", profile.name));
                }
                Err(e) if is_auth_error(&e) => {
                    auth_fails += 1;
                    warn!(profile = %profile.name, error = %e, "LLM 401/403; rotating");
                    last_err = Some(e);
                }
                Err(e) => {
                    warn!(profile = %profile.name, error = %e, cause = format!("{e:#}"), "LLM request failed");
                    last_err = Some(e);
                }
            }
        }
        if auth_fails == self.profiles.len() {
            bail!("all LLM profiles returned 401/403 (fail closed)");
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("all LLM profiles failed")))
    }

    /// Probe every profile on its own (no failover) and time each one.
    pub async fn chat_each(
        &self,
        messages: &[Message],
    ) -> Vec<(String, Duration, Result<String>)> {
        let mut out = Vec::new();
        for profile in &self.profiles {
            let t = std::time::Instant::now();
            let r = self.chat_one(profile, messages, None).await;
            out.push((profile.name.clone(), t.elapsed(), r));
        }
        out
    }

    pub async fn sub_query(&self, prompt: &str) -> Result<String> {
        let messages = vec![Message {
            role: "user".to_string(),
            content: prompt.to_string(),
        }];
        self.chat(&messages, None).await
    }

    async fn chat_one(
        &self,
        profile: &LlmProfile,
        messages: &[Message],
        model_override: Option<&str>,
    ) -> Result<String> {
        let budget = profile.request_timeout();
        let turn = async {
            match profile.kind {
                ProfileKind::Openai => self.chat_openai(profile, messages, model_override).await,
                ProfileKind::Opencode => {
                    self.chat_opencode(profile, messages, model_override).await
                }
                ProfileKind::Acp => self.chat_acp(profile, messages, model_override).await,
            }
        };
        match tokio::time::timeout(budget, turn).await {
            Ok(res) => res,
            Err(_) => bail!(
                "LLM profile {} timed out after {}ms",
                profile.name,
                budget.as_millis()
            ),
        }
    }

    async fn chat_openai(
        &self,
        profile: &LlmProfile,
        messages: &[Message],
        model_override: Option<&str>,
    ) -> Result<String> {
        let model = model_override.unwrap_or(&profile.model);
        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "temperature": 0.3,
            "max_tokens": 2048,
        });
        let mut req = self
            .client
            .post(openai_endpoint(&profile.base_url))
            .timeout(profile.request_timeout())
            .json(&body);
        if let Some(key) = api_key(profile) {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
        let resp = req.send().await.context("LLM request failed")?;
        let status = resp.status();
        let text = resp.text().await.context("Failed to read LLM response")?;
        if status.as_u16() == 401 || status.as_u16() == 403 {
            bail!("auth {}: {text}", status.as_u16());
        }
        if !status.is_success() {
            bail!("LLM HTTP {status}: {text}");
        }
        let json: serde_json::Value =
            serde_json::from_str(&text).context("Failed to parse LLM JSON")?;
        Ok(json["choices"]
            .get(0)
            .and_then(|c| c["message"]["content"].as_str())
            .unwrap_or("")
            .to_string())
    }

    async fn chat_acp(
        &self,
        profile: &LlmProfile,
        messages: &[Message],
        model_override: Option<&str>,
    ) -> Result<String> {
        let command = profile
            .command
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("ACP profile {} missing command", profile.name))?;
        let args = profile.args.clone().unwrap_or_default();
        let model = model_override
            .or(Some(profile.model.as_str()))
            .filter(|s| !s.is_empty());
        crate::llm_acp::chat_acp(
            crate::llm_acp::AcpSpec {
                command,
                args: &args,
                cwd: profile.cwd.as_deref(),
                model,
                timeout: profile.request_timeout(),
            },
            messages,
        )
        .await
    }

    /// Legacy OpenCode HTTP serve (agent sessions). Prefer `kind = "acp"`.
    async fn chat_opencode(
        &self,
        profile: &LlmProfile,
        messages: &[Message],
        model_override: Option<&str>,
    ) -> Result<String> {
        let origin = profile.base_url.trim_end_matches('/');
        let model =
            model_override.unwrap_or(profile.sub_model.as_deref().unwrap_or(&profile.model));
        let (provider_id, model_id) =
            split_opencode_model(profile.provider_id.as_deref().unwrap_or("opencode"), model);
        let prompt = flatten_messages(messages);

        // `plan` is a primary agent that does not run the coding tool loop.
        // Default `build` treated our RLM prompt as a coding task (bash/edit
        // for 20+ steps) and the blocking POST timed out — that is not a
        // mesh/network miss; OpenCode is loopback on the bot host.
        let mut create = self
            .client
            .post(format!("{origin}/session"))
            .timeout(profile.request_timeout())
            .json(&serde_json::json!({
                "title": "edgar-rlm",
                "agent": "plan",
                "model": {"id": model_id, "providerID": provider_id},
                "permission": {
                    "*": "deny",
                    "bash": "deny",
                    "edit": "deny",
                    "write": "deny",
                    "read": "deny",
                    "external_directory": "deny"
                },
            }));
        if let Some(key) = api_key(profile) {
            create = create.header("Authorization", format!("Bearer {key}"));
        }
        let created = create
            .send()
            .await
            .context("opencode session create failed")?;
        let st = created.status();
        let created_text = created
            .text()
            .await
            .context("opencode session create body")?;
        if st.as_u16() == 401 || st.as_u16() == 403 {
            bail!("auth {}: {created_text}", st.as_u16());
        }
        if !st.is_success() {
            bail!("opencode session HTTP {st}: {created_text}");
        }
        let created_json: serde_json::Value =
            serde_json::from_str(&created_text).context("opencode session JSON")?;
        let sid = created_json["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("opencode session missing id"))?;

        let tools: serde_json::Map<String, serde_json::Value> = OPENCODE_TOOLS_OFF
            .iter()
            .map(|(k, v)| ((*k).to_string(), serde_json::Value::Bool(*v)))
            .collect();
        let mut msg = self
            .client
            .post(format!("{origin}/session/{sid}/message"))
            .timeout(profile.request_timeout())
            .json(&serde_json::json!({
                "agent": "plan",
                "tools": tools,
                "parts": [{"type": "text", "text": prompt}],
                "model": {"providerID": provider_id, "modelID": model_id},
            }));
        if let Some(key) = api_key(profile) {
            msg = msg.header("Authorization", format!("Bearer {key}"));
        }
        let resp = msg
            .send()
            .await
            .with_context(|| format!("opencode message failed ({origin}/session/{sid}/message)"))?;
        let st = resp.status();
        let text = resp.text().await.context("opencode message body")?;
        if st.as_u16() == 401 || st.as_u16() == 403 {
            bail!("auth {}: {text}", st.as_u16());
        }
        if !st.is_success() {
            bail!("opencode message HTTP {st}: {text}");
        }
        let json: serde_json::Value =
            serde_json::from_str(&text).context("opencode message JSON")?;
        Ok(extract_opencode_text(&json))
    }
}

struct LoadedProfiles {
    profiles: Vec<LlmProfile>,
    health: Vec<ProfileHealth>,
}

fn load_profiles() -> Result<LoadedProfiles> {
    let path = std::env::var("LLM_PROFILES")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "llm-profiles.toml".into());
    if let Ok(raw) = std::fs::read_to_string(&path) {
        let mut loaded = resolve_profile_file(&raw).with_context(|| format!("parse {path}"))?;
        if !loaded.profiles.is_empty() {
            return Ok(loaded);
        }
        // Every entry was unusable (missing ACP binary, empty file). Do not crash.
        warn!("no usable LLM profiles in file; using env fallback");
        let fb = env_fallback_profile();
        loaded.health.push(assess_profile(&fb));
        loaded.profiles = vec![fb];
        return Ok(loaded);
    }
    let fb = env_fallback_profile();
    let health = vec![assess_profile(&fb)];
    Ok(LoadedProfiles {
        profiles: vec![fb],
        health,
    })
}

fn resolve_profile_file(raw: &str) -> Result<LoadedProfiles> {
    let file: ProfilesFile = toml::from_str(raw)?;
    let health: Vec<ProfileHealth> = file.profiles.iter().map(assess_profile).collect();
    let profiles = file
        .profiles
        .into_iter()
        .zip(health.iter())
        .filter(|(_, h)| h.usable)
        .map(|(p, _)| p)
        .collect();
    Ok(LoadedProfiles { profiles, health })
}

fn assess_profile(p: &LlmProfile) -> ProfileHealth {
    let kind = match p.kind {
        ProfileKind::Openai => "openai",
        ProfileKind::Opencode => "opencode",
        ProfileKind::Acp => "acp",
    };
    let mut row = ProfileHealth {
        name: p.name.clone(),
        kind,
        model: p.model.clone(),
        usable: true,
        detail: "ok".into(),
    };
    if p.kind == ProfileKind::Acp {
        let Some(cmd) = p.command.as_deref().filter(|s| !s.is_empty()) else {
            row.usable = false;
            row.detail = "missing command".into();
            return row;
        };
        if !command_on_path(cmd) {
            row.usable = false;
            row.detail = "command not on PATH".into();
            return row;
        }
        if let Some(cwd) = p.cwd.as_deref().filter(|s| !s.is_empty()) {
            if !std::path::Path::new(cwd).is_dir() {
                // Still selected. Spawn failure fails over; startup must not die
                // because the jail directory is absent on this host.
                row.detail = "cwd missing; request will fail over".into();
            }
        }
    }
    row
}

fn log_profile_health(rows: &[ProfileHealth]) {
    info!(
        checked = rows.len(),
        "LLM profile health: timeout or a missing model fails over; startup keeps going"
    );
    for row in rows {
        if row.usable {
            info!(
                profile = %row.name,
                kind = row.kind,
                model = %row.model,
                detail = %row.detail,
                "LLM profile loaded"
            );
        } else {
            warn!(
                profile = %row.name,
                kind = row.kind,
                model = %row.model,
                detail = %row.detail,
                "LLM profile not usable"
            );
        }
    }
}

fn env_fallback_profile() -> LlmProfile {
    // Compat: single profile from env (empty LLM_BASE_URL used to default to LM Studio).
    let base_url = dotenv::var("LLM_BASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:4096".to_string());
    let model =
        dotenv::var("LLM_MODEL").unwrap_or_else(|_| "muse-spark-1.3-contributor-free".to_string());
    let sub_model = dotenv::var("LLM_SUB_MODEL").ok().filter(|s| !s.is_empty());
    let kind =
        if base_url.contains("/v1") || dotenv::var("LLM_KIND").ok().as_deref() == Some("openai") {
            ProfileKind::Openai
        } else {
            ProfileKind::Opencode
        };
    LlmProfile {
        name: "env".into(),
        kind,
        base_url,
        command: None,
        args: None,
        cwd: None,
        model,
        sub_model,
        provider_id: dotenv::var("LLM_PROVIDER_ID").ok(),
        api_key_env: Some("LLM_API_KEY".into()),
        timeout_ms: DEFAULT_TIMEOUT_MS,
    }
}

fn command_on_path(cmd: &str) -> bool {
    if cmd.contains('/') {
        return std::path::Path::new(cmd).is_file();
    }
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(cmd).is_file())
}

fn api_key(profile: &LlmProfile) -> Option<String> {
    let name = profile.api_key_env.as_deref().unwrap_or("LLM_API_KEY");
    std::env::var(name).ok().filter(|k| !k.is_empty())
}

fn openai_endpoint(base: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else if base.ends_with("/v1") {
        format!("{base}/chat/completions")
    } else {
        format!("{base}/v1/chat/completions")
    }
}

fn split_opencode_model<'a>(default_provider: &'a str, model: &'a str) -> (&'a str, &'a str) {
    if let Some((p, m)) = model.split_once('/') {
        if p == "opencode" || p == "zen" {
            return (p, m);
        }
        // `opencode/muse-spark-1.3-contributor-free`
        return (p, m);
    }
    (default_provider, model)
}

pub(crate) fn flatten_messages(messages: &[Message]) -> String {
    let mut out = String::new();
    for m in messages {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&format!("[{}]\n{}", m.role, m.content));
    }
    out
}

fn extract_opencode_text(json: &serde_json::Value) -> String {
    let mut texts = Vec::new();
    if let Some(parts) = json["parts"].as_array() {
        for p in parts {
            if p["type"].as_str() == Some("text") {
                if let Some(t) = p["text"].as_str() {
                    if !t.is_empty() {
                        texts.push(t.to_string());
                    }
                }
            }
        }
    }
    texts.join("\n")
}

fn is_auth_error(e: &anyhow::Error) -> bool {
    let s = format!("{e:#}");
    s.contains("auth 401") || s.contains("auth 403")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_opencode_profile() {
        let raw = r#"
[[profiles]]
name = "muse-free"
kind = "opencode"
base_url = "http://127.0.0.1:4096"
model = "muse-spark-1.3-contributor-free"
provider_id = "opencode"
"#;
        let f: ProfilesFile = toml::from_str(raw).unwrap();
        assert_eq!(f.profiles[0].kind, ProfileKind::Opencode);
        assert_eq!(f.profiles[0].model, "muse-spark-1.3-contributor-free");
        assert_eq!(f.profiles[0].timeout_ms, DEFAULT_TIMEOUT_MS);
    }

    #[test]
    fn parse_acp_profile() {
        let raw = r#"
[[profiles]]
name = "muse-free"
kind = "acp"
command = "opencode"
args = ["acp", "--pure"]
model = "opencode/muse-spark-1.3-contributor-free"
"#;
        let f: ProfilesFile = toml::from_str(raw).unwrap();
        assert_eq!(f.profiles[0].kind, ProfileKind::Acp);
        assert_eq!(f.profiles[0].command.as_deref(), Some("opencode"));
        assert_eq!(f.profiles[0].timeout_ms, DEFAULT_TIMEOUT_MS);
        assert_eq!(
            f.profiles[0].request_timeout(),
            Duration::from_millis(DEFAULT_TIMEOUT_MS)
        );
    }

    #[test]
    fn example_profiles_are_free_acp_in_failover_order() {
        let raw = include_str!("../llm-profiles.toml.example");
        let file: ProfilesFile = toml::from_str(raw).unwrap();
        let models: Vec<&str> = file.profiles.iter().map(|p| p.model.as_str()).collect();
        assert_eq!(
            models,
            vec![
                "opencode/deepseek-v4.1-flash-free",
                "opencode/muse-spark-1.3-contributor-free",
                "opencode/nemotron-3.5-lightning-free",
            ]
        );
        for p in &file.profiles {
            assert_eq!(p.kind, ProfileKind::Acp);
            assert_eq!(p.command.as_deref(), Some("opencode"));
            assert_eq!(p.cwd.as_deref(), Some("/opt/terp-bot/opencode-jail"));
            assert_eq!(p.timeout_ms, DEFAULT_TIMEOUT_MS);
            assert_eq!(
                p.args.as_ref().map(|a| a.as_slice()),
                Some(["acp".to_string(), "--pure".to_string()].as_slice())
            );
        }
        let loaded = resolve_profile_file(raw).unwrap();
        let reported: Vec<&str> = loaded.health.iter().map(|h| h.model.as_str()).collect();
        assert_eq!(reported, models);
        log_profile_health(&loaded.health);
    }

    #[test]
    fn health_check_reports_a_missing_profile_and_keeps_the_rest() {
        let raw = r#"
[[profiles]]
name = "deepseek-v4.1-flash-free"
kind = "acp"
command = "/bin/echo"
model = "opencode/deepseek-v4.1-flash-free"
cwd = "/no/such/edgar-opencode-jail"

[[profiles]]
name = "muse-spark-1.3-contributor-free"
kind = "acp"
command = "no-such-edgar-acp-bin"
model = "opencode/muse-spark-1.3-contributor-free"

[[profiles]]
name = "broken-missing-command"
kind = "acp"
model = "opencode/muse-spark-1.3-contributor-free"

[[profiles]]
name = "nemotron-3.5-lightning-free"
kind = "acp"
command = "/bin/echo"
args = ["acp", "--pure"]
model = "opencode/nemotron-3.5-lightning-free"
"#;
        let loaded = resolve_profile_file(raw).unwrap();
        assert_eq!(loaded.health.len(), 4);
        assert!(loaded.health[0].usable);
        assert_eq!(
            loaded.health[0].detail,
            "cwd missing; request will fail over"
        );
        assert!(!loaded.health[1].usable);
        assert_eq!(loaded.health[1].detail, "command not on PATH");
        assert!(!loaded.health[2].usable);
        assert_eq!(loaded.health[2].detail, "missing command");
        assert!(loaded.health[3].usable);
        let names: Vec<&str> = loaded.profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["deepseek-v4.1-flash-free", "nemotron-3.5-lightning-free",]
        );
        log_profile_health(&loaded.health);
    }

    #[test]
    fn timeout_and_missing_model_are_not_auth_failures() {
        let timed = anyhow::anyhow!("LLM profile deepseek-v4.1-flash-free timed out after 400ms");
        let missing = anyhow::anyhow!("ACP session/new error: model not found");
        assert!(!is_auth_error(&timed));
        assert!(!is_auth_error(&missing));
        assert!(is_auth_error(&anyhow::anyhow!(
            "auth 401 ACP session/new: denied"
        )));
    }

    #[tokio::test]
    async fn http_timeout_fails_over_to_next_profile() {
        let hang = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let hang_port = hang.local_addr().unwrap().port();
        let ok = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ok_port = ok.local_addr().unwrap().port();
        tokio::spawn(serve_hang(hang));
        tokio::spawn(serve_http(
            ok,
            http_json(
                200,
                "OK",
                r#"{"choices":[{"message":{"content":"from-fallback"}}]}"#,
            ),
        ));

        let client = LlmClient::with_profiles(vec![
            openai_profile("hang", &format!("http://127.0.0.1:{hang_port}"), 400),
            openai_profile("ok", &format!("http://127.0.0.1:{ok_port}"), 2_000),
        ])
        .unwrap();
        let started = std::time::Instant::now();
        let text = client
            .chat(
                &[Message {
                    role: "user".into(),
                    content: "ping".into(),
                }],
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "from-fallback");
        assert!(
            started.elapsed() < Duration::from_millis(1500),
            "http timeout did not fail over quickly: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn acp_timeout_fails_over_to_next_profile() {
        let ok = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ok_port = ok.local_addr().unwrap().port();
        tokio::spawn(serve_http(
            ok,
            http_json(
                200,
                "OK",
                r#"{"choices":[{"message":{"content":"after-acp-timeout"}}]}"#,
            ),
        ));
        let client = LlmClient::with_profiles(vec![
            LlmProfile {
                name: "slow-acp".into(),
                kind: ProfileKind::Acp,
                base_url: String::new(),
                command: Some("/bin/sleep".into()),
                args: Some(vec!["5".into()]),
                cwd: None,
                model: "opencode/deepseek-v4.1-flash-free".into(),
                sub_model: None,
                provider_id: None,
                api_key_env: None,
                timeout_ms: 400,
            },
            openai_profile("ok", &format!("http://127.0.0.1:{ok_port}"), 2_000),
        ])
        .unwrap();
        let started = std::time::Instant::now();
        let text = client
            .chat(
                &[Message {
                    role: "user".into(),
                    content: "ping".into(),
                }],
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "after-acp-timeout");
        assert!(
            started.elapsed() < Duration::from_millis(1500),
            "acp timeout did not fail over quickly: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn missing_model_http_fails_over() {
        let missing = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let missing_port = missing.local_addr().unwrap().port();
        let ok = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ok_port = ok.local_addr().unwrap().port();
        tokio::spawn(serve_http(
            missing,
            http_json(404, "Not Found", r#"{"error":"model not found"}"#),
        ));
        tokio::spawn(serve_http(
            ok,
            http_json(
                200,
                "OK",
                r#"{"choices":[{"message":{"content":"from-next"}}]}"#,
            ),
        ));
        let client = LlmClient::with_profiles(vec![
            openai_profile(
                "missing-model",
                &format!("http://127.0.0.1:{missing_port}"),
                2_000,
            ),
            openai_profile("ok", &format!("http://127.0.0.1:{ok_port}"), 2_000),
        ])
        .unwrap();
        let text = client
            .chat(
                &[Message {
                    role: "user".into(),
                    content: "ping".into(),
                }],
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "from-next");
    }

    fn openai_profile(name: &str, base_url: &str, timeout_ms: u64) -> LlmProfile {
        LlmProfile {
            name: name.into(),
            kind: ProfileKind::Openai,
            base_url: base_url.into(),
            command: None,
            args: None,
            cwd: None,
            model: "opencode/deepseek-v4.1-flash-free".into(),
            sub_model: None,
            provider_id: None,
            api_key_env: None,
            timeout_ms,
        }
    }

    fn http_json(status: u16, reason: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    async fn serve_hang(listener: tokio::net::TcpListener) {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = tokio::io::AsyncReadExt::read(&mut sock, &mut buf).await;
                std::future::pending::<()>().await;
            });
        }
    }

    async fn serve_http(listener: tokio::net::TcpListener, response: String) {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let response = response.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = tokio::time::timeout(
                    Duration::from_secs(2),
                    tokio::io::AsyncReadExt::read(&mut sock, &mut buf),
                )
                .await;
                let _ = tokio::io::AsyncWriteExt::write_all(&mut sock, response.as_bytes()).await;
                let _ = tokio::io::AsyncWriteExt::shutdown(&mut sock).await;
            });
        }
    }

    #[test]
    fn split_model_id() {
        assert_eq!(
            split_opencode_model("opencode", "muse-spark-1.3-contributor-free"),
            ("opencode", "muse-spark-1.3-contributor-free")
        );
        assert_eq!(
            split_opencode_model("opencode", "opencode/muse-spark-1.3-contributor-free"),
            ("opencode", "muse-spark-1.3-contributor-free")
        );
    }

    #[test]
    fn flatten_and_extract() {
        let msgs = vec![
            Message {
                role: "system".into(),
                content: "be brief".into(),
            },
            Message {
                role: "user".into(),
                content: "hi".into(),
            },
        ];
        let f = flatten_messages(&msgs);
        assert!(f.contains("[system]"));
        let json = serde_json::json!({
            "parts": [
                {"type": "step-start"},
                {"type": "text", "text": "pong"}
            ]
        });
        assert_eq!(extract_opencode_text(&json), "pong");
    }
}
