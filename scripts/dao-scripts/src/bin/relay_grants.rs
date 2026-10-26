//! Authz and feegrant plan for self-relaying a fresh client.
//!
//! Prints one grant per chain. The lists match. Nothing is signed or broadcast.
//! `enable_relay_sessions` is what opens them, and it finishes every chain
//! before a client or packet message is sent.
//!
//! ```sh
//! cargo run -p dao-scripts --bin relay_grants -- --chains 240u-1,osmosis-1
//! ```

use anyhow::{anyhow, Result};
use clap::Parser;
use cw_orch_kms::relay_grants;

#[derive(Parser, Debug)]
#[command(name = "relay_grants")]
struct Cli {
    /// Comma-separated chain ids. Each one receives the same relay grant.
    #[arg(long, default_value = "240u-1,osmosis-1")]
    chains: String,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let ids: Vec<&str> = cli
        .chains
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .collect();
    if ids.len() < 2 {
        return Err(anyhow!(
            "relay grant needs at least two chains, got {}",
            cli.chains
        ));
    }
    let grants = relay_grants(&ids);
    let body = serde_json::json!({
        "broadcast": false,
        "note": "Open these grants on every chain before MsgCreateClient or any packet message.",
        "chains": grants.into_iter().map(|(chain_id, type_urls)| serde_json::json!({
            "chain_id": chain_id,
            "authz": type_urls,
            "feegrant": true,
        })).collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}
