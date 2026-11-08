//! Project discovery — find all Rust workspace roots under a repo root.
//!
//! Mirrors `dep_common.py`'s discovery logic: walks directories looking for
//! Cargo.toml files with `[workspace]` sections or `cosmwasm-std` deps.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A discovered project root (workspace or standalone crate).
#[derive(Debug, Clone)]
pub struct ProjectRoot {
    /// Human-friendly name (dir path with `/` replaced by `-`).
    pub name: String,
    /// Path relative to repo root.
    pub rel_path: String,
    /// Absolute path to Cargo.toml.
    pub cargo_toml: PathBuf,
    /// Whether this is a workspace (has [workspace.members]).
    pub is_workspace: bool,
    /// Resolved member Cargo.toml paths (for workspaces).
    pub member_tomls: Vec<PathBuf>,
}

impl ProjectRoot {
    pub fn dir(&self) -> &Path {
        self.cargo_toml.parent().unwrap()
    }
}

/// Discover all project roots under `repo_root`.
pub fn discover_projects(repo_root: &Path) -> anyhow::Result<Vec<ProjectRoot>> {
    let mut projects = Vec::new();
    let mut seen = HashSet::new();
    let skip_dirs: HashSet<&str> = [
        ".git",
        "target",
        "_devops",
        "_scripts",
        ".claude",
        "node_modules",
    ]
    .into_iter()
    .collect();

    _discover(
        repo_root,
        repo_root,
        &skip_dirs,
        &mut seen,
        &mut projects,
        4,
    )?;
    Ok(projects)
}

fn _discover(
    repo_root: &Path,
    dir: &Path,
    skip: &HashSet<&str>,
    seen: &mut HashSet<PathBuf>,
    projects: &mut Vec<ProjectRoot>,
    depth: usize,
) -> anyhow::Result<()> {
    if depth == 0 {
        return Ok(());
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_dir() || path.starts_with(".") {
            continue;
        }
        let fname = path.file_name().unwrap().to_string_lossy().to_string();
        if skip.contains(fname.as_str()) || fname.starts_with('.') {
            continue;
        }

        let cargo_toml = path.join("Cargo.toml");
        if cargo_toml.exists() {
            if let Some(proj) = build_project(repo_root, &cargo_toml)
                && seen.insert(cargo_toml.clone())
            {
                projects.push(proj.clone());
                // For workspaces, recurse into excluded members
                if proj.is_workspace {
                    _discover_excluded(repo_root, &cargo_toml, skip, seen, projects)?;
                }
            }
            // Recurse deeper to find sub-projects
            _discover(repo_root, &path, skip, seen, projects, depth - 1)?;
        } else {
            // No Cargo.toml here — recurse
            _discover(repo_root, &path, skip, seen, projects, depth - 1)?;
        }
    }
    Ok(())
}

fn _discover_excluded(
    repo_root: &Path,
    cargo_toml: &Path,
    skip: &HashSet<&str>,
    seen: &mut HashSet<PathBuf>,
    projects: &mut Vec<ProjectRoot>,
) -> anyhow::Result<()> {
    let content = match std::fs::read_to_string(cargo_toml) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };
    let toml: toml::Table = match toml::from_str(&content) {
        Ok(t) => t,
        Err(_) => return Ok(()),
    };
    let excludes = toml
        .get("workspace")
        .and_then(|w| w.get("exclude"))
        .and_then(|e| e.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let ws_dir = cargo_toml.parent().unwrap();
    for excl in excludes {
        let abs = ws_dir.join(&excl);
        let exc_cargo = abs.join("Cargo.toml");
        if exc_cargo.exists() {
            if let Some(proj) = build_project(repo_root, &exc_cargo)
                && seen.insert(exc_cargo)
            {
                projects.push(proj);
            }
        } else if abs.is_dir() {
            _discover(repo_root, &abs, skip, seen, projects, 2)?;
        }
    }
    Ok(())
}

fn build_project(repo_root: &Path, cargo_toml: &Path) -> Option<ProjectRoot> {
    let content = std::fs::read_to_string(cargo_toml).ok()?;
    let data: toml::Table = toml::from_str(&content).ok()?;

    let proj_dir = cargo_toml.parent()?;
    let rel = pathdiff::diff_paths(proj_dir, repo_root)?;
    let rel_str = rel.to_string_lossy().to_string();
    let name = if rel_str.contains('/') {
        rel_str.replace('/', "-")
    } else {
        proj_dir.file_name()?.to_string_lossy().to_string()
    };

    let is_workspace = data
        .get("workspace")
        .and_then(|w| w.get("members"))
        .is_some();

    let member_tomls: Vec<PathBuf> = if is_workspace {
        data["workspace"]["members"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .flat_map(|p| glob::glob(proj_dir.join(p).to_string_lossy().as_ref()).ok())
                    .flatten()
                    .filter_map(|m| match m {
                        Ok(p) => {
                            let c = if p.is_dir() { p.join("Cargo.toml") } else { p };
                            if c.exists() { Some(c) } else { None }
                        }
                        Err(_) => None,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        vec![]
    };

    Some(ProjectRoot {
        name,
        rel_path: rel_str,
        cargo_toml: cargo_toml.to_path_buf(),
        is_workspace,
        member_tomls,
    })
}
