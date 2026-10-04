//! gen-api: Terp Network API client library generator.
//!
//! Runs the gen-tools pipeline on all CosmWasm workspaces defined in
//! `tools/gen-tools/gen-tools.yaml`, generating typed API client libraries
//! (proto, TypeScript, Python, Go, Zod, OpenAPI, README docs, etc.) into the
//! `api/<lang>/<contract>/` directory at the terp-rs workspace root.
//!
//! Usage:
//!   gen-api                         # Generate for all projects (default steps)
//!   gen-api --project dao-contracts # Generate for a single project
//!   gen-api --filter proto,python   # Only run specific generators
//!   gen-api --help

use clap::Parser;
use gen_tools::config::{build_context, ProjectConfigFile, ProjectOverrides};
use gen_tools::pipeline::Pipeline;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(name = "gen-api", version, about)]
struct Args {
    /// Run only this project from the config (default: all).
    #[arg(short = 'p', long)]
    project: Option<String>,

    /// Comma-separated pipeline steps (default: full pipeline).
    #[arg(short = 's', long, default_value = "default")]
    filter: String,

    /// Path to the terp-rs workspace root. Auto-detected by default.
    #[arg(long)]
    workspace_root: Option<PathBuf>,

    /// Skip schema regeneration (use existing schema files).
    #[arg(long)]
    skip_schema: bool,
}

// ---------------------------------------------------------------------------
// Workspace root detection
// ---------------------------------------------------------------------------

/// Determine the terp-rs workspace root.
///
/// Priority:
///   1. `--workspace-root` CLI flag
///   2. `TERP_RS_ROOT` env var
///   3. Auto-detect: binary is at `bin/gen-api/` within the workspace,
///      so workspace root is two levels up from `CARGO_MANIFEST_DIR`.
fn resolve_workspace_root(cli_override: Option<PathBuf>) -> PathBuf {
    if let Some(p) = cli_override {
        if p.exists() {
            return p;
        }
        eprintln!("warning: --workspace-root {:?} not found, falling back", p);
    }

    if let Ok(env) = std::env::var("TERP_RS_ROOT") {
        let p = PathBuf::from(env);
        if p.exists() {
            return p;
        }
        eprintln!("warning: $TERP_RS_ROOT={:?} not found, falling back", p);
    }

    // Fallback: from `bin/gen-api/` up two levels to the workspace root.
    // In dev builds, CARGO_MANIFEST_DIR is the gen-api crate directory.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    if root.exists() {
        return root;
    }

    // Last resort: current directory
    std::env::current_dir().unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Config file resolution
// ---------------------------------------------------------------------------

fn resolve_config_path(workspace_root: &Path) -> PathBuf {
    let candidates = [
        workspace_root.join("tools/gen-tools/gen-tools.yaml"),
        workspace_root.join("../tools/gen-tools/gen-tools.yaml"),
    ];

    for c in &candidates {
        if c.exists() {
            return c.canonicalize().unwrap_or_else(|_| c.clone());
        }
    }

    // If nothing found, return the expected path for a helpful error message
    candidates[0].clone()
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Args::parse();
    let ws_root = resolve_workspace_root(args.workspace_root);
    let config_path = resolve_config_path(&ws_root);

    log::info!("Workspace root: {}", ws_root.display());
    log::info!("Config file:    {}", config_path.display());

    if !config_path.exists() {
        anyhow::bail!(
            "gen-tools.yaml not found. Expected at: {}",
            config_path.display()
        );
    }

    let config_file = ProjectConfigFile::load(&config_path)?;
    let config_dir = config_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("config path has no parent"))?;

    // Select which projects to run
    let selected: Vec<&gen_tools::config::ProjectEntry> = if let Some(ref name) = args.project {
        let entry = config_file
            .find_by_name(name)
            .ok_or_else(|| anyhow::anyhow!("project '{}' not found in config", name))?;
        vec![entry]
    } else {
        config_file.projects.iter().collect()
    };

    log::info!("Projects to generate: {}", selected.len());

    for entry in &selected {
        let project_root = config_file.resolve_project_path(config_dir, entry);
        log::info!("── {:>24} ──", entry.name);

        let overrides = ProjectOverrides {
            steps: entry.steps.clone().unwrap_or_else(|| args.filter.clone()),
            skip_schema: entry.skip_schema.unwrap_or(args.skip_schema),
            proto_modules: entry
                .proto_modules
                .clone()
                .unwrap_or_else(|| "terp,osmosis,ibc,cosmos".to_string()),
            ts_out: entry.ts_out.clone(),
            py_out: entry.py_out.clone(),
            zod_out: entry.zod_out.clone(),
            proto_out: entry.proto_out.clone(),
            go_out: entry.go_out.clone(),
            rust_out: entry.rust_out.clone(),
            openapi_out: entry.openapi_out.clone(),
            tz_out: entry.tz_out.clone(),
            tz_heuristics: entry.tz_heuristics.clone(),
            tz_episodes: entry.tz_episodes.clone(),
            tz_recipes_dir: entry.tz_recipes_dir.clone(),
            tz_profiles_dir: entry.tz_profiles_dir.clone(),
            unified: false, // Use per-project output dirs from config
            unified_base: None,
        };

        match build_context(&project_root, &overrides, None) {
            Ok(ctx) => {
                let steps = entry.steps.as_deref().unwrap_or(&args.filter).to_string();
                let pipeline = Pipeline::new(&steps);
                let results = pipeline.run(&ctx);
                Pipeline::print_results(&results);

                // Check for failures
                let failures: Vec<_> = results.iter().filter(|r| r.is_err()).collect();
                if !failures.is_empty() {
                    eprintln!("  warning: {} step(s) failed for '{}'", failures.len(), entry.name);
                }
            }
            Err(e) => {
                eprintln!("  error: failed to build context for '{}': {}", entry.name, e);
            }
        }
    }

    log::info!(
        "Done. Generated API libraries are in {}/api/",
        ws_root.display()
    );

    Ok(())
}