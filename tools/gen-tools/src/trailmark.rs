//! Trailmark + Githem + QMD — semantic code analysis and LLM agent recipe.
//!
//! Three responsibilities:
//! 1. Detect and report QMD, Trailmark, Githem installation state (0-to-port)
//! 2. Run Trailmark structural analysis + Githem semantic graph (optional)
//! 3. Write an LLM-agent recipe that a Hermes/Codex/Claude agent can pick up
//!    to generate mermaid diagrams, indexer formulas, or tz episodes.
//!
//! The agent recipe is the async handoff point — the pipeline writes a recipe
//! file and the external agent tool (or a second pass) executes it.

use crate::config::GenerationContext;
use crate::{GenerationResult, Generator};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct TrailmarkGenerator;

impl Generator for TrailmarkGenerator {
    fn name(&self) -> &'static str {
        "trailmark"
    }

    fn enabled_by_default(&self) -> bool {
        true // always run — includes workspace sources AND generated API types
    }

    fn generate(&self, ctx: &GenerationContext) -> anyhow::Result<GenerationResult> {
        let mut total_files = 0;
        let out_dir = ctx.workspace_root.join(".trailmark");

        // Collect generated API type directories for trailmark/githem scan
        let mut api_dirs: Vec<PathBuf> = Vec::new();
        if ctx.ts_out.exists() {
            api_dirs.push(ctx.ts_out.clone());
        }
        if ctx.py_out.exists() {
            api_dirs.push(ctx.py_out.clone());
        }
        if ctx.zod_out.exists() {
            api_dirs.push(ctx.zod_out.clone());
        }
        if ctx.proto_out.exists() {
            api_dirs.push(ctx.proto_out.clone());
        }
        if ctx.go_out.exists() {
            api_dirs.push(ctx.go_out.clone());
        }
        if ctx.openapi_out.exists() {
            api_dirs.push(ctx.openapi_out.clone());
        }
        if let Some(ref tz_out) = ctx.tz_out
            && tz_out.exists()
        {
            api_dirs.push(tz_out.clone());
        }

        // ── Phase 1: QMD probe (0-to-port) ─────────────────────────
        let qmd_state = probe_qmd();
        log::info!("[trailmark] QMD probe: {:?}", qmd_state);

        // Write a QMD status report
        let probe_path = out_dir.join("qmd-probe.json");
        std::fs::create_dir_all(&out_dir)?;
        std::fs::write(&probe_path, serde_json::to_string_pretty(&qmd_state)?)?;
        total_files += 1;

        // If QMD is installed, check existing collections / index
        if qmd_state.installed {
            if let Some(idx) = probe_qmd_index() {
                let idx_path = out_dir.join("qmd-index.json");
                std::fs::write(&idx_path, serde_json::to_string_pretty(&idx)?)?;
                total_files += 1;
                log::info!(
                    "[trailmark] QMD index: {} collections, {} files indexed",
                    idx.collections.len(),
                    idx.total_files
                );
            }

            // Run QMD index on the workspace (via CLI call)
            if let Ok(qmd_files) = run_qmd_index(&ctx.workspace_root) {
                total_files += qmd_files;
            }
        } else {
            // Write install instructions
            let install_path = out_dir.join("qmd-install.sh");
            std::fs::write(&install_path, generate_qmd_install_script())?;
            total_files += 1;
            log::info!(
                "[trailmark] QMD not found — wrote install script to {:?}",
                install_path
            );
        }

        // ── Phase 2: Trailmark structural analysis ──────────────────
        // Scan both workspace sources and generated API types
        let mut scan_targets = vec![ctx.workspace_root.clone()];
        scan_targets.extend(api_dirs.clone());
        if let Ok(trailmark_files) = run_trailmark(&scan_targets) {
            total_files += trailmark_files;
        }

        // ── Phase 3: Githem semantic graph ──────────────────────────
        if let Ok(githem_files) = run_githem(&scan_targets) {
            total_files += githem_files;
        }

        // ── Phase 4: Write LLM agent recipe (async handoff) ─────────
        let recipe = AgentRecipe {
            workspace: ctx.workspace_root.to_string_lossy().to_string(),
            project: ctx.project_name.clone(),
            contracts: ctx
                .contracts
                .iter()
                .map(|c| ContractSummary {
                    name: c.name.clone(),
                    schema_dir: c.schema_dir.to_string_lossy().to_string(),
                })
                .collect(),
            generated_dir: ctx.output_dir.to_string_lossy().to_string(),
            qmd_installed: qmd_state.installed,
            qmd_index: qmd_state.index_path.clone(),
            tensorzero_output: ctx.tz_out.as_ref().map(|p| p.to_string_lossy().to_string()),
            steps: vec![
                StepGoal {
                    id: "mermaid-diagrams".into(),
                    description: "Generate Mermaid sequence diagrams from contract schemas".into(),
                    input_sources: vec![
                        "generated/*/schema/*.json".into(),
                        ".trailmark/structure/*.json".into(),
                    ],
                    output_path: "docs/diagrams/".into(),
                    llm_runtime: "tensorzero".into(),
                    function_ref: "generate_mermaid_diagram".into(),
                },
                StepGoal {
                    id: "indexer-formulas".into(),
                    description: "Generate subgraph/GQL indexer formulas from schema types".into(),
                    input_sources: vec![
                        "generated/*/schema/*.json".into(),
                        ".trailmark/githem/semantic_graph.json".into(),
                    ],
                    output_path: "indexer/".into(),
                    llm_runtime: "tensorzero".into(),
                    function_ref: "generate_indexer_formula".into(),
                },
                StepGoal {
                    id: "tz-episodes".into(),
                    description:
                        "Generate temporal zone episode definitions from time-aware schemas".into(),
                    input_sources: vec![
                        "generated/*/schema/*.json".into(),
                        "generated/openapi/*.json".into(),
                    ],
                    output_path: "tz-episodes/".into(),
                    llm_runtime: "tensorzero".into(),
                    function_ref: "generate_tz_episode".into(),
                },
            ],
        };
        let recipe_path = out_dir.join("agent-recipe.json");
        std::fs::write(&recipe_path, serde_json::to_string_pretty(&recipe)?)?;
        total_files += 1;
        log::info!("[trailmark] Agent recipe written to {:?}", recipe_path);

        Ok(GenerationResult {
            name: "trailmark",
            success: true,
            files_generated: total_files,
            output_dir: Some(out_dir.to_string_lossy().to_string()),
            message: Some(format!(
                "{} artifacts: QMD probe, trailmark analysis, githem graph, agent recipe",
                total_files
            )),
        })
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// QMD Probe — zero-to-port detection
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Serialize)]
struct QmdProbeResult {
    installed: bool,
    binary_path: Option<String>,
    version: Option<String>,
    index_path: Option<String>,
    install_methods: Vec<InstallMethod>,
    collections: Vec<String>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct InstallMethod {
    name: String,
    command: String,
    detected: bool,
}

/// Probe for QMD installation across all known locations.
fn probe_qmd() -> QmdProbeResult {
    let mut methods = Vec::new();
    let mut found_path: Option<String> = None;
    let mut version: Option<String> = None;
    let mut error: Option<String> = None;

    // ── Check 1: PATH (which qmd) ──────────────────────────────────
    if let Some(bin) = which("qmd") {
        found_path = Some(bin.clone());
        version = get_version(&bin);
        methods.push(InstallMethod {
            name: "PATH".into(),
            command: bin.clone(),
            detected: true,
        });
    }

    // ── Check 2: ~/.bun/bin/qmd ───────────────────────────────────
    let home = std::env::var("HOME").unwrap_or_default();
    let bun_path = format!("{}/.bun/bin/qmd", home);
    if Path::new(&bun_path).exists() && found_path.is_none() {
        found_path = Some(bun_path.clone());
        version = get_version(&bun_path);
        methods.push(InstallMethod {
            name: "bun-global".into(),
            command: bun_path,
            detected: true,
        });
    }

    // ── Check 3: npm global ────────────────────────────────────────
    for npm_root in &[
        format!("{}/.npm-global/bin/qmd", home),
        format!("{}/.local/share/npm/bin/qmd", home),
        format!("{}/.nvm/current/bin/qmd", home),
    ] {
        if Path::new(npm_root).exists() && found_path.is_none() {
            found_path = Some(npm_root.clone());
            version = get_version(npm_root);
            methods.push(InstallMethod {
                name: "npm-global".into(),
                command: npm_root.clone(),
                detected: true,
            });
        }
    }

    // ── Check 4: asdf ──────────────────────────────────────────────
    let asdf_path = format!("{}/.asdf/shims/qmd", home);
    if Path::new(&asdf_path).exists() && found_path.is_none() {
        let ver = get_version(&asdf_path);
        found_path = Some(asdf_path);
        version = ver;
        methods.push(InstallMethod {
            name: "asdf".into(),
            command: "asdf global qmd".into(),
            detected: true,
        });
    }

    // ── Check 5: nix profile ───────────────────────────────────────
    let nix_path = format!("{}/.nix-profile/bin/qmd", home);
    if Path::new(&nix_path).exists() && found_path.is_none() {
        let ver = get_version(&nix_path);
        found_path = Some(nix_path);
        version = ver;
        methods.push(InstallMethod {
            name: "nix".into(),
            command: "nix profile install github:terpnetwork/qmd".into(),
            detected: true,
        });
    }

    // ── Install methods (for quick install guidance) ────────────────
    methods.push(InstallMethod {
        name: "bun".into(),
        command: "bun install -g @terpnetwork/qmd".into(),
        detected: which("bun").is_some(),
    });
    methods.push(InstallMethod {
        name: "npm".into(),
        command: "npm install -g @terpnetwork/qmd".into(),
        detected: which("npm").is_some(),
    });
    methods.push(InstallMethod {
        name: "cargo".into(),
        command: "cargo install qmd".into(),
        detected: which("cargo").is_some(),
    });
    methods.push(InstallMethod {
        name: "go-install".into(),
        command: "go install github.com/terpnetwork/qmd@latest".into(),
        detected: which("go").is_some(),
    });

    // ── Detect existing collections ────────────────────────────────
    let collections = probe_qmd_collections(&found_path);

    // ── Index path ─────────────────────────────────────────────────
    let index_path = find_qmd_index();

    if found_path.is_none() {
        error = Some("QMD not found on PATH or common install locations".into());
    }

    QmdProbeResult {
        installed: found_path.is_some(),
        binary_path: found_path,
        version,
        index_path: Some(index_path.to_string_lossy().to_string()),
        install_methods: methods,
        collections,
        error,
    }
}

/// Detect the QMD index SQLite database.
fn find_qmd_index() -> PathBuf {
    // Check XDG_CACHE_HOME first, then ~/.cache/qmd/
    let cache = std::env::var("XDG_CACHE_HOME")
        .map(|c| PathBuf::from(c).join("qmd"))
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_default();
            PathBuf::from(home).join(".cache").join("qmd")
        });

    cache.join("index.sqlite")
}

