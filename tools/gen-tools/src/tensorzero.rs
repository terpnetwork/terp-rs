//! TensorZero function spec generator.
//!
//! Port of o-line/scripts/gen/gen-tz.py into the gen-tools Rust pipeline.
//! Transforms CosmWasm contract schema types into TensorZero function specs:
//!   - functions/<name>/args_schema.json   — JSON Schema for arguments
//!   - functions/<name>/system.minijinja   — system prompt template
//!   - functions/<name>/user.minijinja     — user prompt template
//!   - tensorzero.toml                     — top-level config with functions + model routing
//!   - llms.txt                            — index of generated functions
//!   - episodes/                           — episode configs from markdown
//!
//! Per-project configuration lives in gen-tools.yaml:
//!   tz_heuristics: "path/to/tz-heuristics.toml"
//!   tz_episodes:   "path/to/episodes/dir"
//!   tz_out:        "path/to/output"
//!   tz_model:      { name, routing, provider_type, model_name, api_base }

use crate::config::GenerationContext;
use crate::resolver::SourceResolver;
use crate::{GenerationResult, Generator};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

// ─── Heuristics (tz-heuristics.toml format) ─────────────────────────────────

/// Parsed tz-heuristics.toml — drives categorization, templates, model config.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct TzHeuristics {
    #[serde(default)]
    pub categories: BTreeMap<String, Vec<String>>,

    #[serde(default)]
    pub secrets: SecretsConfig,

    #[serde(default)]
    pub templates: TemplatesConfig,

    #[serde(default)]
    pub model: ModelConfig,

    #[serde(default)]
    pub function_hints: BTreeMap<String, FunctionHint>,

    #[serde(default)]
    pub extra_functions: BTreeMap<String, ExtraFunction>,

    #[serde(default)]
    pub targets: BTreeMap<String, TargetConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SecretsConfig {
    #[serde(default)]
    pub patterns: Vec<String>,
    #[serde(default)]
    pub action: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TemplatesConfig {
    #[serde(default = "default_system_prefix")]
    pub system_prefix: String,
    #[serde(default = "default_user_format")]
    pub user_format: String,
    #[serde(default)]
    pub category_prompts: BTreeMap<String, String>,
}

fn default_system_prefix() -> String {
    "You are a smart contract interaction assistant.".into()
}

fn default_user_format() -> String {
    "Execute: {{function_name}}({{args | tojson}})".into()
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ModelConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub routing: Vec<String>,
    #[serde(default = "default_provider_type")]
    pub provider_type: String,
    #[serde(default)]
    pub model_name: String,
    #[serde(default)]
    pub api_base: String,
    #[serde(default)]
    pub api_key_location: String,
}

fn default_provider_type() -> String {
    "openai".into()
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct FunctionHint {
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ExtraFunction {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub args_schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TargetConfig {
    #[serde(default)]
    pub bin_name: String,
    #[serde(default)]
    pub cli_ref: Option<String>,
    #[serde(default)]
    pub cli_ref_cmd: Option<String>,
    #[serde(default)]
    pub tools_json: Option<String>,
    #[serde(default)]
    pub output_dir: Option<String>,
}

impl TzHeuristics {
    /// Load from a TOML file path. Returns default if path doesn't exist.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            log::info!("[tz] No heuristics file at {:?}, using defaults", path);
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read heuristics {:?}: {}", path, e))?;
        Ok(toml::from_str(&content)?)
    }

    /// Categorize a function name by keyword matching.
    pub fn categorize(&self, name: &str) -> Vec<String> {
        let name_lower = name.to_lowercase();
        let mut tags: Vec<String> = Vec::new();
        for (category, keywords) in &self.categories {
            if keywords.iter().any(|kw| name_lower.contains(kw)) {
                tags.push(category.clone());
            }
        }
        if tags.is_empty() {
            tags.push("general".into());
        }
        tags
    }

    /// Check if an argument name matches a secret pattern.
    pub fn is_secret(&self, arg_name: &str) -> bool {
        let name_lower = arg_name.to_lowercase();
        self.secrets
            .patterns
            .iter()
            .any(|pat| name_lower.contains(pat))
    }

    /// Get the most specific system prompt for given tags.
    pub fn system_prompt(&self, tags: &[String]) -> String {
        for tag in tags {
            if let Some(prompt) = self.templates.category_prompts.get(tag) {
                return prompt.clone();
            }
        }
        self.templates.system_prefix.clone()
    }

    /// Get per-function semantic hints if available.
    pub fn function_hint(&self, name: &str) -> Option<String> {
        let key = name.replace('.', "_");
        if let Some(hint) = self.function_hints.get(&key) {
            return Some(hint.text.clone());
        }
        let tags = self.categorize(name);
        for tag in &tags {
            if let Some(hint) = self.function_hints.get(tag) {
                return Some(hint.text.clone());
            }
        }
        None
    }
}

// ─── TensorZero Generator ───────────────────────────────────────────────────

pub struct TensorZeroGenerator;

impl TensorZeroGenerator {
    pub fn new() -> Self {
        Self
    }
}

impl Generator for TensorZeroGenerator {
    fn name(&self) -> &'static str {
        "tensorzero"
    }

    fn enabled_by_default(&self) -> bool {
        false
    }

    fn generate(&self, ctx: &GenerationContext) -> anyhow::Result<GenerationResult> {
        // 1. Load heuristics from context or default locations
        let heuristics = match &ctx.tz_heuristics_path {
            Some(hp) => {
                let abs_path = ctx.workspace_root.join(hp);
                TzHeuristics::load(&abs_path)?
            }
            None => {
                let candidates = [
                    ctx.workspace_root.join("recipes/tz-heuristics.toml"),
                    ctx.workspace_root.join("tz-heuristics.toml"),
                ];
                candidates
                    .iter()
                    .find(|p| p.exists())
                    .map(|p| TzHeuristics::load(p).unwrap())
                    .unwrap_or_default()
            }
        };
        let heuristics = heuristics;

        // 2. Determine output directory
        let output_dir = if let Some(tz_out) = ctx.tz_out.as_ref() {
            tz_out.clone()
        } else {
            ctx.workspace_root.join("recipes/tz-contracts")
        };
        let output_dir = output_dir.join(&ctx.project_name);
        std::fs::create_dir_all(&output_dir)?;

        // 3. Generate function specs from resolver types
        let mut function_configs = Vec::new();
        let mut total_files = 0u64;

        let (_label, groups) = SourceResolver::resolve_grouped(ctx);
        for (contract_name, type_map) in &groups {
            if type_map.is_empty() {
                continue;
            }

            for (type_name, schema) in type_map {
                let tags = heuristics.categorize(type_name);
                let func_name = format!(
                    "{}_{}",
                    contract_name.replace('-', "_"),
                    type_name.replace('-', "_")
                );

                let description = schema
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or(type_name);

                // args_schema.json
                let args_schema = schema_to_args_schema(schema, &heuristics);
                let func_dir = output_dir.join("functions").join(&func_name);
                std::fs::create_dir_all(&func_dir)?;
                std::fs::write(
                    func_dir.join("args_schema.json"),
                    serde_json::to_string_pretty(&args_schema)? + "\n",
                )?;
                total_files += 1;

                // system.minijinja
                let system_content =
                    build_system_prompt(&func_name, type_name, description, &tags, &heuristics);
                std::fs::write(func_dir.join("system.minijinja"), &system_content)?;
                total_files += 1;

                // user.minijinja
                let user_content = build_user_template(&func_name, &tags, description);
                std::fs::write(func_dir.join("user.minijinja"), &user_content)?;
                total_files += 1;

                function_configs.push(FunctionConfig {
                    name: func_name.clone(),
                    description: description.to_string(),
                    tags: tags.clone(),
                    type_: "chat".into(),
                    system_schema: format!("functions/{func_name}/system.minijinja"),
                    user_schema: format!("functions/{func_name}/user.minijinja"),
                    args_schema: format!("functions/{func_name}/args_schema.json"),
                });

                log::info!("[tz] Generated function: {} ({})", func_name, description);
            }
        }

        // 4. Inject extra functions from heuristics
        for (name, extra) in &heuristics.extra_functions {
            let tags = heuristics.categorize(name);
            let func_name = name.replace('.', "_");

            let schema = extra
                .args_schema
                .clone()
                .unwrap_or(serde_json::json!({"type": "object", "properties": {}, "required": []}));

            let func_dir = output_dir.join("functions").join(&func_name);
            std::fs::create_dir_all(&func_dir)?;
            std::fs::write(
                func_dir.join("args_schema.json"),
                serde_json::to_string_pretty(&schema)? + "\n",
            )?;
            total_files += 1;

            let system_content = format!(
                "{}\n\nFunction: {}\nDescription: {}\nTags: {}\n",
                heuristics.system_prompt(&tags),
                func_name,
                extra.description,
                tags.join(", ")
            );
            std::fs::write(func_dir.join("system.minijinja"), &system_content)?;
            total_files += 1;

            let user_content = format!(
                "Execute: {{{{function_name}}}}({{{{args | tojson}}}})\n\nDescription: {}\n",
                extra.description
            );
            std::fs::write(func_dir.join("user.minijinja"), &user_content)?;
            total_files += 1;

            function_configs.push(FunctionConfig {
                name: func_name.clone(),
                description: extra.description.clone(),
                tags,
                type_: "chat".into(),
                system_schema: format!("functions/{func_name}/system.minijinja"),
                user_schema: format!("functions/{func_name}/user.minijinja"),
                args_schema: format!("functions/{func_name}/args_schema.json"),
            });
        }

        // 5. Generate tensorzero.toml
        // model_override can be extended from gen-tools.yaml later; for now use heuristics defaults
        generate_tensorzero_toml(&output_dir, &function_configs, &heuristics, &None)?;
        total_files += 1;

        // 6. Generate llms.txt
        generate_llms_txt(&output_dir, &function_configs)?;
        total_files += 1;

        // 7. Generate episodes (if configured)
        if let Some(ep_dir) = &ctx.tz_episodes_dir {
            let abs_ep_dir = ctx.workspace_root.join(ep_dir);
            if abs_ep_dir.exists() {
                let ep_count = generate_episodes(&output_dir, &abs_ep_dir)?;
                total_files += ep_count as u64;
            }
        }

        // 8. Write TZ manifest for QMD / agent recipe consumption
        write_tz_manifest(&output_dir, &function_configs, &heuristics)?;
        total_files += 1;

        Ok(GenerationResult {
            name: "tensorzero",
            success: true,
            files_generated: total_files as usize,
            output_dir: Some(output_dir.to_string_lossy().to_string()),
            message: Some(format!(
                "{} TensorZero functions generated",
                function_configs.len()
            )),
        })
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Convert a raw JSON schema value into an args_schema.json for TZ.
fn schema_to_args_schema(
    schema: &serde_json::Value,
    heuristics: &TzHeuristics,
) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
        for (key, val) in props {
            let mut entry = val.clone();
            if heuristics.is_secret(key) {
                entry["writeOnly"] = serde_json::Value::Bool(true);
            }
            properties.insert(key.clone(), entry);
        }
    }

    serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "properties": properties,
        "required": []
    })
}

