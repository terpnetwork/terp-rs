//! Frontend server management: config patching + dev server spawning.
//! programmable pipeline of frontend tuning. register frontend pipeline actions as needed from unified config file.
//! default front ends: terp.network static pages, terp-docs documentation page, dao-dao-ui dao page

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

/// Root of the website repo.
fn website_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Root of the terp-docs repo (if it exists).
fn docs_root() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("websites/terp-docs")
}

/// Chain endpoint info for config patching.
pub struct ChainEndpointConfig {
    pub chain_id: String,
    pub rpc_url: String,
    pub grpc_url: String,
}

/// Patch `public/config.json` with deployed contract addresses and chain endpoints.
///
/// Updates the chain entry matching `endpoints.chain_id`. Creates the entry if missing.
pub fn patch_config(
    endpoints: &ChainEndpointConfig,
    addresses: &HashMap<String, String>,
) -> anyhow::Result<()> {
    let config_path = website_root().join("public/config.json");
    let raw = std::fs::read_to_string(&config_path)?;
    let mut config: serde_json::Value = serde_json::from_str(&raw)?;

    let chains = config
        .get_mut("chains")
        .and_then(|c| c.as_object_mut())
        .ok_or_else(|| anyhow::anyhow!("config.json missing 'chains' object"))?;

    // Get or create chain entry
    let chain_id = &endpoints.chain_id;
    if !chains.contains_key(chain_id) {
        chains.insert(chain_id.clone(), serde_json::json!({}));
    }
    let entry = chains.get_mut(chain_id).unwrap();

    // Patch chain-level fields
    entry["chainId"] = serde_json::json!(chain_id);
    entry["chainName"] = serde_json::json!("Local Terp");
    // RPC goes through serve.py proxy to avoid CORS
    entry["rpc"] = serde_json::json!("http://localhost:3000/rpc");
    // REST: extract port from grpc URL, guess REST is on the same host
    entry["rest"] = serde_json::json!(format!(
        "http://localhost:{}",
        extract_port(&endpoints.grpc_url).unwrap_or(1317) - 1 // REST is typically gRPC-1
    ));
    entry["grpc"] = serde_json::json!(&endpoints.grpc_url);

    // Patch contracts
    if entry.get("contracts").is_none() {
        entry["contracts"] = serde_json::json!({});
    }
    if let Some(contracts) = entry.get_mut("contracts").and_then(|c| c.as_object_mut()) {
        for (key, addr) in addresses {
            contracts.insert(key.clone(), serde_json::json!(addr));
        }
    }

    let output = serde_json::to_string_pretty(&config)?;
    std::fs::write(&config_path, output)?;
    println!("Patched config.json for chain {}", chain_id);

    Ok(())
}

/// Start the website dev server (serve.py) on port 3000.
///
/// Sets `CHAIN_RPC` env var so the RPC proxy points to the actual chain RPC port.
pub fn start_website_server(chain_rpc_url: &str) -> anyhow::Result<Child> {
    let serve_py = website_root().join("scripts/serve.py");
    println!("Starting website dev server on http://localhost:3000");
    println!("  RPC proxy -> {}", chain_rpc_url);

    let child = Command::new("python3")
        .arg(&serve_py)
        .env("CHAIN_RPC", chain_rpc_url)
        .env("WEBSITE_PORT", "3000")
        .spawn()?;

    Ok(child)
}

/// Start the terp-docs dev server (pnpm dev) on port 3001.
///
/// Returns None if terp-docs directory doesn't exist or pnpm isn't available.
pub fn start_docs_server(addresses: &HashMap<String, String>) -> anyhow::Result<Option<Child>> {
    let docs = docs_root();
    if !docs.exists() {
        println!("terp-docs not found at {:?} — skipping", docs);
        return Ok(None);
    }

    // Check pnpm is available
    if Command::new("pnpm").arg("--version").output().is_err() {
        println!("pnpm not found — skipping terp-docs");
        return Ok(None);
    }

    // Write .env.local
    write_docs_env(addresses)?;

    println!("Starting terp-docs on http://localhost:3001");

    let child = Command::new("pnpm")
        .args(["dev", "--port", "3001"])
        .current_dir(&docs)
        .spawn()?;

    Ok(Some(child))
}

/// Write .env.local for terp-docs Next.js app with deployed contract addresses.
fn write_docs_env(addresses: &HashMap<String, String>) -> anyhow::Result<()> {
    let docs = docs_root();
    if !docs.exists() {
        return Ok(());
    }

    let env_path = docs.join(".env.local");

    // Read existing .env.local if it exists, or start fresh
    let mut content = if env_path.exists() {
        std::fs::read_to_string(&env_path)?
    } else {
        String::new()
    };

    // Update or append NEXT_PUBLIC_CALENDAR
    if let Some(addr) = addresses.get("daoCalendar") {
        let key = "NEXT_PUBLIC_CALENDAR=";
        let line = format!("{}{}", key, addr);

        if content.contains(key) {
            // Replace the existing line: keep lines before, swap in new value, keep lines after
            let mut new_lines = Vec::new();
            for l in content.lines() {
                if l.starts_with(key) {
                    new_lines.push(line.clone());
                } else {
                    new_lines.push(l.to_string());
                }
            }
            content = new_lines.join("\n");
        } else {
            // Append new line
            if !content.is_empty() && !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str(&line);
            content.push('\n');
        }
    }

    std::fs::write(&env_path, content.trim())?;
    println!("Wrote .env.local for terp-docs");

    Ok(())
}

/// Extract port number from a URL like "http://localhost:55004".
fn extract_port(url: &str) -> Option<u16> {
    url.rsplit(':').next()?.trim_end_matches('/').parse().ok()
}

/// Kill child processes gracefully.
pub fn stop_servers(children: &mut Vec<Child>) {
    for child in children.iter_mut() {
        let _ = child.kill();
        let _ = child.wait();
    }
}
