//! Dependency switching — rewrite Cargo.toml files between stable/local/git modes.
//!
//! `patch-local` and `patch-git` (version lines, one `[patch.crates-io]`
//! replacement) live in `dep.py` only. This Rust switch does not emit them.
//!
//! Mirrors `dep-switch.py`. Edits three target levels:
//!   - `patches`:  [patch.crates-io] in workspace root
//!   - `ws-deps`:  [workspace.dependencies] entries
//!   - `members`:  direct deps in member Cargo.tomls

use crate::deps::discover::ProjectRoot;
use crate::deps::matrix::MatrixFile;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Switch configuration.
pub struct SwitchConfig<'a> {
    pub matrix: &'a MatrixFile,
    pub repo_root: &'a Path,
    pub projects: &'a [ProjectRoot],
    pub mode: &'a str,
    pub target: &'a str,
    pub target_workspace: Option<&'a str>,
    pub target_crate: Option<&'a str>,
    pub dry_run: bool,
}

/// Result of a single file edit.
#[derive(Debug)]
pub struct FileEdit {
    pub path: PathBuf,
    pub action: String,
}

/// Execute a dependency switch across all targets.
pub fn execute_switch(config: &SwitchConfig) -> anyhow::Result<Vec<FileEdit>> {
    let mut edits = Vec::new();
    let pkg_map = config.matrix.pkg_to_matrix();
    let all_pkg_names = config.matrix.all_pkg_names();

    let filtered_projects: Vec<&ProjectRoot> = config
        .projects
        .iter()
        .filter(|p| {
            if let Some(ws) = config.target_workspace {
                p.rel_path == ws || p.name == ws
            } else {
                true
            }
        })
        .collect();

    for project in &filtered_projects {
        let ws_root = project.cargo_toml.parent().unwrap();

        // 1. Edit patches
        if config.target == "patches" || config.target == "all" {
            let result = edit_patches(
                ws_root,
                config.matrix,
                config.mode,
                &pkg_map,
                &all_pkg_names,
                config.dry_run,
            );
            edits.extend(result?);
        }

        // 2. Edit workspace deps
        if (config.target == "ws-deps" || config.target == "all") && project.is_workspace {
            edits.extend(edit_ws_deps(
                &project.cargo_toml,
                config.matrix,
                config.mode,
                &pkg_map,
                config.target_crate,
                config.dry_run,
            )?);
        }

        // 3. Edit member deps
        if config.target == "members" || config.target == "all" {
            for member_cargo in &project.member_tomls {
                edits.extend(edit_member_deps(
                    member_cargo,
                    ws_root,
                    config.matrix,
                    config.mode,
                    &pkg_map,
                    config.target_crate,
                    config.dry_run,
                )?);
            }
        }
    }

    Ok(edits)
}

// ── Patch editing ──────────────────────────────────────────────────────

fn edit_patches(
    ws_root: &Path,
    matrix: &MatrixFile,
    mode: &str,
    pkg_map: &HashMap<String, (String, &crate::deps::matrix::CrateEntry)>,
    _all_pkg_names: &HashSet<String>,
    dry_run: bool,
) -> anyhow::Result<Vec<FileEdit>> {
    let cargo_toml = ws_root.join("Cargo.toml");
    if !cargo_toml.exists() {
        return Ok(vec![]);
    }
    let content = match std::fs::read_to_string(&cargo_toml) {
        Ok(c) => c,
        Err(_) => return Ok(vec![]),
    };
    let mut edits = Vec::new();
let mut changed = false;

    let mut lines: Vec<String> = content.lines().map(String::from).collect();
    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];
        if line.trim().starts_with("[patch.")
            && line.contains("crates-io")
        {
            i += 1;
            let mut patch_lines: Vec<(usize, String)> = Vec::new();
            while i < lines.len() && !lines[i].trim().starts_with('[') {
                if !lines[i].trim().is_empty() && !lines[i].trim().starts_with('#') {
                    let trimmed = lines[i].trim().to_string();
                    if let Some(dep_name) = trimmed.split('=').next() {
                        let dep_name = dep_name.trim();
                        if let Some((_key, entry)) = pkg_map.get(dep_name) {
                            let base = build_patch_entry(dep_name, entry, mode, matrix);
                            let features = extract_features_from_line(&trimmed).unwrap_or_default();
                            let replacement = append_features_to_line(&base, &features);
                            if replacement != trimmed {
                                patch_lines.push((i, format!("{}{}", leading_whitespace(&lines[i]), replacement)));
                                changed = true;
                            }
                        }
                    }
                }
                i += 1;
            }
            for (idx, new_line) in &patch_lines {
                lines[*idx] = new_line.clone();
            }
        } else {
            i += 1;
        }
    }

    if changed {
        let new_content = lines.join("\n");
        if !dry_run {
            std::fs::write(&cargo_toml, &new_content)?;
        }
        edits.push(FileEdit {
            path: cargo_toml,
            action: if dry_run {
                "[dry-run] would update patches".into()
            } else {
                "updated patches".into()
            },
        });
    }

    Ok(edits)
}

