//! Guard-railed node API access for the RLM loop.
//!
//! The sandbox guest has no network. `node_query(path)` is served **host-side**
//! through this module, which enforces:
//! - GET only, against `TERP_NODE_REST` (operator-configured; the guest can
//!   never choose or influence the base URL).
//! - Strict path allowlist: read-only Cosmos SDK LCD endpoints only
//!   (node info, blocks, bank, staking, gov, auth, distribution, mint,
//!   slashing). No tx submission, no `/txs` POST routes, no abci_query.
//! - Query-string allowlist: `pagination.*`, `proposal_status`, `status`,
//!   and `denom`. (`?height=` is rejected — it forces expensive historical
//!   reads across endpoints. The single exception is `/blocks/{height}`,
//!   an explicit bounded lookup of one historical block, which stays
//!   allowlisted as a path template.)
//! - 10s timeout, 32KB response cap (truncated with a marker, never streamed
//!   unbounded into the reasoning context).
//!
//! Both execution paths funnel here (there is exactly one backend: the
//! Wasmer marker round-trip).

use anyhow::{Context, Result};

/// Base REST endpoint, operator-configured only.
pub fn base_url() -> Option<String> {
    std::env::var("TERP_NODE_REST").ok().filter(|s| !s.trim().is_empty())
}

/// Read-only LCD path prefixes the guest may request.
/// `{param}` segments are matched as a single non-empty path component
/// (alphanumeric plus `-_.`, no slashes, no `..`).
const ALLOWED: &[&str] = &[
    // node / chain state
    "/cosmos/base/tendermint/v1beta1/node_info",
    "/cosmos/base/tendermint/v1beta1/blocks/latest",
    "/cosmos/base/tendermint/v1beta1/blocks/{height}",
    "/cosmos/base/tendermint/v1beta1/validatorsets/latest",
    "/cosmos/base/node/v1beta1/status",
    // bank
    "/cosmos/bank/v1beta1/supply",
    "/cosmos/bank/v1beta1/balances/{addr}",
    "/cosmos/bank/v1beta1/denoms_metadata",
    // staking
    "/cosmos/staking/v1beta1/validators",
    "/cosmos/staking/v1beta1/validators/{valoper}",
    "/cosmos/staking/v1beta1/validators/{valoper}/delegations",
    "/cosmos/staking/v1beta1/delegations/{delegator}",
    "/cosmos/staking/v1beta1/pool",
    "/cosmos/staking/v1beta1/params",
    // gov
    "/cosmos/gov/v1/proposals",
    "/cosmos/gov/v1/proposals/{id}",
    "/cosmos/gov/v1/proposals/{id}/tally",
    "/cosmos/gov/v1/params/{kind}",
    // auth / distribution / mint / slashing
    "/cosmos/auth/v1beta1/accounts/{addr}",
    "/cosmos/distribution/v1beta1/params",
    "/cosmos/distribution/v1beta1/validators/{valoper}/commission",
    "/cosmos/mint/v1beta1/inflation",
    "/cosmos/mint/v1beta1/params",
    "/cosmos/slashing/v1beta1/params",
    "/cosmos/upgrade/v1beta1/applied_plan/{name}",
    "/cosmos/upgrade/v1beta1/current_plan",
    "/cosmos/bank/v1beta1/supply/by_denom",
    // IBC reads. Channel ids look like `channel-0`; ports like `transfer`.
    "/ibc/core/channel/v1/channels",
    "/ibc/core/channel/v1/channels/{channel}/ports/{port}/client_state",
    "/ibc/core/client/v1/client_states",
    "/ibc/core/connection/v1/connections",
];

/// Allowed query-string keys. `?height=` is not among them.
const ALLOWED_QUERY_KEYS: &[&str] = &[
    "pagination.key",
    "pagination.offset",
    "pagination.limit",
    "pagination.count_total",
    "pagination.reverse",
    "proposal_status",
    "status",
    "denom",
];

