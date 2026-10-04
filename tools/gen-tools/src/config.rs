//! Configuration types, CLI argument parsing, and project config file loading.
//!
//! The CLI supports two modes:
//!   **Direct mode** — provide `workspace_root` positional arg (default: `.`).
//!   **Config mode** — provide `--config <path>` and optionally `--project <name>` / `--all`.

use clap::Parser;
use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::tensorzero::ModelConfig;

// ---------------------------------------------------------------------------
// CLI argument struct
// ---------------------------------------------------------------------------

/// Subcommand wrapper — currently only `deps`, but extensible for future command groups.
#[derive(clap::Subcommand, Debug)]
pub enum CliSubcommand {
    /// Dependency management commands: switch, status, scrape, overview, graph, update, push.
    #[command(name = "deps", subcommand)]
    Deps(crate::deps::DepsCommand),
}

/// Unified CLI for CosmWasm contract type generation.
///
/// Runs a configurable pipeline of code generators against one or more
/// CosmWasm workspaces defined in a project config file (gen-tools.yaml).
/// Each step produces typed output from the contract's JSON schemas.
///
/// With no arguments, runs the full default pipeline on the current directory.
#[derive(Parser, Debug)]
#[command(name = "gen-tools", version, about, subcommand_required = false)]
pub struct Cli {
    // ── Mode selector ───────────────────────────────────────────────

    /// Path to the CosmWasm workspace or library root.
    ///
    /// Ignored when `--config` is set (path comes from the config file).
    /// This is the directory containing Cargo.toml for the contract(s).
    #[arg(default_value = ".")]
    pub workspace_root: PathBuf,

    /// Path to a gen-tools.yaml project config file.
    ///
    /// When set, runs the pipeline on one or more projects defined in that
    /// file. Use `--project <name>` to run a single project, or `--all`
    /// to run every project defined in the config.
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// Run only this project from the config file.
    ///
    /// Requires `--config`. Mutually exclusive with `--all`.
    #[arg(long)]
    pub project: Option<String>,

    /// Run all projects defined in the config file.
    ///
    /// Requires `--config`. Mutually exclusive with `--project`.
    #[arg(long)]
    pub all: bool,

    /// Optional subcommand.
    #[command(subcommand)]
    pub command: Option<CliSubcommand>,

    /// Comma-separated list of pipeline steps to run, or "list" to show
    /// available generators.
    ///
    /// Prefix with `no-` to skip a default step, e.g. `--steps schema,ts-codegen,no-zod`.
    /// In config mode, project-level steps override this value.
    #[arg(short = 's', long, default_value = "default")]
    pub steps: String,

    // ── Output dir overrides ─────────────────────────────────────────

    /// Base output directory root. Default: `{workspace_root}/generated/`
    #[arg(short = 'o', long)]
    pub output_dir: Option<PathBuf>,

    /// ts-codegen output directory.
    #[arg(long)]
    pub ts_out: Option<PathBuf>,

    /// Python output directory.
    #[arg(long)]
    pub py_out: Option<PathBuf>,

    /// Zod output directory.
    #[arg(long)]
    pub zod_out: Option<PathBuf>,

    /// Proto output directory.
    #[arg(long)]
    pub proto_out: Option<PathBuf>,

    /// Go type output directory.
    #[arg(long)]
    pub go_out: Option<PathBuf>,

    /// Rust crate output directory.
    #[arg(long)]
    pub rust_out: Option<PathBuf>,

    /// OpenAPI output directory.
    #[arg(long)]
    pub openapi_out: Option<PathBuf>,

    // ── Misc ─────────────────────────────────────────────────────────

    /// Comma-separated proto modules to generate (e.g. terp,osmosis,ibc).
    #[arg(long, default_value = "terp,osmosis,ibc,cosmos")]
    pub proto_modules: String,

    /// Skip schema generation (use existing schema files).
    #[arg(long)]
    pub skip_schema: bool,