fn build_system_prompt(
    func_name: &str,
    _type_name: &str,
    description: &str,
    tags: &[String],
    heuristics: &TzHeuristics,
) -> String {
    let base = heuristics.system_prompt(tags);
    let mut out = format!(
        "{}\n\nFunction: {}\nDescription: {}\nTags: {}\n",
        base,
        func_name,
        description,
        tags.join(", ")
    );

    if let Some(hint) = heuristics.function_hint(func_name) {
        out.push_str(&format!("\nSEMANTIC HINTS:\n{}\n", hint));
    }

    out
}

fn build_user_template(_func_name: &str, tags: &[String], description: &str) -> String {
    let tag_set: HashSet<&str> = tags.iter().map(|s| s.as_str()).collect();

    if tag_set.contains("deploy") {
        return format!(
            "Execute the deployment operation.\n\n\
             Command: {{{{function_name}}}}\n\
             Arguments: {{{{args | tojson}}}}\n\n\
             IMPORTANT: Before executing, verify:\n\
             1. All required env vars are set\n\
             2. Contract state is synced\n\
             3. Gas estimates are sufficient\n\
             4. Never broadcast without confirmation"
        );
    }

    if tag_set.contains("query") {
        return format!(
            "Query the contract state.\n\n\
             Command: {{{{function_name}}}}\n\
             Arguments: {{{{args | tojson}}}}\n\n\
             Note: Queries are read-only and do not modify state.\n\
             They do not require gas or broadcasting."
        );
    }

    if tag_set.contains("security") {
        return format!(
            "Execute the security operation.\n\n\
             Command: {{{{function_name}}}}\n\
             Arguments: {{{{args | tojson}}}}\n\n\
             CRITICAL: Never log or echo secret values.\n\
             Secret args are marked writeOnly in the schema."
        );
    }

    format!(
        "Execute: {{{{function_name}}}}({{{{args | tojson}}}})\n\n\
         Description: {}\n",
        description
    )
}