// ── Workspace deps editing ─────────────────────────────────────────────

fn edit_ws_deps(
    cargo_toml: &Path,
    matrix: &MatrixFile,
    mode: &str,
    pkg_map: &HashMap<String, (String, &crate::deps::matrix::CrateEntry)>,
    target_crate: Option<&str>,
    dry_run: bool,
) -> anyhow::Result<Vec<FileEdit>> {
    let content = match std::fs::read_to_string(cargo_toml) {
        Ok(c) => c,
        Err(_) => return Ok(vec![]),
    };
    let mut changed = false;

    let mut lines: Vec<String> = content.lines().map(String::from).collect();
    let mut in_ws_deps = false;
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim().to_string();
        if trimmed == "[workspace.dependencies]" {
            in_ws_deps = true;
            i += 1;
            continue;
        }
        if in_ws_deps {
            // Reached next section
            if trimmed.starts_with('[') {
                in_ws_deps = false;
                i += 1;
                continue;
            }
            // Parse dep name from inline or table entry
            if let Some(dep_name) = parse_dep_name_from_line(&trimmed) {
                if let Some((_key, entry)) = pkg_map.get(&dep_name) {
                    if let Some(tc) = target_crate {
                        if &dep_name != tc && pkg_map.get(tc).map(|(k, _)| k.as_str()) != Some(&dep_name) {
                            i += 1;
                            continue;
                        }
                    }
                    let (new_lines, modified) = rewrite_ws_dep_entry(&lines, i, &dep_name, entry, mode, matrix);
                    if modified {
                        // Replace the entry lines
                        for (offset, nl) in new_lines.iter().enumerate() {
                            if i + offset < lines.len() {
                                lines[i + offset] = nl.clone();
                            }
                        }
                        changed = true;
                        i += new_lines.len();
                        continue;
                    }
                }
            }
        }
        i += 1;
    }

    if changed {
        let new_content = lines.join("\n");
        if !dry_run {
            std::fs::write(cargo_toml, &new_content)?;
        }
    }
    let action = if changed {
        if dry_run { "[dry-run] would update ws-deps".into() } else { "updated ws-deps".into() }
    } else {
        "no changes to ws-deps".into()
    };
    Ok(vec![FileEdit { path: cargo_toml.to_path_buf(), action }])
}

// ── Member deps editing ────────────────────────────────────────────────