const MAX_QUERY_LIMIT: u32 = 100;
const RESPONSE_CAP: usize = 32 * 1024;
/// Hard reject when the server *declares* more than this (chunk loop caps liars).
const MAX_BODY_BYTES: u64 = 256 * 1024;

/// Validate a guest-supplied path (may include `?query`). Returns the
/// normalized path+query to request on success, or a refusal reason.
pub fn validate(path: &str) -> Result<String, String> {
    let path = path.trim();
    if path.is_empty() || !path.starts_with('/') {
        return Err("path must be absolute (start with /)".into());
    }
    if path.len() > 512 {
        return Err("path too long".into());
    }
    let (raw_path, raw_query) = match path.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path, None),
    };
    if raw_path.contains("..") || raw_path.contains("//") || raw_path.contains(' ') {
        return Err("path traversal / malformed path rejected".into());
    }
    if !ALLOWED.iter().any(|pat| match_template(pat, raw_path)) {
        return Err(format!("endpoint not on the read-only allowlist: {raw_path}"));
    }
    let mut query_out = Vec::new();
    if let Some(q) = raw_query {
        for pair in q.split('&') {
            if pair.is_empty() {
                continue;
            }
            let (k, v) = match pair.split_once('=') {
                Some((k, v)) => (k, v),
                None => (pair, ""),
            };
            if !ALLOWED_QUERY_KEYS.contains(&k) {
                return Err(format!("query param not allowed: {k}"));
            }
            if k == "pagination.limit" {
                let n: u32 = v.parse().map_err(|_| "bad pagination.limit".to_string())?;
                if n == 0 || n > MAX_QUERY_LIMIT {
                    return Err(format!("pagination.limit must be 1..={MAX_QUERY_LIMIT}"));
                }
            }
            if k == "denom" {
                if !valid_denom(v) {
                    return Err(format!("bad value for {k}"));
                }
            } else if v.len() > 256 || v.contains('/') || v.contains("..") {
                return Err(format!("bad value for {k}"));
            }
            query_out.push(format!("{k}={v}"));
        }
    }
    Ok(if query_out.is_empty() {
        raw_path.to_string()
    } else {
        format!("{}?{}", raw_path, query_out.join("&"))
    })
}

/// Bank denoms include `ibc/HASH`. Reject traversal. Other query values stay slash-free.
fn valid_denom(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 128
        && !v.contains("..")
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '.' || c == '-' || c == '_')
}

/// Match `/a/{x}/c` templates against a concrete path.
fn match_template(template: &str, path: &str) -> bool {
    let t: Vec<&str> = template.split('/').collect();
    let p: Vec<&str> = path.split('/').collect();
    if t.len() != p.len() {
        return false;
    }
    for (tt, pp) in t.iter().zip(p.iter()) {
        if tt.starts_with('{') && tt.ends_with('}') {
            if pp.is_empty() || pp.len() > 128 {
                return false;
            }
            if !pp.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.') {
                return false;
            }
        } else if tt != pp {
            return false;
        }
    }
    true
}

