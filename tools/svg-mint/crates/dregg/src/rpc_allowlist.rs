//! Fail-closed hosts for DREGG watch RPCs (privacy: no public SaaS query plane).

const DENY: &[&str] = &[
    "alchemy.com",
    "infura.io",
    "publicnode.com",
    "blastapi.io",
    "ankr.com",
    "llamarpc.com",
    "g.alchemy.com",
];

fn extra_allow() -> Vec<String> {
    std::env::var("DREGG_RPC_ALLOWLIST")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

fn host_of(url: &str) -> Option<String> {
    let u = url.trim();
    if u.is_empty() {
        return None;
    }
    let rest = u
        .split("://")
        .nth(1)
        .unwrap_or(u)
        .split('/')
        .next()
        .unwrap_or("");
    let hostport = rest.rsplit('@').next().unwrap_or(rest);
    let host = hostport
        .trim_start_matches('[')
        .split(']')
        .next()
        .unwrap_or(hostport)
        .split(':')
        .next()
        .unwrap_or(hostport);
    if host.is_empty() {
        None
    } else {
        Some(host.to_lowercase())
    }
}

fn host_allowed(host: &str) -> bool {
    if host == "127.0.0.1" || host == "localhost" || host == "::1" || host == "[::1]" {
        return true;
    }
    if host.starts_with("192.168.10.") {
        return true;
    }
    // Docker / compose service names used by ict-rs lab
    if host == "anvil"
        || host == "bitcoind"
        || host == "geth"
        || host == "reth"
        || host == "base-el"
        || host == "hardhat"
        || host.ends_with(".internal")
        || host == "host.docker.internal"
    {
        return true;
    }
    // RFC1918 docker bridges
    if host.starts_with("10.") || host.starts_with("172.16.") || host.starts_with("172.17.")
        || host.starts_with("172.18.") || host.starts_with("172.19.")
        || host.starts_with("172.20.") || host.starts_with("172.21.")
        || host.starts_with("172.22.") || host.starts_with("172.23.")
        || host.starts_with("172.24.") || host.starts_with("172.25.")
        || host.starts_with("172.26.") || host.starts_with("172.27.")
        || host.starts_with("172.28.") || host.starts_with("172.29.")
        || host.starts_with("172.30.") || host.starts_with("172.31.")
    {
        return true;
    }
    extra_allow().iter().any(|h| h == host || host.ends_with(&format!(".{h}")))
}

/// `true` if this URL is acceptable as ETH_RPC / BASE_RPC / BTC_RPC / compact streamer host.
pub fn rpc_url_is_self_hosted(url: &str) -> bool {
    let lower = url.to_lowercase();
    if DENY.iter().any(|d| lower.contains(d)) {
        return false;
    }
    let Some(host) = host_of(url) else {
        return false;
    };
    if DENY.iter().any(|d| host.contains(d) || host.ends_with(d)) {
        return false;
    }
    host_allowed(&host)
}

pub fn require_self_hosted() -> bool {
    matches!(
        std::env::var("SVG_MINT_REQUIRE_SELF_HOSTED_RPC")
            .unwrap_or_default()
            .as_str(),
        "1" | "true" | "yes"
    )
}

/// Check a named env var. Missing is Ok (watcher no-ops). Present public SaaS is Err.
pub fn check_env_rpc(name: &str) -> Result<(), String> {
    let Ok(url) = std::env::var(name) else {
        return Ok(());
    };
    if url.trim().is_empty() {
        return Ok(());
    }
    if rpc_url_is_self_hosted(&url) {
        Ok(())
    } else {
        Err(format!(
            "{name} is not a self-hosted query plane (house/lab). Refusing public SaaS RPC."
        ))
    }
}

pub fn check_watch_rpcs() -> Result<(), String> {
    check_env_rpc("ETH_RPC")?;
    check_env_rpc("BASE_RPC")?;
    check_env_rpc("BTC_RPC")?;
    // House ZEC JSON-RPC (tip + optional submit). Public tip is *not* a watch plane.
    check_env_rpc("ZCASH_PRIVATE_RPC")?;
    check_env_rpc("ZAKURA_RPC")?;
    if let Ok(s) = std::env::var("ZCASH_DEVTOOL_SERVER") {
        if !s.trim().is_empty() {
            let fake = if s.contains("://") {
                s.clone()
            } else {
                format!("grpc://{s}")
            };
            if !rpc_url_is_self_hosted(&fake) {
                return Err(
                    "ZCASH_DEVTOOL_SERVER is not house/lab CompactTxStreamer (want 127.0.0.1:9067 house or :19067 lab, not public LWD)"
                        .into(),
                );
            }
            if s.contains(":8232") {
                return Err("ZCASH_DEVTOOL_SERVER must not be Zakura JSON-RPC :8232".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_ok() {
        assert!(rpc_url_is_self_hosted("http://127.0.0.1:8545"));
        assert!(rpc_url_is_self_hosted("http://localhost:8332"));
        assert!(rpc_url_is_self_hosted("http://127.0.0.1:8546"));
        assert!(rpc_url_is_self_hosted("grpc://127.0.0.1:9067"));
    }

    #[test]
    fn lab_docker_ok() {
        assert!(rpc_url_is_self_hosted("http://anvil:8545"));
        assert!(rpc_url_is_self_hosted("http://bitcoind:18443"));
        assert!(rpc_url_is_self_hosted("http://192.168.10.103:8545"));
    }

    #[test]
    fn public_saas_denied() {
        assert!(!rpc_url_is_self_hosted(
            "https://eth-mainnet.g.alchemy.com/v2/KEY"
        ));
        assert!(!rpc_url_is_self_hosted(
            "https://mainnet.infura.io/v3/KEY"
        ));
        assert!(!rpc_url_is_self_hosted("https://ethereum.publicnode.com"));
        assert!(!rpc_url_is_self_hosted("https://rpc.ankr.com/eth"));
        assert!(!rpc_url_is_self_hosted("https://eth.llamarpc.com"));
        assert!(!rpc_url_is_self_hosted("https://eth.blastapi.io/x"));
    }
}