    /// Verbose output.
    #[arg(short = 'v', long)]
    pub verbose: bool,

    /// Generate ALL outputs into a single api/<lang>/ directory
    /// within the gen-tools crate root.
    ///
    /// When set:
    ///   - ts-codegen → api/ts/<contract>/{types,client,message-composer}/
    ///   - Python     → api/python/<contract>/
    ///   - Zod        → api/ts/<contract>/zod/
    ///   - Proto      → api/proto/<contract>/ plus api/proto/terp-proto/
    ///   - Go         → api/go/<contract>/
    ///   - OpenAPI    → api/openapi/<contract>/
    ///   - TZ         → api/tz/
    ///   - Readme     → api/readme/
    ///
    /// This flag overrides all per-project output dirs and consolidates
    /// all generated artifacts into one place. Compatible with justfile
    /// commands (e.g. `just gen-unified-ts`).
    #[arg(long, short = 'u')]
    pub unified: bool,
}

// ---------------------------------------------------------------------------
// YAML project config file format
// ---------------------------------------------------------------------------

/// Top-level gen-tools.yaml structure.
#[derive(Debug, Deserialize)]
pub struct ProjectConfigFile {
    /// Optional base path that all project `path` fields are relative to.
    /// Defaults to the directory containing this config file.
    #[serde(default)]
    pub base_path: Option<PathBuf>,

    /// The list of projects to generate types for.
    pub projects: Vec<ProjectEntry>,
}

/// A single project entry in gen-tools.yaml.
#[derive(Debug, Deserialize, Clone)]
pub struct ProjectEntry {
    /// Human-readable project name (used with `--project <name>`).
    pub name: String,

    /// Path to the workspace root, relative to the config file's directory
    /// (or to `base_path` if set).
    pub path: PathBuf,

    /// Optional per-project steps override.
    #[serde(default)]
    pub steps: Option<String>,

    /// Skip schema generation for this project.
    #[serde(default)]
    pub skip_schema: Option<bool>,

    // Per-step output directory overrides (relative to project root).
    #[serde(default)]
    pub ts_out: Option<PathBuf>,
    #[serde(default)]
    pub py_out: Option<PathBuf>,
    #[serde(default)]
    pub zod_out: Option<PathBuf>,
    #[serde(default)]
    pub proto_out: Option<PathBuf>,
    #[serde(default)]
    pub go_out: Option<PathBuf>,
    #[serde(default)]
    pub rust_out: Option<PathBuf>,
    #[serde(default)]
    pub openapi_out: Option<PathBuf>,

    /// Proto module filter for this project.
    #[serde(default)]
    pub proto_modules: Option<String>,

    /// ── TensorZero configuration ─────────────────────────────────────────
    /// Path to tz-heuristics.toml (relative to project root).
    #[serde(default)]
    pub tz_heuristics: Option<PathBuf>,
    /// Path to episode markdown files (relative to project root).
    #[serde(default)]
    pub tz_episodes: Option<PathBuf>,
    /// Path to TZ recipes directory (relative to project root).
    #[serde(default)]
    pub tz_recipes_dir: Option<PathBuf>,
    /// Path to TZ profiles directory (relative to project root).
    #[serde(default)]
    pub tz_profiles_dir: Option<PathBuf>,
    /// Output directory for TZ recipes (relative to project root).
    #[serde(default)]
    pub tz_out: Option<PathBuf>,
    /// Model config overrides for tensorzero.toml.
    #[serde(default)]
    pub tz_model: Option<ModelConfig>,
}

impl ProjectConfigFile {
    /// Load and validate a gen-tools.yaml file.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read config file {:?}: {}", path, e))?;
        let config: ProjectConfigFile = serde_yaml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse config file {:?}: {}", path, e))?;

        // Validate: no duplicate project names
        let mut seen = std::collections::HashSet::new();
        for p in &config.projects {
            if !seen.insert(p.name.as_str()) {
                anyhow::bail!("Duplicate project name '{}' in config file {:?}", p.name, path);
            }
        }