// ─── tensorzero.toml generation ─────────────────────────────────────────────

#[derive(Debug, Clone)]
struct FunctionConfig {
    name: String,
    description: String,
    tags: Vec<String>,
    type_: String,
    system_schema: String,
    user_schema: String,
    args_schema: String,
}

fn generate_tensorzero_toml(
    output_dir: &Path,
    functions: &[FunctionConfig],
    heuristics: &TzHeuristics,
    model_override: &Option<ModelConfig>,
) -> anyhow::Result<()> {
    let mut lines = Vec::new();

    lines.push("# Auto-generated by gen-tools tensorzero generator".into());
    lines.push(String::new());

    // Gateway config
    lines.push("[gateway]".into());
    lines.push("bind_address = \"0.0.0.0:3000\"".into());
    lines.push(String::new());

    // Model config
    let model = model_override.as_ref().unwrap_or(&heuristics.model);
    let model_name = if model.name.is_empty() {
        "default_model".into()
    } else {
        model.name.clone()
    };
    lines.push(format!("[models.{}]", model_name));
    lines.push(format!(
        "routing = {}",
        serde_json::to_string(&model.routing).unwrap_or_else(|_| "[\"default\"]".into())
    ));
    lines.push(String::new());

    let routing_key = model
        .routing
        .first()
        .map(|s| s.as_str())
        .unwrap_or("default");
    lines.push(format!("[models.{}.providers.{}]", model_name, routing_key));
    lines.push(format!("type = \"{}\"", model.provider_type));
    if !model.model_name.is_empty() {
        lines.push(format!("model_name = \"{}\"", model.model_name));
    }
    if !model.api_base.is_empty() {
        lines.push(format!("api_base = \"{}\"", model.api_base));
    }
    lines.push(String::new());

    // Functions
    for fc in functions {
        lines.push(format!("[functions.{}]", fc.name));
        lines.push(format!("type = \"{}\"", fc.type_));
        lines.push(format!(
            "description = \"{}\"",
            fc.description.replace('"', "\\\"")
        ));
        lines.push(format!(
            "tags = {}",
            serde_json::to_string(&fc.tags).unwrap_or_default()
        ));
        lines.push(format!("args_schema = \"{}\"", fc.args_schema));
        lines.push(String::new());
        lines.push(format!("[functions.{}.variants.default]", fc.name));
        lines.push(format!("type = \"chat\"",));
        lines.push(format!("model = \"{}\"", model_name));
        lines.push(format!("system_template = \"{}\"", fc.system_schema));
        lines.push(format!("user_template = \"{}\"", fc.user_schema));
        lines.push(String::new());
    }

    // Metrics
    lines.push(
        "# ─── Metrics for Feedback Loop ───────────────────────────────────────────────".into(),
    );
    lines.push(String::new());
    lines.push("[metrics.task_success]".into());
    lines.push("type = \"boolean\"".into());
    lines.push(String::new());
    lines.push("[metrics.execution_time]".into());
    lines.push("type = \"float\"".into());
    lines.push("unit = \"seconds\"".into());
    lines.push(String::new());
    lines.push("[metrics.token_usage]".into());
    lines.push("type = \"uint\"".into());
    lines.push("unit = \"tokens\"".into());
    lines.push(String::new());

    // Datasets
    lines
        .push("# ─── Datasets ────────────────────────────────────────────────────────────".into());
    lines.push(String::new());
    lines.push("[datasets]".into());
    lines.push(String::new());

    let content = lines.join("\n");
    std::fs::write(output_dir.join("tensorzero.toml"), &content)?;
    Ok(())
}

