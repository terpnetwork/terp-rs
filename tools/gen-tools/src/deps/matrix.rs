//! Dependency matrix — single source of truth for forked crate definitions.
//!
//! Maps to `_devops/dependency-matrix.toml`. Each entry documents a forked
//! crate across all resolution modes (stable/local/git/zk).

use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Top-level `dependency-matrix.toml` structure.
#[derive(Debug, Deserialize)]
pub struct MatrixFile {
    #[serde(default)]
    pub defaults: Defaults,
    pub crates: HashMap<String, CrateEntry>,
}

/// Global default branches.
#[derive(Debug, Deserialize, Default)]
pub struct Defaults {
    #[serde(default = "default_main")]
    pub git_branch: String,
    #[serde(default = "default_zk_mvp")]
    pub zk_git_branch: String,
}

fn default_main() -> String {
    "main".into()
}
fn default_zk_mvp() -> String {
    "zk-mvp".into()
}

/// A single matrix crate entry.
#[derive(Debug, Deserialize, Clone)]
pub struct CrateEntry {
    pub description: Option<String>,
    /// Published package name (if different from the matrix key).
    #[serde(default)]
    pub package: Option<String>,
    /// crates.io version for stable builds.
    pub stable: Option<String>,
    /// Git fork details.
    #[serde(default)]
    pub git: Option<GitSource>,
    /// Local path relative to repo root.
    #[serde(default)]
    pub local: Option<String>,
    /// ZK variant git source.
    #[serde(default)]
    pub zk_git: Option<GitSource>,
    /// ZK variant local path.
    #[serde(default)]
    pub zk_local: Option<String>,
    /// Alternate names for this crate (dep aliases).
    #[serde(default)]
    pub dep_aliases: Vec<String>,
    /// Workspace directories that consume this crate.
    #[serde(default)]
    pub consumers: Vec<String>,
    /// Repo-level metadata (not a publishable crate).
    #[serde(default)]
    pub repo_only: Option<bool>,
    /// Feature flags used across consumers — maps Cargo.toml path to features.
    /// Allows per-consumer feature tracking so switching preserves the right features.
    #[serde(default)]
    pub features: std::collections::HashMap<String, Vec<String>>,
}

/// Git source: URL + optional branch. URL can be omitted for ZK overrides
/// that inherit the URL from the main git source.
#[derive(Debug, Deserialize, Clone)]
pub struct GitSource {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
}

impl MatrixFile {
    /// Load and parse a dependency-matrix.toml file.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let matrix: MatrixFile = toml::from_str(&content)?;
        Ok(matrix)
    }

    /// All published package names from the matrix (keys + package fields + dep_aliases).
    pub fn all_pkg_names(&self) -> HashSet<String> {
        let mut names = HashSet::new();
        for (key, entry) in &self.crates {
            if entry.repo_only.unwrap_or(false) {
                continue;
            }
            names.insert(entry.package.clone().unwrap_or_else(|| key.clone()));
            for alias in &entry.dep_aliases {
                names.insert(alias.clone());
            }
        }
        names
    }

    /// Map published package name → (matrix_key, CrateEntry).
    pub fn pkg_to_matrix(&self) -> HashMap<String, (String, &CrateEntry)> {
        let mut map = HashMap::new();
        for (key, entry) in &self.crates {
            if entry.repo_only.unwrap_or(false) {
                continue;
            }
            let pkg = entry.package.clone().unwrap_or_else(|| key.clone());
            map.insert(pkg.clone(), (key.clone(), entry));
            for alias in &entry.dep_aliases {
                map.insert(alias.clone(), (key.clone(), entry));
            }
        }
        map
    }

    /// Git source for a crate in the given mode.
    /// Returns (url, branch) or None if not configured.
    pub fn resolve_git(&self, key: &str, mode: &str) -> Option<(String, String)> {
        let entry = self.crates.get(key)?;
        if entry.repo_only.unwrap_or(false) {
            return None;
        }
        let git = entry.git.as_ref()?;
        match mode {
            "git" | "dev" => {
                let url = git.url.clone()?;
                let branch = git
                    .branch
                    .clone()
                    .or_else(|| Some(self.defaults.git_branch.clone()))
                    .unwrap_or_else(|| "main".into());
                Some((url, branch))
            }
            "zk_git" | "zk_dev" => {
                let zg = entry.zk_git.as_ref().or(Some(git))?;
                let url = zg.url.clone().or_else(|| git.url.clone())?;
                let branch = zg
                    .branch
                    .clone()
                    .or_else(|| Some(self.defaults.zk_git_branch.clone()))
                    .unwrap_or_else(|| git.branch.clone().unwrap_or_else(|| "main".into()));
                Some((url, branch))
            }
            _ => None,
        }
    }

    /// Local path for a crate in the given mode (local or zk_local).
    pub fn resolve_local(&self, key: &str, mode: &str) -> Option<String> {
        let entry = self.crates.get(key)?;
        if entry.repo_only.unwrap_or(false) {
            return None;
        }
        match mode {
            "local" | "dev" => entry.local.clone(),
            "zk_local" | "zk_dev" => entry.zk_local.clone().or_else(|| entry.local.clone()),
            _ => None,
        }
    }
}
