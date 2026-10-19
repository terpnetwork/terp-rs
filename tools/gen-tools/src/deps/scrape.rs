//! Dependency scraping — scan all Cargo.tomls, classify deps, report diagnostics.
//!
//! Mirrors `dep-scrape.py`. Finds all forked dependencies across every
//! Cargo.toml in the repo, cross-references them against the matrix,
//! and reports consistency issues.

use crate::deps::discover::ProjectRoot;
use crate::deps::matrix::MatrixFile;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

/// A single dep reference found in a Cargo.toml.
#[derive(Debug, Clone, Serialize)]
pub struct DepRef {
    /// Published package name (as seen in Cargo.toml).
    pub package: String,
    /// Path to the Cargo.toml that references it.
    pub source: String,
    /// Resolution mode detected: "stable", "local", "git", or "unknown".
    pub mode: String,
    /// Whether this dep is present in the dependency matrix.
    pub in_matrix: bool,
    /// Whether this dep is a known forked crate.
    pub is_fork: bool,
    /// Feature flags enabled for this dep in the containing Cargo.toml.
    #[serde(default)]
    pub features: Vec<String>,
}

/// Full scrape result for a repo.
#[derive(Debug, Clone, Serialize)]
pub struct ScrapeResult {
    pub total_cargo_tomls: usize,
    pub total_deps: usize,
    pub forked_deps: Vec<DepRef>,
    pub orphan_deps: Vec<DepRef>,
    pub unmatched_deps: Vec<DepRef>,
    pub warnings: Vec<String>,
}

/// Scrape all Cargo.tomls in the discovered projects.
pub fn execute_scrape(
    matrix: &MatrixFile,
    projects: &[ProjectRoot],
) -> anyhow::Result<ScrapeResult> {
    let pkg_names = matrix.all_pkg_names();
    let mut result = ScrapeResult {
        total_cargo_tomls: 0,
        total_deps: 0,
        forked_deps: Vec::new(),
        orphan_deps: Vec::new(),
        unmatched_deps: Vec::new(),
        warnings: Vec::new(),
    };

    for project in projects {
        scan_cargo_toml(&project.cargo_toml, &pkg_names, &mut result)?;
        result.total_cargo_tomls += 1;
        for member in &project.member_tomls {
            scan_cargo_toml(member, &pkg_names, &mut result)?;
            result.total_cargo_tomls += 1;
        }
    }

    Ok(result)
}

fn scan_cargo_toml(
    path: &Path,
    pkg_names: &std::collections::HashSet<String>,
    result: &mut ScrapeResult,
) -> anyhow::Result<()> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };
    let data: toml::Table = match toml::from_str(&content) {
        Ok(t) => t,
        Err(_) => return Ok(()),
    };

    let dep_sections = ["dependencies", "dev-dependencies", "build-dependencies"];

    for section in dep_sections {
        let deps = match data.get(section) {
            Some(t) => t.as_table(),
            None => continue,
        };

        if deps.is_none() {
            // Check for inline sections like [dependencies.serde]
            if let Some(tbl) = data.get(section).and_then(|v| v.as_table()) {
                for (name, _value) in tbl {
                    result.total_deps += 1;
                    if pkg_names.contains(name) {
                        let mode = detect_mode_str(Some(_value));
                        result.forked_deps.push(DepRef {
                            package: name.clone(),
                            source: path.to_string_lossy().to_string(),
                            mode,
                            in_matrix: true,
                            is_fork: true,
                            features: extract_features(_value),
                        });
                    }
                }
            }
            continue;
        }

        let table = deps.unwrap();
        for (name, value) in table {
            result.total_deps += 1;
            let is_fork = pkg_names.contains(name);
            let in_matrix = is_fork;

            if is_fork {
                let mode = detect_mode_str(Some(value));
                result.forked_deps.push(DepRef {
                    package: name.clone(),
                    source: path.to_string_lossy().to_string(),
                    mode,
                    in_matrix,
                    is_fork: true,
                    features: extract_features(value),
                });
            } else {
                // Check if this is an orphan — a dep whose workspace dep isn't forked but the
                // member overrides it locally (could be stale)
                let mode = detect_mode_str(Some(value));
                if mode != "unknown" {
                    result.unmatched_deps.push(DepRef {
                        package: name.clone(),
                        source: path.to_string_lossy().to_string(),
                        mode,
                        in_matrix: false,
                        is_fork: false,
                        features: extract_features(value),
                    });
                }
            }
        }
    }

    // Check workspace.dependencies
    if let Some(ws) = data.get("workspace").and_then(|w| w.get("dependencies")) {
        if let Some(table) = ws.as_table() {
            for (name, value) in table {
                if pkg_names.contains(name) {
                    let mode = detect_mode_str(Some(value));
                    result.forked_deps.push(DepRef {
                        package: name.clone(),
                        source: path.to_string_lossy().to_string(),
                        mode,
                        in_matrix: true,
                        is_fork: true,
                        features: extract_features(value),
                    });
                }
            }
        }
    }

    // Check patch.crates-io
    if let Some(patches) = data.get("patch") {
        if let Some(crates_io) = patches.get("crates-io") {
            if let Some(table) = crates_io.as_table() {
                for (name, value) in table {
                    if pkg_names.contains(name) {
                        let mode = detect_mode_str(Some(value));
                        result.forked_deps.push(DepRef {
                            package: name.clone(),
                            source: path.to_string_lossy().to_string(),
                            mode,
                            in_matrix: true,
                            is_fork: true,
                            features: extract_features(value),
                        });
                    }
                }
            }
        }
    }

    Ok(())
}

fn extract_features(dep_value: &toml::Value) -> Vec<String> {
    if let toml::Value::Table(tbl) = dep_value {
        if let Some(toml::Value::Array(arr)) = tbl.get("features") {
            return arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
        }
    }
    vec![]
}

fn detect_mode_str(dep_value: Option<&toml::Value>) -> String {
    match dep_value {
        Some(toml::Value::String(_)) => "stable".into(),
        Some(toml::Value::Table(tbl)) => {
            if tbl.contains_key("git") {
                "git".into()
            } else if tbl.contains_key("path") {
                "local".into()
            } else if tbl.contains_key("workspace") {
                "inherited".into()
            } else {
                "unknown".into()
            }
        }
        _ => "unknown".into(),
    }
}

/// Check for inconsistencies in fork usage.
pub fn validate_scrape(result: &ScrapeResult) -> Vec<String> {
    let mut issues = Vec::new();

    // Group forked deps by package name
    let mut by_pkg: HashMap<&str, Vec<&DepRef>> = HashMap::new();
    for dep in &result.forked_deps {
        by_pkg.entry(&dep.package).or_default().push(dep);
    }

    for (pkg, refs) in &by_pkg {
        let modes: std::collections::HashSet<&str> = refs.iter().map(|r| r.mode.as_str()).collect();
        if modes.len() > 1 {
            issues.push(format!("inconsistent mode for '{}': {:?}", pkg, modes));
        }
    }

    issues
}
