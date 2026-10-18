//! hash-market-relay binary.
//!
//! Nostr relay for HashMerchant root events. Connects to a configured
//! Terp Network chain via WebSocket, watches for `hashmerchant_root_confirmed`
//! events, and publishes them as kind:30070 Nostr events.
//!
//! Also accepts webhooks for external root injection and serves as a
//! NIP-77 negentropy sync endpoint.
//!
//! The default bind is `127.0.0.1:9444`. Startup refuses `0.0.0.0`,
//! `[::]`, and any other non-loopback host unless `allow_public_bind`
//! or `HASH_MARKET_ALLOW_PUBLIC_BIND` is set.

use anyhow::{Context, Result};
use clap::Parser;
use hash_market::client::nostr::{discovery, HashMerchantRelayInfo};
use hash_market::client::{MinioIpfsClient, RootConfirmation};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Parser)]
#[command(
    name = "hash-market-relay",
    about = "Nostr relay for hashmerchant root events"
)]
struct Cli {
    #[arg(short, long, default_value = "relay.toml")]
    config: String,
}

#[derive(Debug, Deserialize, Clone)]
struct Config {
    /// Advertised listen address. Default `127.0.0.1:9444`.
    /// A non-loopback host, including `0.0.0.0` and `[::]`, refuses to
    /// start unless `allow_public_bind` or `HASH_MARKET_ALLOW_PUBLIC_BIND`.
    #[serde(default = "default_bind")]
    bind: String,
    /// Opt-in for a non-loopback `bind`. Logged when startup allows it.
    #[serde(default)]
    allow_public_bind: bool,
    #[serde(default)]
    nostr_relays: Vec<String>,
    #[serde(default)]
    chain_ws_url: Option<String>,
    #[serde(default)]
    webhook_urls: Vec<String>,
    #[serde(default)]
    kubo_api_url: Option<String>,
    #[serde(default)]
    announce_name: String,
    #[serde(default)]
    announce_description: String,
}

fn default_bind() -> String {
    "127.0.0.1:9444".to_string()
}

/// Same opt-in words as the hash-market server.
fn env_flag_is_on(flag: Option<&str>) -> bool {
    let Some(flag) = flag.map(str::trim) else {
        return false;
    };
    flag == "1" || flag.eq_ignore_ascii_case("true") || flag.eq_ignore_ascii_case("yes")
}

/// Apply the env opt-in, then refuse a non-loopback bind.
/// `Ok(true)` means a public bind was explicitly allowed.
fn apply_public_bind(config: &mut Config, env_flag: Option<&str>) -> Result<bool, String> {
    if env_flag_is_on(env_flag) {
        config.allow_public_bind = true;
    }
    hash_market::tree_operator::inspect_bind(&config.bind, config.allow_public_bind)
}