        Ok(config)
    }

    /// Resolve a single project entry to an absolute path.
    pub fn resolve_project_path(&self, config_dir: &Path, entry: &ProjectEntry) -> PathBuf {
        let base = self
            .base_path
            .as_ref()
            .map(|b| {
                if b.is_absolute() {
                    b.clone()
                } else {
                    config_dir.join(b)
                }
            })
            .unwrap_or_else(|| config_dir.to_path_buf());

        let path = if entry.path.is_absolute() {
            entry.path.clone()
        } else {
            base.join(&entry.path)
        };
        path.canonicalize().unwrap_or(path)
    }

    /// Find a project by name.
    pub fn find_by_name(&self, name: &str) -> Option<&ProjectEntry> {
        self.projects.iter().find(|p| p.name == name)
    }
}

// ---------------------------------------------------------------------------
// Generation context — derived from CLI + workspace inspection
// ---------------------------------------------------------------------------

/// Resolved generation context passed to each generator.
#[derive(Debug, Clone)]
pub struct GenerationContext {
    /// Human-readable project name (if running from config).
    pub project_name: String,

    /// Absolute path to workspace root.
    pub workspace_root: PathBuf,

    /// Base output directory.
    pub output_dir: PathBuf,

    /// Resolved per-step output directories.
    pub ts_out: PathBuf,
    pub py_out: PathBuf,
    pub zod_out: PathBuf,
    pub proto_out: PathBuf,
    pub go_out: PathBuf,
    pub rust_out: PathBuf,
    pub openapi_out: PathBuf,

    /// Proto module filter.
    pub proto_modules: Vec<String>,

    /// TensorZero output directory.
    pub tz_out: Option<PathBuf>,

    /// TensorZero heuristics file path (relative to workspace root).
    pub tz_heuristics_path: Option<PathBuf>,

/// TensorZero episodes directory path (relative to workspace root).
    pub tz_episodes_dir: Option<PathBuf>,
    /// TensorZero recipes directory path (relative to workspace root).
    pub tz_recipes_dir: Option<PathBuf>,
    /// TensorZero profiles directory path (relative to workspace root).
    pub tz_profiles_dir: Option<PathBuf>,
    /// Whether to skip invoking `cargo schema`.
    /// Whether to skip invoking `cargo schema`.
    pub skip_schema: bool,

    /// Whether the workspace is a single contract lib vs multi-contract workspace.
    pub is_library: bool,

    /// Workspace members found by inspecting Cargo.toml.
    pub workspace_members: Vec<String>,

    /// Contract names found by scanning for CosmWasm entry points.
    pub contracts: Vec<ContractMeta>,
}

/// Metadata about a single CosmWasm contract in the workspace.
#[derive(Debug, Clone)]
pub struct ContractMeta {
    /// Contract name (from Cargo.toml package name or directory name).
    pub name: String,
    /// Path to the contract's Cargo.toml.
    pub manifest_path: PathBuf,
    /// Schema directory (typically `{manifest_dir}/schema/`).
    pub schema_dir: PathBuf,
    /// Whether this contract has been identified as a CosmWasm contract.
    pub is_cosmwasm: bool,
}

impl ContractMeta {
    /// Path to the merged schema JSON file.
    pub fn schema_file(&self) -> PathBuf {
        self.schema_dir.join(format!("{}.json", self.name))
    }
}

// ---------------------------------------------------------------------------
// Context resolution
// ---------------------------------------------------------------------------

