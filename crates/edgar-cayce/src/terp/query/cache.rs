//! Path-keyed TTL cache. Design §3.4.
//!
//! node_info and params: 10 min. Validators and the staking pool: 60 s.
//! A proposal id in a final status: 1 h. Balances and contract queries: 15 s.
//! Everything else, including gov lists: 30 s.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::cite::Cited;
use crate::terp::chain_watch;

const TEN_MIN: Duration = Duration::from_secs(10 * 60);
const ONE_MIN: Duration = Duration::from_secs(60);
const THIRTY: Duration = Duration::from_secs(30);
const ONE_HOUR: Duration = Duration::from_secs(60 * 60);
const FIFTEEN: Duration = Duration::from_secs(15);

#[derive(Debug)]
struct Entry {
    cited: Cited<Value>,
    expires: Instant,
}

#[derive(Debug, Default)]
pub struct Cache {
    inner: Mutex<HashMap<String, Entry>>,
}

impl Cache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, path: &str) -> Option<Cited<Value>> {
        let mut map = lock(&self.inner);
        let expired = map.get(path).is_some_and(|e| Instant::now() >= e.expires);
        if expired {
            map.remove(path);
            return None;
        }
        map.get(path).map(|e| {
            let mut cited = e.cited.clone();
            cited.cached = true;
            cited
        })
    }

    pub fn put(&self, path: &str, cited: &Cited<Value>) {
        let ttl = ttl_for(path, &cited.data);
        let mut stored = cited.clone();
        stored.cached = false;
        lock(&self.inner).insert(
            path.to_string(),
            Entry {
                cited: stored,
                expires: Instant::now() + ttl,
            },
        );
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

pub fn ttl_for(path: &str, body: &Value) -> Duration {
    let path = path.split('?').next().unwrap_or(path);
    if path.contains("node_info") || path.contains("/params") {
        return TEN_MIN;
    }
    if path.contains("/validators") || path.ends_with("/pool") {
        return ONE_MIN;
    }
    if path.contains("/balances/") || path.contains("/smart/") {
        return FIFTEEN;
    }
    if is_proposal_id(path) {
        let status = proposal_status(body).unwrap_or_default();
        return if matches!(status.as_str(), "PASSED" | "REJECTED" | "FAILED") {
            ONE_HOUR
        } else {
            THIRTY
        };
    }
    THIRTY
}

fn is_proposal_id(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/cosmos/gov/v1/proposals/") else {
        return false;
    };
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

fn proposal_status(body: &Value) -> Option<String> {
    let raw = body.pointer("/proposal/status")?;
    let text = raw
        .as_str()
        .map(|s| s.to_string())
        .or_else(|| raw.as_u64().map(|n| n.to_string()))
        .or_else(|| raw.as_i64().map(|n| n.to_string()))?;
    Some(chain_watch::normalize_status(&text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ttls_follow_the_design() {
        let empty = json!({});
        assert_eq!(
            ttl_for("/cosmos/base/tendermint/v1beta1/node_info", &empty),
            TEN_MIN
        );
        assert_eq!(ttl_for("/cosmos/staking/v1beta1/params", &empty), TEN_MIN);
        assert_eq!(
            ttl_for(
                "/cosmos/staking/v1beta1/validators?status=BOND_STATUS_BONDED",
                &empty
            ),
            ONE_MIN
        );
        assert_eq!(ttl_for("/cosmos/staking/v1beta1/pool", &empty), ONE_MIN);
        assert_eq!(
            ttl_for("/cosmos/gov/v1/proposals?pagination.limit=1", &empty),
            THIRTY
        );
        assert_eq!(
            ttl_for("/cosmos/bank/v1beta1/balances/terp1abc", &empty),
            FIFTEEN
        );
        let passed = json!({"proposal": {"status": "PROPOSAL_STATUS_PASSED"}});
        assert_eq!(ttl_for("/cosmos/gov/v1/proposals/62", &passed), ONE_HOUR);
        let voting = json!({"proposal": {"status": "PROPOSAL_STATUS_VOTING_PERIOD"}});
        assert_eq!(ttl_for("/cosmos/gov/v1/proposals/63", &voting), THIRTY);
        assert_eq!(
            ttl_for("/cosmos/gov/v1/proposals/62/tally", &passed),
            THIRTY
        );
        assert_eq!(
            ttl_for("/cosmos/bank/v1beta1/supply/by_denom", &empty),
            THIRTY
        );
    }
}
