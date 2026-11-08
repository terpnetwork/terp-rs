//! Runner for `gen-tools deps <subcommand>`.
//!
//! Dispatches to matrix loading, project discovery, and the appropriate
//! switch/scrape/graph/update/push modules.

use gen_tools::deps::DepsCommand;
use gen_tools::deps::discover::discover_projects;
use gen_tools::deps::graph;
use gen_tools::deps::matrix::MatrixFile;
use gen_tools::deps::scrape;
use gen_tools::deps::switch::{self, SwitchConfig};

/// Execute a deps subcommand.
pub fn run_deps_command(cmd: &DepsCommand) -> anyhow::Result<()> {
    let repo_root = discover_repo_root()?;

    // Load matrix from _devops/ in various possible locations
    let matrix_paths = [
        repo_root.join("_devops").join("dependency-matrix.toml"),
        repo_root
            .join("crates")
            .join("_devops")
            .join("dependency-matrix.toml"),
        repo_root.join("dependency-matrix.toml"),
        // New location: inside gen-tools crate
        repo_root
            .join("crates")
            .join("terp-rs")
            .join("tools")
            .join("gen-tools")
            .join("_devops")
            .join("dependency-matrix.toml"),
    ];

    let matrix_path = matrix_paths.iter().find(|p| p.exists()).ok_or_else(|| {
        anyhow::anyhow!(
            "dependency-matrix.toml not found in ./_devops/ or ./. Try running from the repo root."
        )
    })?;

    let matrix = MatrixFile::load(matrix_path)?;

    // Discover projects unless not needed
    let projects = match cmd {
        DepsCommand::Status
        | DepsCommand::Scrape { .. }
        | DepsCommand::Switch { .. }
        | DepsCommand::Snapshot { .. }
        | DepsCommand::Drift { .. } => discover_projects(&repo_root)?,
        _ => vec![],
    };

    match cmd {
        DepsCommand::Switch {
            mode,
            target,
            workspace,
            crate_name,
            dry_run,
        } => {
            let config = SwitchConfig {
                matrix: &matrix,
                repo_root: &repo_root,
                projects: &projects,
                mode,
                target,
                target_workspace: workspace.as_deref(),
                target_crate: crate_name.as_deref(),
                dry_run: *dry_run,
            };

            let edits = switch::execute_switch(&config)?;

            if edits.is_empty() {
                println!("no edits needed — all deps already in '{}' mode", mode);
            } else {
                for edit in &edits {
                    println!("{}  {}", if *dry_run { "  " } else { "?" }, edit.action);
                }
            }
        }

        DepsCommand::Status => {
            println!("=== Dependency Status ===");
            println!("Repository: {}", repo_root.display());
            println!("Matrix: {} crates defined", matrix.crates.len());
            println!("Projects: {} discovered", projects.len());

            let result = scrape::execute_scrape(&matrix, &projects)?;
            let mut by_mode: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            for dep in &result.forked_deps {
                *by_mode.entry(dep.mode.clone()).or_insert(0) += 1;
            }

            println!("\nForked dep statistics:");
            for (mode_val, count) in &by_mode {
                println!("  {:10} -> {} references", mode_val, count);
            }

            let issues = scrape::validate_scrape(&result);
            if !issues.is_empty() {
                println!("\nWarnings:");
                for issue in &issues {
                    println!("  ? {}", issue);
                }
            }
        }

        DepsCommand::Scrape { check, json } => {
            let result = scrape::execute_scrape(&matrix, &projects)?;

            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("=== Scrape Results ===");
                println!(
                    "Scanned {} Cargo.toml files, {} total deps",
                    result.total_cargo_tomls, result.total_deps
                );
                println!(
                    "  {} forked deps ({} unique warnings)",
                    result.forked_deps.len(),
                    scrape::validate_scrape(&result).len()
                );

                if !result.warnings.is_empty() {
                    println!("\nWarnings:");
                    for w in &result.warnings {
                        println!("  ? {}", w);
                    }
                }
            }

            if *check && !scrape::validate_scrape(&result).is_empty() {
                std::process::exit(1);
            }
        }

        DepsCommand::Overview { output } => {
            let path = output.clone().unwrap_or_else(|| "OVERVIEW.md".to_string());
            let result = scrape::execute_scrape(&matrix, &projects)?;
            let mut md = String::new();

            md.push_str("# Dependency Overview\n\n");
            md.push_str(&format!(
                "Generated: {}\n\n",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| format!("unix-{}", d.as_secs()))
                    .unwrap_or_else(|_| "unknown".to_string())
            ));
            md.push_str(&format!("- **Repository**: {}\n", repo_root.display()));
            md.push_str(&format!("- **Matrix crates**: {}\n", matrix.crates.len()));
            md.push_str(&format!(
                "- **Projects discovered**: {}\n\n",
                projects.len()
            ));

            md.push_str("## Forked Dependencies\n\n");
            md.push_str("| Package | Mode | Source |\n");
            md.push_str("|---------|------|--------|\n");
            for dep in &result.forked_deps {
                md.push_str(&format!(
                    "| {} | {} | {} |\n",
                    dep.package, dep.mode, dep.source
                ));
            }

            std::fs::write(&path, &md)?;
            println!("wrote {}", path);
        }

        DepsCommand::Graph { svg, order, dot } => {
            let topo = graph::topo_push_order(&matrix);

            if *order {
                println!("=== Push Order (topological) ===\n");
                for (i, node) in topo.iter().enumerate() {
                    println!(
                        "{:3}. {}  {}",
                        i + 1,
                        node.key,
                        node.description.as_deref().unwrap_or("")
                    );
                }
                return Ok(());
            }

            let dot_str = graph::generate_dot(&matrix);

            if *dot {
                println!("{}", dot_str);
                return Ok(());
            }

            if *svg {
                let dot_path = "deps.dot";
                std::fs::write(dot_path, &dot_str)?;

                let result = std::process::Command::new("dot")
                    .args(["-Tsvg", dot_path, "-o", "deps.svg"])
                    .output()?;

                if result.status.success() {
                    println!("wrote deps.svg");
                } else {
                    eprintln!(
                        "failed to render SVG (graphviz 'dot' must be installed): {}",
                        String::from_utf8_lossy(&result.stderr)
                    );
                }

                std::fs::remove_file(dot_path).ok();
                return Ok(());
            }

            println!("{}", dot_str);
        }

        DepsCommand::Update { dry_run } => {
            let results = graph::execute_update(&matrix, *dry_run)?;
            for r in &results {
                println!("{}", r);
            }
        }

        DepsCommand::Push { mode, dry_run } => {
            let results = graph::execute_push(&matrix, mode, *dry_run)?;
            for r in &results {
                println!("{}", r);
            }
        }

        DepsCommand::Snapshot { output, check } => {
            use gen_tools::deps::snapshot::{capture_snapshot, compute_drift, write_snapshot};

            let out_path = repo_root.join(output);
            let snap = capture_snapshot(&matrix, &projects, &repo_root)?;
            write_snapshot(&out_path, &snap)?;
            println!("snapshot written to {}", out_path.display());

            if *check {
                let drift = compute_drift(&matrix, &snap, &projects)?;
                if !drift.missing.is_empty() {
                    println!("\n  missing from workspace: {}", drift.missing.join(", "));
                }
                if !drift.unexpected.is_empty() {
                    println!(
                        "  unexpected (in workspace but not in snapshot): {}",
                        drift.unexpected.join(", ")
                    );
                }
                if !drift.mode_changed.is_empty() {
                    println!("\n  mode changes:");
                    for mc in &drift.mode_changed {
                        println!("    {}: {} → {}", mc.package, mc.expected, mc.actual);
                    }
                }
                if drift.missing.is_empty()
                    && drift.unexpected.is_empty()
                    && drift.mode_changed.is_empty()
                {
                    println!("  no drift detected — snapshot matches current state");
                }
            }
        }

        DepsCommand::Baseline { input, dry_run } => {
            use gen_tools::deps::snapshot::{read_snapshot, write_baseline};

            let snap_path = repo_root.join(input);
            let snap = read_snapshot(&snap_path)?;
            println!(
                "loading snapshot from {} ({} crates)",
                snap_path.display(),
                snap.crates.len()
            );

            if *dry_run {
                println!(
                    "[dry-run] would write {} crates as new baseline",
                    snap.crates.len()
                );
            } else {
                write_baseline(matrix_path, &snap)?;
                println!("baseline written to {}", matrix_path.display());
            }
        }

        DepsCommand::Drift { baseline } => {
            use gen_tools::deps::snapshot::{compute_drift, read_snapshot};

            let snap_path = repo_root.join(baseline);
            let snap = read_snapshot(&snap_path)?;
            let drift = compute_drift(&matrix, &snap, &projects)?;

            println!("=== Drift Report ===");
            println!(
                "Baseline: {} ({} crates)",
                snap_path.display(),
                snap.crates.len()
            );

            if !drift.missing.is_empty() {
                println!("\n  missing from workspace:");
                for pkg in &drift.missing {
                    println!("    - {}", pkg);
                }
            }

            if !drift.unexpected.is_empty() {
                println!("\n  unexpected additions in workspace:");
                for pkg in &drift.unexpected {
                    println!("    + {}", pkg);
                }
            }

            if !drift.mode_changed.is_empty() {
                println!("\n  mode changes:");
                for mc in &drift.mode_changed {
                    println!("    {}: {} → {}", mc.package, mc.expected, mc.actual);
                }
            }

            if drift.missing.is_empty()
                && drift.unexpected.is_empty()
                && drift.mode_changed.is_empty()
            {
                println!("  no drift — workspace matches baseline");
            }

            if !drift.missing.is_empty() || !drift.mode_changed.is_empty() {
                println!(
                    "\n  TIP: run `gen-tools deps snapshot` to capture current state, then `gen-tools deps baseline` to freeze it"
                );
            }
        }
    }

    Ok(())
}

/// Walk up from cwd to find the repo root (first dir with Cargo.toml containing [workspace]).
fn discover_repo_root() -> anyhow::Result<std::path::PathBuf> {
    let mut current = std::env::current_dir()?;
    let mut deepest_workspace: Option<std::path::PathBuf> = None;
    loop {
        // Check for .git directory — the repo root always has a real .git dir (not a submodule file)
        if current.join(".git").is_dir() {
            return Ok(current);
        }
        // Remember the deepest Cargo.toml with [workspace] as fallback
        let cargo_toml = current.join("Cargo.toml");
        if cargo_toml.exists()
            && let Ok(content) = std::fs::read_to_string(&cargo_toml)
            && content.contains("[workspace]")
        {
            deepest_workspace = Some(current.clone());
        }
        if !current.pop() {
            // No .git found anywhere — use the deepest workspace root as fallback
            if let Some(root) = deepest_workspace {
                return Ok(root);
            }
            anyhow::bail!("could not find repo root (no .git or Cargo.toml with [workspace] found)")
        }
    }
}