/// Validated, capped node query. Errors are returned as `Error:` strings so
/// the reasoning loop can observe refusals and adjust (never panics).
///
/// The response cap is enforced *while reading* (chunk loop stops past the
/// cap) plus a content-length pre-check, so a hostile endpoint cannot force
/// unbounded RAM before truncation.
pub async fn node_query(client: &reqwest::Client, path: &str) -> Result<String> {
    let base = base_url().ok_or_else(|| anyhow::anyhow!("node API not configured (TERP_NODE_REST)"))?;
    let clean = validate(path).map_err(|r| anyhow::anyhow!("refused: {r}"))?;
    let url = format!("{}{}", base.trim_end_matches('/'), clean);
    let mut resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .with_context(|| format!("node request failed: {clean}"))?
        .error_for_status()
        .with_context(|| format!("node returned error for: {clean}"))?;
    if let Some(len) = resp.content_length() {
        if len > MAX_BODY_BYTES {
            anyhow::bail!("refused: response too large ({len} bytes)");
        }
    }
    let mut buf: Vec<u8> = Vec::with_capacity(8192);
    let mut truncated = false;
    while let Some(chunk) = resp.chunk().await.context("reading node response")? {
        if buf.len() + chunk.len() > RESPONSE_CAP {
            let room = RESPONSE_CAP.saturating_sub(buf.len());
            buf.extend_from_slice(&chunk[..room.min(chunk.len())]);
            truncated = true;
            break;
        }
        buf.extend_from_slice(&chunk);
    }
    let mut text = String::from_utf8_lossy(&buf).into_owned();
    // Pretty-print JSON when possible so the loop reads structure, not a blob.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
        text = serde_json::to_string_pretty(&v).unwrap_or(text);
    }
    if truncated || text.len() > RESPONSE_CAP {
        text.truncate(crate::util::floor_char_boundary(&text, RESPONSE_CAP));
        text.push_str("\n[... response truncated at 32KB — narrow with pagination ...]");
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_read_only_core() {
        assert!(validate("/cosmos/base/tendermint/v1beta1/node_info").is_ok());
        assert!(validate("/cosmos/gov/v1/proposals?pagination.limit=20").is_ok());
        assert!(validate("/cosmos/bank/v1beta1/balances/terp1abc_xyz-9").is_ok());
        assert!(validate("/cosmos/staking/v1beta1/validators/terpvaloper1abc").is_ok());
    }

    #[test]
    fn refuses_writes_and_unknown() {
        assert!(validate("/cosmos/tx/v1beta1/txs").is_err());
        assert!(validate("/cosmos/auth/v1beta1/accounts").is_err()); // needs {addr}
        assert!(validate("/../../../etc/passwd").is_err());
        assert!(validate("//cosmos/bank/v1beta1/supply").is_err());
        assert!(validate("https://evil.com/x").is_err());
        assert!(validate("/cosmos/bank/v1beta1/supply?height=5").is_err());
        assert!(validate("/cosmos/gov/v1/proposals?pagination.limit=1000").is_err());
        assert!(validate("/cosmos/gov/v1/proposals?foo=bar").is_err());
        assert!(validate("/cosmos/bank/v1beta1/balances/a/b").is_err());
        assert!(validate("/ibc/core/../../etc/passwd").is_err());
        assert!(validate("/cosmos/bank/v1beta1/supply/by_denom?denom=../secret").is_err());
    }

    #[test]
    fn allows_chain_query_extensions() {
        assert!(
            validate("/cosmos/gov/v1/proposals?pagination.limit=1&pagination.reverse=true").is_ok()
        );
        assert!(validate("/cosmos/gov/v1/proposals?proposal_status=2&pagination.limit=5").is_ok());
        assert!(validate("/cosmos/upgrade/v1beta1/current_plan").is_ok());
        assert!(validate("/cosmos/upgrade/v1beta1/applied_plan/v6.3").is_ok());
        assert!(validate(
            "/cosmos/staking/v1beta1/validators?status=BOND_STATUS_BONDED&pagination.limit=2"
        )
        .is_ok());
        assert!(validate("/cosmos/bank/v1beta1/supply/by_denom?denom=uthiol").is_ok());
        assert!(validate("/cosmos/bank/v1beta1/supply/by_denom?denom=ibc/ABCDEF").is_ok());
        assert!(validate("/ibc/core/channel/v1/channels?pagination.limit=100").is_ok());
        assert!(
            validate("/ibc/core/channel/v1/channels/channel-0/ports/transfer/client_state").is_ok()
        );
        assert!(validate("/ibc/core/client/v1/client_states").is_ok());
        assert!(validate("/ibc/core/connection/v1/connections").is_ok());
    }
}