/// CLI flags that control how a project is resolved.
pub struct ProjectOverrides {
    pub steps: String,
    pub skip_schema: bool,
    pub proto_modules: String,
    pub ts_out: Option<PathBuf>,
    pub py_out: Option<PathBuf>,
    pub zod_out: Option<PathBuf>,
    pub proto_out: Option<PathBuf>,
    pub go_out: Option<PathBuf>,
    pub rust_out: Option<PathBuf>,
    pub openapi_out: Option<PathBuf>,
    pub tz_out: Option<PathBuf>,
    pub tz_heuristics: Option<PathBuf>,
    pub tz_episodes: Option<PathBuf>,
    pub tz_recipes_dir: Option<PathBuf>,
    pub tz_profiles_dir: Option<PathBuf>,
    /// When true, all output dirs resolve against unified_base
    /// instead of workspace_root. All generated artifacts go into
    /// a single api/<lang>/<contract>/ directory.
    pub unified: bool,
    /// Base path for unified output dirs (the gen-tools crate root).
    pub unified_base: Option<PathBuf>,
}

impl Cli {
    /// Resolve the CLI args into a validated GenerationContext (direct mode).
    pub fn into_context(self) -> anyhow::Result<GenerationContext> {
        let workspace_root = self.workspace_root.canonicalize()?;
        let overrides = ProjectOverrides {
            steps: self.steps.clone(),
            skip_schema: self.skip_schema,
            proto_modules: self.proto_modules.clone(),
            ts_out: self.ts_out,
            py_out: self.py_out,
            zod_out: self.zod_out,
            proto_out: self.proto_out,
            go_out: self.go_out,
            rust_out: self.rust_out,
            openapi_out: self.openapi_out,
            tz_out: None,
            tz_heuristics: None,
            tz_episodes: None,
            tz_recipes_dir: None,
            tz_profiles_dir: None,
            unified: self.unified,
            unified_base: if self.unified {
                Some(std::env::current_dir().unwrap_or_default())
            } else {
                None
            },
        };
        build_context(&workspace_root, &overrides, self.output_dir)
    }
}

/// Build a GenerationContext for a single workspace root.
///
/// Applies CLI- or project-level overrides on top of default output paths.
pub fn build_context(
    workspace_root: &Path,
    overrides: &ProjectOverrides,
    output_dir_override: Option<PathBuf>,
) -> anyhow::Result<GenerationContext> {
    // In unified mode, all output dirs resolve against unified_base
    // (the gen-tools crate root) instead of workspace_root.
    let root_for_output = if overrides.unified {
        overrides
            .unified_base
            .as_deref()
            .unwrap_or(workspace_root)
    } else {
        workspace_root
    };

    let output_dir = output_dir_override
        .clone()
        .unwrap_or_else(|| root_for_output.join("api"));

    let ts_out = resolve_dir(overrides.ts_out.as_ref(), root_for_output, &["api", "ts"]);
    let py_out = resolve_dir(
        overrides.py_out.as_ref(),
        root_for_output,
        &["api", "python"],
    );
    let zod_out = resolve_dir(overrides.zod_out.as_ref(), root_for_output, &["api", "ts"]);
    let proto_out = resolve_dir(
        overrides.proto_out.as_ref(),
        root_for_output,
        &["api", "proto"],
    );
    let go_out = resolve_dir(overrides.go_out.as_ref(), root_for_output, &["api", "go"]);
    let rust_out = resolve_dir(
        overrides.rust_out.as_ref(),
        root_for_output,
        &["api", "rust"],
    );
    let openapi_out = resolve_dir(
        overrides.openapi_out.as_ref(),
        root_for_output,
        &["api", "openapi"],
    );

    let tz_out = overrides
        .tz_out
        .as_ref()
        .map(|p| {
            if p.is_absolute() {
                p.clone()
            } else {
                root_for_output.join(p)
            }
        })
        .or_else(|| {
            // Default TZ outputs to api/tz (relative to root_for_output
            // in unified mode, or workspace_root otherwise)
            Some(root_for_output.join("api").join("tz"))
        });

    let proto_modules: Vec<String> = overrides
        .proto_modules
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let (workspace_members, contracts, is_library) = detect_workspace(workspace_root)?;

    let project_name = workspace_root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "default".to_string());

    Ok(GenerationContext {
        project_name,
        workspace_root: workspace_root.to_path_buf(),
        output_dir,
        ts_out,
        py_out,
        zod_out,
        proto_out,
        go_out,
        rust_out,
        openapi_out,
        proto_modules,
        tz_out,
        tz_heuristics_path: overrides.tz_heuristics.clone(),
        tz_episodes_dir: overrides.tz_episodes.clone(),
        tz_recipes_dir: overrides.tz_recipes_dir.clone(),
        tz_profiles_dir: overrides.tz_profiles_dir.clone(),
        skip_schema: overrides.skip_schema,
        is_library,
        workspace_members,
        contracts,
    })
}