impl Config {
    fn relay_info(&self) -> HashMerchantRelayInfo {
        HashMerchantRelayInfo {
            relay_url: format!("wss://{}", self.bind.replace("0.0.0.0", "localhost")),
            chains: vec![],
            algos: vec!["keccak256".to_string(), "sha256".to_string()],
            ws_url: Some(format!("ws://{}", self.bind)),
            webhook_url: self.webhook_urls.first().cloned(),
            bud_cid: None,
            pubkey: "0".repeat(64), // placeholder
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let config_str = std::fs::read_to_string(&cli.config)
        .with_context(|| format!("failed to read config: {}", cli.config))?;
    let mut config: Config = toml::from_str(&config_str).context("invalid config")?;
    let public_bind = apply_public_bind(
        &mut config,
        std::env::var("HASH_MARKET_ALLOW_PUBLIC_BIND")
            .ok()
            .as_deref(),
    )
    .map_err(anyhow::Error::msg)?;
    if public_bind {
        tracing::warn!(
            bind = %config.bind,
            "non-loopback bind allowed by allow_public_bind / HASH_MARKET_ALLOW_PUBLIC_BIND"
        );
    }

    tracing::info!("hash-market-relay starting on {}", config.bind);

    // Build clients
    let pinner = if let Some(kubo_url) = &config.kubo_api_url {
        Some(MinioIpfsClient::with_kubo_url(kubo_url))
    } else {
        Some(MinioIpfsClient::new())
    };

    let webhook_client = if !config.webhook_urls.is_empty() {
        Some(Arc::new(HttpWebhookRelay::new(config.webhook_urls.clone())))
    } else {
        None
    };

    // Announce NIP-87 discovery event
    tracing::info!("announcing NIP-87 discovery event for hashmerchant relay");
    let relay_info = config.relay_info();
    let _discovery_tags = discovery::build_discovery_tags(&relay_info);

    // Main loop: if a chain WS URL is configured, watch for events
    if let Some(ws_url) = &config.chain_ws_url {
        tracing::info!("watching chain websocket: {ws_url}");
        watch_chain_ws(
            ws_url.clone(),
            &config.nostr_relays,
            pinner.as_ref(),
            webhook_client.as_ref(),
        )
        .await?;
    } else {
        tracing::warn!("no chain_ws_url configured — running in webhook-only mode");
        // In webhook-only mode, just serve HTTP for webhook receipt
        // For now, log and wait
        tokio::signal::ctrl_c().await?;
    }

    Ok(())
}

/// Watch the Terp chain WebSocket for hashmerchant_root_confirmed events.
async fn watch_chain_ws(
    ws_url: String,
    _nostr_relays: &[String],
    _pinner: Option<&MinioIpfsClient>,
    _webhook: Option<&Arc<HttpWebhookRelay>>,
) -> Result<()> {
    tracing::info!("chain WebSocket watcher (stub): {ws_url}");
    // Placeholder — actual WebSocket event parsing will go here once
    // the chain watcher transport is wired in. For now, block on signal.
    tokio::signal::ctrl_c().await?;
    Ok(())
}

/// Relay client that POSTs to configured webhooks and logs roots.
struct HttpWebhookRelay {
    urls: Vec<String>,
    client: reqwest::Client,
}

impl HttpWebhookRelay {
    fn new(urls: Vec<String>) -> Self {
        Self {
            urls,
            client: reqwest::Client::new(),
        }
    }

    async fn send(&self, confirmation: &RootConfirmation) {
        let body = serde_json::to_value(confirmation).unwrap_or_default();
        for url in &self.urls {
            if let Ok(resp) = self.client.post(url).json(&body).send().await {
                tracing::info!(%url, status = %resp.status(), "webhook sent");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_public_bind, Config};

    fn parse(text: &str) -> Config {
        toml::from_str(text).expect("config")
    }

    #[test]
    fn default_bind_is_loopback() {
        let mut cfg = parse("");
        assert_eq!(cfg.bind, "127.0.0.1:9444");
        assert!(!cfg.allow_public_bind);
        assert!(!apply_public_bind(&mut cfg, None).unwrap());
        let info = cfg.relay_info();
        assert_eq!(info.relay_url, "wss://127.0.0.1:9444");
        assert_eq!(info.ws_url.as_deref(), Some("ws://127.0.0.1:9444"));
        assert!(!info.relay_url.contains("0.0.0.0"));
    }

    #[test]
    fn public_bind_needs_an_explicit_override() {
        let mut any = parse("bind = \"0.0.0.0:9444\"\n");
        let err = apply_public_bind(&mut any, None).unwrap_err();
        assert!(err.contains("refusing non-loopback bind"));
        assert!(err.contains("0.0.0.0:9444"));
        assert!(!any.allow_public_bind);

        let mut v6 = parse("bind = \"[::]:9444\"\n");
        assert!(apply_public_bind(&mut v6, Some("0")).is_err());

        let mut lan = parse("bind = \"10.0.0.8:9444\"\n");
        assert!(apply_public_bind(&mut lan, Some("false")).is_err());

        let mut opted = parse("bind = \"0.0.0.0:9444\"\nallow_public_bind = true\n");
        assert!(apply_public_bind(&mut opted, None).unwrap());

        let mut from_env = parse("bind = \"0.0.0.0:9444\"\n");
        assert!(apply_public_bind(&mut from_env, Some("1")).unwrap());
        assert!(from_env.allow_public_bind);

        let mut yes = parse("bind = \"[::]:9444\"\n");
        assert!(apply_public_bind(&mut yes, Some(" yes ")).unwrap());

        let mut loopback = parse("bind = \"[::1]:9444\"\n");
        assert!(!apply_public_bind(&mut loopback, None).unwrap());
        let mut local = parse("bind = \"localhost:9444\"\n");
        assert!(!apply_public_bind(&mut local, Some("1")).unwrap());
    }
}
