//! Snapshots — capture and baseline current workspace dependency state.
//!
//! `gen-tools deps snapshot` captures the current dependency resolution state
//! across the workspace and writes it to a portable TOML snapshot file. Use
//! `gen-tools deps baseline` to promote the snapshot into the dependency-matrix.toml,
//! freezing the current state as the new matrix reference.
//!
//! Together, these commands let the team detect drift: run `snapshot` to freeze
//! the current reality, then `baseline` to record it, then `scrape --check` in CI
//! to flag any future deviation.

use crate::deps::discover::ProjectRoot;
use crate::deps::matrix::MatrixFile;
use crate::deps::scrape::execute_scrape;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// One snapshot entry — what a single dep resolution looks like right now.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEntry {
    /// Resolved mode: stable, local, git, zk_local, zk_git, inherited, unknown
    pub mode: String,
    /// Human-readable source: crates.io version, path, or git URL.
    pub source: String,
    /// The matrix key (package name from matrix, or dep key if not in matrix).
    pub matrix_key: Option<String>,
    /// Feature flags enabled for this dep in the containing Cargo.toml.
    #[serde(default)]
    pub features: Vec<String>,
}

/// Full snapshot: generated timestamp + per-crate entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencySnapshot {
    pub generated_at: String,
    pub repo_root: String,
    pub crates: HashMap<String, SnapshotEntry>,
}

/// Capture the current dependency state from running scrape + matrix analysis.
pub fn capture_snapshot(
    matrix: &MatrixFile,
    projects: &[ProjectRoot],
    repo_root: &Path,
) -> anyhow::Result<DependencySnapshot> {
    let scrape = execute_scrape(matrix, projects)?;
    let pkg_map = matrix.pkg_to_matrix();

    let mut crates: HashMap<String, SnapshotEntry> = HashMap::new();

    // Add every forked dep from the scrape
    for dep in &scrape.forked_deps {
        let matrix_key = pkg_map.get(&dep.package).map(|(k, _)| k.clone());
        crates.insert(
            dep.package.clone(),
            SnapshotEntry {
                mode: dep.mode.clone(),
                source: dep.source.clone(),
                matrix_key,
                features: dep.features.clone(),
            },
        );
    }

    // Also add matrix crates that weren't found in any Cargo.toml (missing = drift)
    for (name, _entry) in &matrix.crates {
        if !crates.contains_key(name) {
            let pkg = entry_to_pkg_name(_entry, name);
            crates.entry(pkg).or_insert(SnapshotEntry {
                mode: "missing".into(),
                source: "not found in any Cargo.toml".into(),
                matrix_key: Some(name.clone()),
                features: vec![],
            });
        }
    }

    Ok(DependencySnapshot {
        generated_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| format!("unix-{}", d.as_secs()))
            .unwrap_or_else(|_| "unknown".to_string()),
        repo_root: repo_root.to_string_lossy().to_string(),
        crates,
    })
}

/// Write a snapshot to a TOML file.
pub fn write_snapshot(path: &Path, snapshot: &DependencySnapshot) -> anyhow::Result<()> {
    let toml_content = toml::to_string_pretty(snapshot)?;
    std::fs::write(path, toml_content)?;
    Ok(())
}

/// Read a snapshot from a TOML file.
pub fn read_snapshot(path: &Path) -> anyhow::Result<DependencySnapshot> {
    let content = std::fs::read_to_string(path)?;
    let snap: DependencySnapshot = toml::from_str(&content)?;
    Ok(snap)
}