/// Directory for one contract under `api/<lang>/<contract>/`.
pub fn contract_api_dir(lang_root: &Path, contract: &str) -> PathBuf {
    lang_root.join(contract)
}

/// TypeScript variant directory: `api/ts/<contract>/<variant>/`.
/// Variants are `zod`, `esbundles`, `types`, `client`, `message-composer`,
/// `filters`, and `argus`.
pub fn ts_variant_dir(ts_root: &Path, contract: &str, variant: &str) -> PathBuf {
    ts_root.join(contract).join(variant)
}

/// Resolve an output directory: use override if set, else default relative to workspace root.
fn resolve_dir(
    override_dir: Option<&PathBuf>,
    workspace_root: &Path,
    default_segments: &[&str],
) -> PathBuf {
    match override_dir {
        Some(p) if p.is_absolute() => p.clone(),
        Some(p) => workspace_root.join(p),
        None => {
            let mut path = workspace_root.to_path_buf();
            for seg in default_segments {
                path = path.join(seg);
            }
            path
        }
    }
}

// ---------------------------------------------------------------------------
// Workspace detection
// ---------------------------------------------------------------------------

fn detect_workspace(
    root: &Path,
) -> anyhow::Result<(Vec<String>, Vec<ContractMeta>, bool)> {
    use std::fs;

    let cargo_toml = root.join("Cargo.toml");
    let mut workspace_members: Vec<String> = Vec::new();
    let contracts: Vec<ContractMeta> = Vec::new();
    let mut is_library = false;

    if !cargo_toml.exists() {
        return Ok((workspace_members, contracts, false));
    }

    let content = fs::read_to_string(&cargo_toml)?;
    let cargo_parsed: toml::Value = toml::from_str(&content)?;

    // Check if this is a library package
    if let Some(package) = cargo_parsed.get("package") {
        if let Some(lib) = package.get("lib") {
            is_library = lib.is_table();
        }
        let has_bins = content.contains("[[bin]]");
        let has_workspace = content.contains("[workspace]");
        if !has_bins && !has_workspace {
            is_library = true;
        }
    }

    // Collect workspace members (with glob expansion)
    if let Some(workspace) = cargo_parsed.get("workspace") {
        if let Some(members) = workspace.get("members") {
            if let Some(arr) = members.as_array() {
                for m in arr {
                    if let Some(pattern) = m.as_str() {
                        // Expand glob patterns (e.g. "contracts/proposal/*")
                        let full_pattern = root.join(pattern);
if let Ok(expanded) = glob::glob(&full_pattern.to_string_lossy()) {
                            let mut found = false;
                            for entry in expanded.flatten() {
                                workspace_members.push(
                                    entry
                                        .strip_prefix(root)
                                        .unwrap_or(&entry)
                                        .to_string_lossy()
                                        .to_string(),
                                );
                                found = true;
                            }
                            if !found {
                                // If no glob match, treat as a literal path
                                workspace_members.push(pattern.to_string());
                            }
                        } else {
                            // If not a valid glob pattern, treat as literal
                            workspace_members.push(pattern.to_string());
                        }
                    }
                }
            }
        }
    }

    // Discover contracts
    let mut discovered_contracts = Vec::new();
    if workspace_members.is_empty() {
        let meta = inspect_contract(root, cargo_toml)?;
        if let Some(m) = meta {
            discovered_contracts.push(m);
        }
    } else {
        for member in &workspace_members {
            let member_dir = root.join(member);
            let member_toml = member_dir.join("Cargo.toml");
            if member_toml.exists() {
                let meta = inspect_contract(&member_dir, member_toml)?;
                if let Some(m) = meta {
                    discovered_contracts.push(m);
                }
            }
        }
    }

    Ok((workspace_members, discovered_contracts, is_library))
}