fn generate_llms_txt(output_dir: &Path, functions: &[FunctionConfig]) -> anyhow::Result<()> {
    let mut content = String::from("# TensorZero Functions\n\n");
    content.push_str(&format!(
        "Generated {} functions for TensorZero gateway.\n\n",
        functions.len()
    ));
    content.push_str("## Available Functions\n\n");
    for fc in functions {
        content.push_str(&format!(
            "- `{}`: {} [tags: {}]\n",
            fc.name,
            fc.description,
            fc.tags.join(", ")
        ));
    }
    std::fs::write(output_dir.join("llms.txt"), &content)?;
    Ok(())
}

/// Write a TZ manifest.json that QMD and the agent recipe can consume.
///
/// This file describes what was generated, where files live, and how
/// an external agent tool can invoke the TensorZero gateway for each function.
fn write_tz_manifest(
    output_dir: &Path,
    configs: &[FunctionConfig],
    heuristics: &TzHeuristics,
) -> anyhow::Result<()> {
    let manifest = serde_json::json!({
        "version": "1",
        "generator": "gen-tools/tensorzero",
        "generated_at": chrono_human_readable(),
        "functions_count": configs.len(),
        "functions": configs.iter().map(|fc| serde_json::json!({
            "name": fc.name,
            "description": fc.description,
            "tags": fc.tags,
            "template_paths": {
                "system": fc.system_schema,
                "user": fc.user_schema,
                "args_schema": fc.args_schema,
            },
            "gateway_url": "http://localhost:3000/chat",
        })).collect::<Vec<_>>(),
        "model": {
            "name": heuristics.model.name,
            "routing": heuristics.model.routing,
            "provider": heuristics.model.provider_type,
            "model_name": heuristics.model.model_name,
        },
        "categories": heuristics.categories.keys().cloned().collect::<Vec<_>>(),
        "targets": heuristics.targets.keys().cloned().collect::<Vec<_>>(),
    });

    let manifest_path = output_dir.join("tz-manifest.json");
    std::fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)?;
    log::info!("[tz] Manifest written to {:?}", manifest_path);
    Ok(())
}