/// Compute drift: compare current scrape against snapshot.
pub fn compute_drift(
    matrix: &MatrixFile,
    snapshot: &DependencySnapshot,
    projects: &[ProjectRoot],
) -> anyhow::Result<DriftReport> {
    let scrape = execute_scrape(matrix, projects)?;
    let mut report = DriftReport::default();

    // Build a set of what the snapshot says should exist
    let snapshot_keys: HashSet<&str> = snapshot.crates.keys().map(|k| k.as_str()).collect();
    let current_keys: HashSet<&str> = scrape
        .forked_deps
        .iter()
        .map(|d| d.package.as_str())
        .collect();

    // Dependencies in snapshot but missing from current workspace
    for key in snapshot_keys.difference(&current_keys) {
        report.missing.push((*key).to_string());
    }

    // Dependencies in current workspace but not in snapshot
    for key in current_keys.difference(&snapshot_keys) {
        report.unexpected.push((*key).to_string());
    }

    // Dependencies with mode changes
    let current_modes: HashMap<&str, &str> = scrape
        .forked_deps
        .iter()
        .map(|d| (d.package.as_str(), d.mode.as_str()))
        .collect();

    for (pkg, entry) in &snapshot.crates {
        if let Some(current_mode) = current_modes.get(pkg.as_str())
            && *current_mode != entry.mode.as_str()
        {
            report.mode_changed.push(ModeChange {
                package: pkg.clone(),
                expected: entry.mode.clone(),
                actual: (*current_mode).to_string(),
            });
        }
    }

    Ok(report)
}

/// Report of differences between a snapshot and current state.
#[derive(Debug, Default)]
pub struct DriftReport {
    pub missing: Vec<String>,
    pub unexpected: Vec<String>,
    pub mode_changed: Vec<ModeChange>,
}

#[derive(Debug, Clone)]
pub struct ModeChange {
    pub package: String,
    pub expected: String,
    pub actual: String,
}

/// Write the snapshot back into the dependency-matrix.toml, creating a new baseline.
///
/// For each crate in the snapshot, updates the matrix entry's mode and source
/// to match the current workspace state. This effectively "freezes" the current
/// state as the new desired baseline.
pub fn write_baseline(matrix_path: &Path, snapshot: &DependencySnapshot) -> anyhow::Result<()> {
    // Read current matrix
    let content = std::fs::read_to_string(matrix_path)?;
    let mut toml_value: toml::Value = toml::from_str(&content)?;

    let crates_table = toml_value
        .get_mut("crates")
        .and_then(|c| c.as_table_mut())
        .ok_or_else(|| anyhow::anyhow!("crates section not found in matrix"))?;

    for (pkg, entry) in &snapshot.crates {
        // Find the matrix key (either direct or by package mapping)
        let matrix_key = entry.matrix_key.as_deref().unwrap_or(pkg);
        if let Some(crate_entry) = crates_table.get_mut(matrix_key)
            && let Some(crate_table) = crate_entry.as_table_mut()
        {
            // Update based on mode
            match entry.mode.as_str() {
                    "stable" => {
                        crate_table
                            .insert("stable".into(), toml::Value::String(entry.source.clone()));
                    }
                    "local" | "zk_local" => {
                        crate_table.insert(
                            if entry.mode == "zk_local" {
                                "zk_local"
                            } else {
                                "local"
                            }
                            .into(),
                            toml::Value::String(entry.source.clone()),
                        );
                    }
                    "git" | "zk_git"
                        // Parse git URL from source (format: "path/to/Cargo.toml")
                        // The git URL and branch are stored in the git/zk_git fields
                        // We preserve existing git config for now — only update the mode
                        // by ensuring git section exists
                        if entry.mode == "zk_git" => {
                            crate_table
                                .entry("zk_git")
                                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
                        }
                    _ => {}
                }
            // Always write features if present — use aggregate key "." to
            // match matrix HashMap<String, Vec<String>> format
            if !entry.features.is_empty() {
                let mut features_map = toml::Table::new();
                let features_val: Vec<toml::Value> = entry
                    .features
                    .iter()
                    .map(|f| toml::Value::String(f.clone()))
                    .collect();
                features_map.insert(".".into(), toml::Value::Array(features_val));
                crate_table.insert("features".into(), toml::Value::Table(features_map));
            }
        }
    }

    // Write back — preserve formatting as much as possible
    let output = toml::to_string_pretty(&toml_value)?;
    std::fs::write(matrix_path, output)?;
    Ok(())
}

fn entry_to_pkg_name(entry: &crate::deps::matrix::CrateEntry, key: &str) -> String {
    entry.package.clone().unwrap_or_else(|| key.to_string())
}