fn edit_member_deps(
    member_cargo: &Path,
    _ws_root: &Path,
    matrix: &MatrixFile,
    mode: &str,
    pkg_map: &HashMap<String, (String, &crate::deps::matrix::CrateEntry)>,
    target_crate: Option<&str>,
    dry_run: bool,
) -> anyhow::Result<Vec<FileEdit>> {
    let content = match std::fs::read_to_string(member_cargo) {
        Ok(c) => c,
        Err(_) => return Ok(vec![]),
    };
    let mut changed = false;

    let mut lines: Vec<String> = content.lines().map(String::from).collect();
    let mut in_deps = false;
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim().to_string();
        // Detect dependency sections
        if trimmed.starts_with("[dependencies]")
            || trimmed.starts_with("[dev-dependencies]")
            || trimmed.starts_with("[build-dependencies]")
        {
            in_deps = true;
            i += 1;
            continue;
        }
        if in_ws_deps_section(&trimmed) {
            in_deps = false;
            i += 1;
            continue;
        }
        if in_deps {
            if trimmed.starts_with('[') {
                in_deps = false;
                i += 1;
                continue;
            }
            if let Some(dep_name) = parse_dep_name_from_line(&trimmed) {
                // Skip workspace-inherited deps
                if trimmed.contains("workspace = true") || trimmed.contains("workspace=true") {
                    i += 1;
                    continue;
                }
                if let Some((_key, entry)) = pkg_map.get(&dep_name) {
                    if let Some(tc) = target_crate {
                        if &dep_name != tc {
                            i += 1;
                            continue;
                        }
                    }
                    let base = build_dep_entry(&dep_name, entry, mode, matrix, 0);
                    let features = extract_features_from_line(&trimmed).unwrap_or_default();
                    let replacement = append_features_to_line(&base, &features);
                    if replacement != trimmed {
                        lines[i] = replacement;
                        changed = true;
                    }
                }
            }
        }
        i += 1;
    }

    if changed {
        let new_content = lines.join("\n");
        if !dry_run {
            std::fs::write(member_cargo, &new_content)?;
        }
    }
    let action = if changed {
        if dry_run { "[dry-run] would update member deps".into() } else { "updated member deps".into() }
    } else {
        "no changes".into()
    };
    Ok(vec![FileEdit { path: member_cargo.to_path_buf(), action }])
}

// ── Helpers ────────────────────────────────────────────────────────────

fn leading_whitespace(s: &str) -> String {
    let len = s.len() - s.trim_start().len();
    s[..len].to_string()
}