/// Returns a human-readable timestamp string.
fn chrono_human_readable() -> String {
    // Naive chrono-free version using std::time
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    format!("{:02}:{:02}:{:02} UTC", h, m, s)
}

// ─── Episode generation ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Episode {
    pub name: String,
    pub title: String,
    pub outcome: String,
    pub wall_clock: String,
    pub input_context: String,
    pub steps: Vec<EpisodeStep>,
    pub tool_calls: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct EpisodeStep {
    pub title: String,
    pub prompt: String,
    pub expected_output: String,
}

/// Parse markdown episodes from a directory and write them as TOML.
fn generate_episodes(output_dir: &Path, episodes_dir: &Path) -> anyhow::Result<usize> {
    let ep_out = output_dir.join("episodes");
    std::fs::create_dir_all(&ep_out)?;
    let mut count = 0usize;

    if !episodes_dir.exists() {
        return Ok(0);
    }

    let re = match std::fs::read_dir(episodes_dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(0),
    };

    for entry in re.flatten() {
        let path = entry.path();
        if path.extension().map_or(true, |e| e != "md") {
            continue;
        }
        match parse_episode_md(&path) {
            Ok(ep) => {
                let out_path = ep_out.join(format!("{}.toml", ep.name));
                let toml_str =
                    toml::to_string(&ep).map_err(|e| anyhow::anyhow!("TOML serialize: {}", e))?;
                std::fs::write(&out_path, &toml_str)?;
                count += 1;
                log::info!("[tz] Generated episode: {}", ep.name);
            }
            Err(e) => {
                log::warn!("[tz] Failed to parse episode {:?}: {}", path, e);
            }
        }
    }

    Ok(count)
}

fn parse_episode_md(path: &Path) -> anyhow::Result<Episode> {
    let content = std::fs::read_to_string(path)?;
    let lines: Vec<&str> = content.lines().collect();
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    // Extract title from first H1
    let mut title = name.clone();
    let mut outcome = String::new();
    let mut wall_clock = String::new();
    let mut input_context = String::new();
    let mut steps = Vec::new();
    let mut tool_calls = Vec::new();

    for line in &lines {
        if line.starts_with("# ") && line.len() > 2 {
            title = line[2..].trim().to_string();
        } else if line.starts_with("**Outcome:**") {
            outcome = line["**Outcome:**".len()..].trim().to_string();
        } else if line.starts_with("**Wall clock:**") {
            wall_clock = line["**Wall clock:**".len()..].trim().to_string();
        } else if line.starts_with("**Context:**") {
            input_context = line["**Context:**".len()..].trim().to_string();
        }
    }

    let step_re =
        regex::Regex::new(r"^##\s+(?:Inference\s+\d+|Pre-flight|Optional|Error handling):\s*(.+)")
            .unwrap();
    let mut current_step: Option<EpisodeStep> = None;

    for line in &lines {
        if let Some(caps) = step_re.captures(line) {
            if let Some(current) = current_step.take() {
                steps.push(current);
            }
            let step_title = caps
                .get(1)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            current_step = Some(EpisodeStep {
                title: step_title,
                prompt: String::new(),
                expected_output: String::new(),
            });
        } else if line.starts_with("Tool Call:") && !tool_calls.contains(&line.to_lowercase()) {
            let cmd = line["Tool Call:".len()..].trim().to_string();
            let normalized: String = cmd
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            if !tool_calls.contains(&normalized.to_lowercase()) {
                tool_calls.push(normalized.to_lowercase());
            }
        } else if let Some(ref mut step) = current_step {
            if step.prompt.is_empty() {
                step.prompt.push_str(line);
            } else {
                step.expected_output.push_str(line);
            }
        }
    }
    if let Some(step) = current_step {
        steps.push(step);
    }

    Ok(Episode {
        name,
        title,
        outcome,
        wall_clock,
        input_context,
        steps,
        tool_calls,
    })
}