/// Get version info from a QMD binary.
fn get_version(bin: &str) -> Option<String> {
    if let Ok(out) = Command::new(bin).arg("--version").output()
        && out.status.success()
    {
        return Some(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    None
}

/// Probe existing QMD collections via `qmd list` subcommand.
fn probe_qmd_collections(bin_path: &Option<String>) -> Vec<String> {
    let bin = match bin_path {
        Some(b) => b.clone(),
        None => return vec![],
    };

    // Try `qmd list` first
    if let Ok(out) = Command::new(&bin).arg("list").output()
        && out.status.success()
    {
        let stdout = String::from_utf8_lossy(&out.stdout);
        return stdout
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();
    }

    // Fallback: stat the index SQLite for table names
    let index_path = find_qmd_index();
    if index_path.exists() {
        // Try to probe via sqlite3 CLI (optional)
        if let Ok(out) = Command::new("sqlite3")
            .args([&index_path.to_string_lossy(), ".tables"])
            .output()
            && out.status.success()
        {
            let stdout = String::from_utf8_lossy(&out.stdout);
            return stdout
                .split_whitespace()
                .map(|s| s.to_string())
                .filter(|t| t != "sqlite_sequence")
                .collect();
        }

        // If SQLite CLI not available, just note the index exists
        vec![format!("index exists at {:?}", index_path)]
    } else {
        vec!["(no index found)".into()]
    }
}

/// Probe the QMD index for detailed stats.
#[derive(Debug, Serialize)]
struct QmdIndexState {
    index_path: String,
    exists: bool,
    total_files: u64,
    collections: Vec<String>,
}

fn probe_qmd_index() -> Option<QmdIndexState> {
    let index_path = find_qmd_index();
    if !index_path.exists() {
        return None;
    }

    // Try sqlite3 to count records
    let mut total_files = 0u64;
    let mut collections = Vec::new();

    if let Ok(out) = Command::new("sqlite3")
        .args([
            &index_path.to_string_lossy(),
            "SELECT name FROM sqlite_master WHERE type='table' AND name != 'sqlite_sequence';",
        ])
        .output()
        && out.status.success()
    {
        let stdout = String::from_utf8_lossy(&out.stdout);
        collections = stdout
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();

        // Count rows in each table
        for table in &collections {
            if let Ok(count_out) = Command::new("sqlite3")
                .arg(&*index_path.to_string_lossy())
                .arg(format!("SELECT COUNT(*) FROM \"{}\";", table))
                .output()
                && count_out.status.success()
            {
                let count_str = String::from_utf8_lossy(&count_out.stdout)
                    .trim()
                    .to_string();
                if let Ok(n) = count_str.parse::<u64>() {
                    total_files += n;
                }
            }
        }
    }

    Some(QmdIndexState {
        index_path: index_path.to_string_lossy().to_string(),
        exists: true,
        total_files,
        collections,
    })
}

/// Generate a shell script that installs QMD via the best available method.
fn generate_qmd_install_script() -> String {
    let mut script = String::new();
    script.push_str("#!/usr/bin/env bash\n");
    script.push_str("# QMD installer — generated by gen-tools trailmark generator\n");
    script.push_str("# https://github.com/terpnetwork/qmd\n\n");

    if which("bun").is_some() {
        script.push_str("bun install -g @terpnetwork/qmd\n");
    } else if which("npm").is_some() {
        script.push_str("npm install -g @terpnetwork/qmd\n");
    } else if which("cargo").is_some() {
        script.push_str("cargo install qmd\n");
    } else if which("go").is_some() {
        script.push_str("go install github.com/terpnetwork/qmd@latest\n");
    } else {
        script.push_str("echo 'No package manager found. Install manually:'\n");
        script.push_str("echo '  curl -fsSL https://terp.network/install-qmd.sh | bash'\n");
    }

    script
}

/// Run the real QMD CLI to index the workspace.
fn run_qmd_index(root: &Path) -> anyhow::Result<usize> {
    let qmd_bin = find_qmd_binary();
    match qmd_bin {
        Some(bin) => {
            log::info!("[trailmark] Running QMD index on {:?}", root);
            let output = Command::new(&bin)
                .args(["index", "--recurse"])
                .arg(root)
                .output();

            match output {
                Ok(out) => {
                    if out.status.success() {
                        log::info!("[trailmark] QMD index updated");
                        Ok(1)
                    } else {
                        log::warn!(
                            "[trailmark] QMD error: {}",
                            String::from_utf8_lossy(&out.stderr)
                        );
                        Ok(0)
                    }
                }
                Err(e) => {
                    log::warn!("[trailmark] QMD execution failed: {e}");
                    Ok(0)
                }
            }
        }
        None => {
            log::info!(
                "[trailmark] QMD not installed; skipping index (probe written to qmd-probe.json)"
            );
            Ok(0)
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Trailmark analysis (preserved from original, with stronger detection)
// ═══════════════════════════════════════════════════════════════════════════

/// Run Trailmark structural analysis on one or more scan targets (workspace + generated API types).
fn run_trailmark(targets: &[PathBuf]) -> anyhow::Result<usize> {
    let trailmark_bin = find_trailmark_binary();
    match trailmark_bin {
        Some(bin) => {
            log::info!(
                "[trailmark] Running Trailmark analysis on {} targets",
                targets.len()
            );
            let mut cmd = Command::new(&bin);
            cmd.args(["analyze", "--structural"]);
            for target in targets {
                cmd.arg(target);
            }
            let output = cmd.output();

            match output {
                Ok(out) => {
                    if out.status.success() {
                        let trailmark_dir = targets[0].join(".trailmark");
                        let count = count_files_recursive(&trailmark_dir);
                        log::info!("[trailmark] Analysis complete: {count} files");
                        Ok(count)
                    } else {
                        log::warn!(
                            "[trailmark] Binary exited with error: {}",
                            String::from_utf8_lossy(&out.stderr)
                        );
                        Ok(0)
                    }
                }
                Err(e) => {
                    log::warn!("[trailmark] Failed to execute: {e}");
                    Ok(0)
                }
            }
        }
        None => {
            log::info!("[trailmark] Binary not found; skipping Trailmark analysis");
            log::info!("  Install: cargo install trailmark");
            Ok(0)
        }
    }
}

/// Run Githem semantic graph extraction on one or more scan targets (workspace + generated API types).
fn run_githem(targets: &[PathBuf]) -> anyhow::Result<usize> {
    let githem_bin = find_githem_binary();
    match githem_bin {
        Some(bin) => {
            log::info!(
                "[trailmark] Running Githem analysis on {} targets",
                targets.len()
            );
            let mut cmd = Command::new(&bin);
            cmd.args(["graph", "--format", "json"]);
            for target in targets {
                cmd.arg(target);
            }
            let output = cmd.output();

            match output {
                Ok(out) => {
                    if out.status.success() {
                        let out_dir = targets[0].join(".trailmark").join("githem");
                        std::fs::create_dir_all(&out_dir)?;
                        let out_path = out_dir.join("semantic_graph.json");
                        std::fs::write(&out_path, &out.stdout)?;
                        log::info!("[trailmark] Githem graph written to {:?}", out_path);
                        Ok(1)
                    } else {
                        log::warn!(
                            "[trailmark] Githem error: {}",
                            String::from_utf8_lossy(&out.stderr)
                        );
                        Ok(0)
                    }
                }
                Err(e) => {
                    log::warn!("[trailmark] Githem execution failed: {e}");
                    Ok(0)
                }
            }
        }
        None => {
            log::info!("[trailmark] Githem not installed; skipping");
            log::info!("  Install: cargo install githem");
            Ok(0)
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Agent Recipe — async LLM handoff contract
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Serialize)]
struct AgentRecipe {
    workspace: String,
    project: String,
    contracts: Vec<ContractSummary>,
    generated_dir: String,
    qmd_installed: bool,
    qmd_index: Option<String>,
    tensorzero_output: Option<String>,
    steps: Vec<StepGoal>,
}

#[derive(Debug, Serialize)]
struct ContractSummary {
    name: String,
    schema_dir: String,
}

#[derive(Debug, Serialize)]
struct StepGoal {
    id: String,
    description: String,
    input_sources: Vec<String>,
    output_path: String,
    llm_runtime: String,
    function_ref: String,
}

// ═══════════════════════════════════════════════════════════════════════════
// Binary detection helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Find the QMD binary across all known locations.
fn find_qmd_binary() -> Option<String> {
    let home = std::env::var("HOME").ok()?;

    // Priority order: PATH > bun > npm > asdf > nix
    let candidates = [
        which("qmd"),
        Some(format!("{}/.bun/bin/qmd", home)).filter(|p| Path::new(p).exists()),
        Some(format!("{}/.npm-global/bin/qmd", home)).filter(|p| Path::new(p).exists()),
        Some(format!("{}/.local/share/npm/bin/qmd", home)).filter(|p| Path::new(p).exists()),
        Some(format!("{}/.asdf/shims/qmd", home)).filter(|p| Path::new(p).exists()),
        Some(format!("{}/.nix-profile/bin/qmd", home)).filter(|p| Path::new(p).exists()),
    ];

    candidates.into_iter().flatten().next()
}

/// Find the trailmark binary.
fn find_trailmark_binary() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let candidates = [
        which("trailmark"),
        which("trailmark-cli"),
        Some(format!("{}/.cargo/bin/trailmark", home)).filter(|p| Path::new(p).exists()),
        Some(format!("{}/.local/bin/trailmark", home)).filter(|p| Path::new(p).exists()),
        Some(format!("{}/go/bin/trailmark", home)).filter(|p| Path::new(p).exists()),
    ];
    candidates.into_iter().flatten().next()
}

/// Find the githem binary.
fn find_githem_binary() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let candidates = [
        which("githem"),
        Some(format!("{}/.cargo/bin/githem", home)).filter(|p| Path::new(p).exists()),
        Some(format!("{}/.local/bin/githem", home)).filter(|p| Path::new(p).exists()),
    ];
    candidates.into_iter().flatten().next()
}

/// Count all files in a directory recursively.
fn count_files_recursive(dir: &Path) -> usize {
    if !dir.exists() {
        return 0;
    }
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .count()
}

/// Simple `which` substitute (PATH walk).
fn which(name: &str) -> Option<String> {
    let path = std::env::var("PATH").unwrap_or_default();
    for dir in path.split(':') {
        let candidate = format!("{dir}/{name}");
        if Path::new(&candidate).exists() {
            return Some(candidate);
        }
    }
    None
}
