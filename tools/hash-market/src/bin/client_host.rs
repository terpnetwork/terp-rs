//! hash-market-client binary.
//!
//! This binary polls an Ethereum node for proof data and submits it to the
//! hash-market-server sidecar. Config is the canonical `hash_market::config::ClientConfig`.

//  TODO: add custody client for signing/handing of signing key

use anyhow::{Context, Result};
use clap::Parser;
use hash_market::client;
use hash_market::client::eth::EthClient;
use hash_market::config::ClientConfig;

#[derive(Parser)]
#[command(
    name = "hash-market-client",
    about = "ETH proof poller for hashmerchant sidecar"
)]
struct Cli {
    /// Path to config file
    #[arg(short, long, default_value = "client.toml")]
    config: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let config_str = std::fs::read_to_string(&cli.config)
        .with_context(|| format!("failed to read config: {}", cli.config))?;
    let config: ClientConfig = toml::from_str(&config_str).context("invalid config")?;

    let eth = EthClient::new(&config.eth_rpc);
    tracing::info!(
        rpc = %config.eth_rpc,
        chain_uid = %config.chain_uid,
        account = %config.account_address,
        "client starting"
    );

    client::run_poll_loop(
        &eth,
        &config.sidecar_url,
        &config.runtime_id,
        &config.chain_uid,
        config.interval_secs,
        &config.storage_keys,
        &config.account_address,
    )
    .await
}