fn parse_dep_name_from_line(line: &str) -> Option<String> {
    // inline: `dep-name = { ... }` or table: `dep-name = "1.0"`
    // table entry: `[dependencies.dep-name]` or just `dep-name =`
    if line.contains('=') {
        let name = line.split('=').next()?.trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    None
}

fn in_ws_deps_section(trimmed: &str) -> bool {
    trimmed == "[workspace.dependencies]" || trimmed.starts_with("[workspace.")
}

/// Extract features = [...] clause from an original dep line.
fn extract_features_from_line(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    if let Some(start) = trimmed.find("features") {
        let after = &trimmed[start..];
        if let Some(bracket_start) = after.find('[') {
            if let Some(bracket_end) = after[bracket_start..].find(']') {
                let content = &after[bracket_start + 1..bracket_start + bracket_end];
                let features: Vec<String> = content
                    .split(',')
                    .map(|s| s.trim().trim_matches('"').to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if !features.is_empty() {
                    return Some(features);
                }
            }
        }
    }
    None
}

/// Append features to a dep replacement line.
/// e.g. `dep = { path = ".." }` + ["staking"] → `dep = { path = "..", features = ["staking"] }`
fn append_features_to_line(line: &str, features: &[String]) -> String {
    if features.is_empty() {
        return line.to_string();
    }
    // Only append if the line has an inline table (contains { and })
    if !line.contains('{') || !line.contains('}') {
        return line.to_string();
    }
    let features_tail = format!(
        ", features = [{}]",
        features
            .iter()
            .map(|f| format!("\"{}\"", f))
            .collect::<Vec<_>>()
            .join(", ")
    );
    // Insert features before the closing brace
    if let Some(pos) = line.rfind('}') {
        let mut result = line[..pos].to_string();
        // Don't add featuress if already present
        if !result.contains("features") {
            result.push_str(&features_tail);
        }
        result.push('}');
        return result;
    }
    line.to_string()
}

fn build_patch_entry(
    dep_name: &str,
    entry: &crate::deps::matrix::CrateEntry,
    mode: &str,
    matrix: &MatrixFile,
) -> String {
    let key = entry
        .package
        .clone()
        .unwrap_or_else(|| dep_name.to_string());
    match mode {
        "stable" => format!(
            "{} = \"{}\"",
            dep_name,
            entry.stable.as_deref().unwrap_or("0.0.0")
        ),
        "local" | "dev" => {
            if let Some(local) = matrix.resolve_local(&key, "local") {
                format!("{} = {{ path = \"{}\" }}", dep_name, local)
            } else {
                format!(
                    "{} = \"{}\"",
                    dep_name,
                    entry.stable.as_deref().unwrap_or("0.0.0")
                )
            }
        }
        "git" => {
            if let Some((url, branch)) = matrix.resolve_git(&key, "git") {
                format!(
                    "{} = {{ git = \"{}\", branch = \"{}\" }}",
                    dep_name, url, branch
                )
            } else {
                format!(
                    "{} = \"{}\"",
                    dep_name,
                    entry.stable.as_deref().unwrap_or("0.0.0")
                )
            }
        }
        "zk_local" | "zk_dev" => {
            if let Some(local) = matrix.resolve_local(&key, "zk_local") {
                format!("{} = {{ path = \"{}\" }}", dep_name, local)
            } else if let Some(local) = matrix.resolve_local(&key, "local") {
                format!("{} = {{ path = \"{}\" }}", dep_name, local)
            } else {
                format!(
                    "{} = \"{}\"",
                    dep_name,
                    entry.stable.as_deref().unwrap_or("0.0.0")
                )
            }
        }
        "zk_git" => {
            if let Some((url, branch)) = matrix.resolve_git(&key, "zk_git") {
                format!(
                    "{} = {{ git = \"{}\", branch = \"{}\" }}",
                    dep_name, url, branch
                )
            } else if let Some((url, branch)) = matrix.resolve_git(&key, "git") {
                format!(
                    "{} = {{ git = \"{}\", branch = \"{}\" }}",
                    dep_name, url, branch
                )
            } else {
                format!(
                    "{} = \"{}\"",
                    dep_name,
                    entry.stable.as_deref().unwrap_or("0.0.0")
                )
            }
        }
        _ => format!(
            "{} = \"{}\"",
            dep_name,
            entry.stable.as_deref().unwrap_or("0.0.0")
        ),
    }
}

fn build_dep_entry(
    dep_name: &str,
    entry: &crate::deps::matrix::CrateEntry,
    mode: &str,
    matrix: &MatrixFile,
    _indent: usize,
) -> String {
    let key = entry
        .package
        .clone()
        .unwrap_or_else(|| dep_name.to_string());
    match mode {
        "stable" => format!(
            "{} = \"{}\"",
            dep_name,
            entry.stable.as_deref().unwrap_or("0.0.0")
        ),
        "local" | "dev" => {
            if let Some(local) = matrix.resolve_local(&key, "local") {
                format!("{} = {{ path = \"{}\" }}", dep_name, local)
            } else {
                format!(
                    "{} = \"{}\"",
                    dep_name,
                    entry.stable.as_deref().unwrap_or("0.0.0")
                )
            }
        }
        "git" => {
            if let Some((url, branch)) = matrix.resolve_git(&key, "git") {
                format!(
                    "{} = {{ git = \"{}\", branch = \"{}\" }}",
                    dep_name, url, branch
                )
            } else {
                format!(
                    "{} = \"{}\"",
                    dep_name,
                    entry.stable.as_deref().unwrap_or("0.0.0")
                )
            }
        }
        _ => format!(
            "{} = \"{}\"",
            dep_name,
            entry.stable.as_deref().unwrap_or("0.0.0")
        ),
    }
}
    fn rewrite_ws_dep_entry(
    lines: &[String],
    idx: usize,
    dep_name: &str,
    entry: &crate::deps::matrix::CrateEntry,
    mode: &str,
    matrix: &MatrixFile,
) -> (Vec<String>, bool) {
    let line = &lines[idx];
    let trimmed = line.trim();

    // Determine if this is a table entry or inline entry
    if trimmed.contains('=') {
        // Inline entry
        let base = build_dep_entry(dep_name, entry, mode, matrix, 0);
        let features = extract_features_from_line(&trimmed).unwrap_or_default();
        let new_line = append_features_to_line(&base, &features);
        (vec![new_line.clone()], new_line != trimmed)
    } else {
        // Could be a table entry [dependencies.dep-name] — skip
        (vec![line.clone()], false)
    }
}

#[allow(dead_code)]
fn resolve_local_path(_ws_root: &Path, local: &str) -> String {
    // For now, just return the local path as-is
    format!("\"../{}\"", local.trim_start_matches("./"))
}