fn inspect_contract(
    dir: &Path,
    manifest: PathBuf,
) -> anyhow::Result<Option<ContractMeta>> {
    use std::fs;

    let content = fs::read_to_string(&manifest)?;
    let cargo_parsed: toml::Value = toml::from_str(&content)?;

    let pkg_name = cargo_parsed
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            dir.file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        });

    let is_cosmwasm = content.contains("cosmwasm-std") || content.contains("cosmwasm_std");
    let schema_dir = dir.join("schema");

    Ok(Some(ContractMeta {
        name: pkg_name,
        manifest_path: manifest,
        schema_dir,
        is_cosmwasm,
    }))
}

// ---------------------------------------------------------------------------
// Step parsing
// ---------------------------------------------------------------------------

/// Parse the --steps flag into a list of step names and exclusions.
///
/// Returns (enabled_steps, disabled_steps).
pub fn parse_steps(steps: &str) -> (Vec<String>, Vec<String>) {
    if steps == "default" {
        let defaults = vec![
"schema", "ts-codegen", "ts-bundles", "proto", "python", "zod", "go", "rust", "readme", "openapi", "vocs-docs", "tensorzero",
        ];
        return (defaults.iter().map(|s| s.to_string()).collect(), Vec::new());
    }

    let mut enabled = Vec::new();
    let mut disabled = Vec::new();

    for step in steps.split(',') {
        let step = step.trim().to_lowercase();
        if let Some(name) = step.strip_prefix("no-") {
            disabled.push(name.to_string());
        } else {
            enabled.push(step);
        }
    }

    (enabled, disabled)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_steps_default() {
        let (enabled, disabled) = parse_steps("default");
        assert!(enabled.len() >= 8);
        assert!(disabled.is_empty());
    }

    #[test]
    fn test_parse_steps_custom() {
        let (enabled, disabled) = parse_steps("schema,ts-codegen,no-zod,no-go");
        assert_eq!(enabled, vec!["schema", "ts-codegen"]);
        assert_eq!(disabled, vec!["zod", "go"]);
    }

    #[test]
    fn test_parse_steps_no_prefix_alone() {
        let (enabled, disabled) = parse_steps("no-trailmark");
        assert!(enabled.is_empty());
        assert_eq!(disabled, vec!["trailmark"]);
    }

    #[test]
    fn test_config_file_parse() {
        let yaml = r#"
projects:
  - name: cw-infuser
    path: crates/cw-infuser
  - name: dao-contracts
    path: crates/dao-contracts
    steps: schema,ts-codegen,no-zod
"#;
        let config: ProjectConfigFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.projects.len(), 2);
        assert_eq!(config.projects[0].name, "cw-infuser");
        assert_eq!(config.projects[1].steps.as_deref(), Some("schema,ts-codegen,no-zod"));
    }

    #[test]
    fn test_resolve_project_path() {
        let yaml = r#"
base_path: "."
projects:
  - name: test
    path: crates/my-contract
"#;
        let config: ProjectConfigFile = serde_yaml::from_str(yaml).unwrap();
        let config_dir = Path::new("/home/user/workspace");
        let resolved = config.resolve_project_path(config_dir, &config.projects[0]);
        assert_eq!(resolved, PathBuf::from("/home/user/workspace/crates/my-contract"));
    }
}