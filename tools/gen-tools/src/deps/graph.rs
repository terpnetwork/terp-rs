//! Dependency graph — topological ordering, cargo update, git push.
//!
//! Mirrors `dep-graph.py`. Builds a dependency graph from the matrix
//! `consumers` field and produces a topological order for push/update
//! operations.

use crate::deps::matrix::MatrixFile;
use std::collections::{HashMap, HashSet, VecDeque};

/// A node in the dependency graph.
#[derive(Debug, Clone)]
pub struct GraphNode {
    pub key: String,
    pub description: Option<String>,
    pub depth: usize,
}

/// Topologically sorted push order (dependencies first).
pub fn topo_push_order(matrix: &MatrixFile) -> Vec<GraphNode> {
    // Build adjacency: which crates depend on which
    let all_keys: Vec<String> = matrix
        .crates
        .iter()
        .filter(|(_, e)| !e.repo_only.unwrap_or(false))
        .map(|(k, _)| k.clone())
        .collect();

    let mut consumers_map: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut has_incoming: HashSet<&str> = HashSet::new();

    for (key, entry) in &matrix.crates {
        if entry.repo_only.unwrap_or(false) {
            continue;
        }
        for consumer in &entry.consumers {
            // consumer is a crate name that depends on `key`
            consumers_map.entry(consumer).or_default().push(key);
            has_incoming.insert(key.as_str());
        }
    }

    // Find roots (crates with no dependents — downstream consumers are pushed first)
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();

    // consumer depends on producer: edge producer -> consumer
    for key in matrix.crates.keys() {
        if !matrix.crates[key].repo_only.unwrap_or(false) {
            in_degree.entry(key).or_insert(0);
            adjacency.entry(key).or_default();
        }
    }

    for (consumer, producers) in &consumers_map {
        for producer in producers {
            adjacency.entry(producer).or_default();
            in_degree.entry(consumer).or_insert(0);
            *in_degree.get_mut(consumer).unwrap() += 1;
        }
    }

    // Kahn's algorithm
    let mut queue: VecDeque<&str> = VecDeque::new();
    for (key, deg) in &in_degree {
        if *deg == 0 && all_keys.contains(&key.to_string()) {
            queue.push_back(key);
        }
    }

    let mut order = Vec::new();
    while let Some(node) = queue.pop_front() {
        let entry = &matrix.crates[node];
        order.push(GraphNode {
            key: node.to_string(),
            description: entry.description.clone(),
            depth: 0,
        });

        if let Some(children) = adjacency.get(node) {
            for child in children {
                if let Some(deg) = in_degree.get_mut(child) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(child);
                    }
                }
            }
        }
    }

    order
}

/// Generate a DOT graph visualization string.
pub fn generate_dot(matrix: &MatrixFile) -> String {
    let mut dot = String::from("digraph deps {\n");
    dot.push_str("    rankdir=LR;\n");
    dot.push_str("    node [shape=box, style=rounded];\n");

    let keys: Vec<&str> = matrix.crates.keys().map(|k| k.as_str()).collect();

    for key in &keys {
        let safe = key.replace('-', "_");
        let label = key;
        dot.push_str(&format!("    {} [label=\"{}\"];\n", safe, label));
    }

    for (key, entry) in &matrix.crates {
        let from = key.replace('-', "_");
        for consumer in &entry.consumers {
            let to = consumer.replace('-', "_");
            dot.push_str(&format!("    {} -> {};\n", from, to));
        }
    }

    dot.push_str("}\n");
    dot
}

/// Run `cargo update` in topological order.
pub fn execute_update(matrix: &MatrixFile, dry_run: bool) -> anyhow::Result<Vec<String>> {
    let order = topo_push_order(matrix);
    let mut results = Vec::new();

    for node in &order {
        if dry_run {
            results.push(format!(
                "[dry-run] cargo update -p {} (depth {})",
                node.key, node.depth
            ));
        } else {
            let output = std::process::Command::new("cargo")
                .args(["update", "-p", &node.key])
                .output()?;
            if output.status.success() {
                results.push(format!("✓ cargo update -p {}", node.key));
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                results.push(format!(
                    "✗ cargo update -p {}: {}",
                    node.key,
                    stderr.lines().next().unwrap_or("unknown")
                ));
            }
        }
    }

    Ok(results)
}

/// Git push order for all repos.
pub fn execute_push(
    matrix: &MatrixFile,
    _mode: &str,
    dry_run: bool,
) -> anyhow::Result<Vec<String>> {
    let order = topo_push_order(matrix);
    let mut results = Vec::new();

    for node in &order {
        if let Some(entry) = matrix.crates.get(&node.key)
            && let Some(_git) = &entry.git
        {
            // Extract local path to determine git repo location
            let local_path = entry
                .local
                .clone()
                .or_else(|| entry.zk_local.clone())
                .unwrap_or_default();

            if dry_run {
                results.push(format!(
                    "[dry-run] cd {} && git push (depth {})",
                    local_path, node.depth
                ));
            } else if !local_path.is_empty() {
                let output = std::process::Command::new("git")
                    .args(["-C", &local_path, "push"])
                    .output()?;
                if output.status.success() {
                    results.push(format!("✓ git push {}", node.key));
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    results.push(format!(
                        "✗ git push {}: {}",
                        node.key,
                        stderr.lines().next().unwrap_or("unknown")
                    ));
                }
            }
        }
    }

    Ok(results)
}
