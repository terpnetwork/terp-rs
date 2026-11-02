//! Predetermined ICS-20 denoms for shitstrap accepted-denoms allowlists.
//!
//! Unlike `routes::derive_terp_ibc_denom` (one Terp-side hop prepended to an
//! assetlist trace, one channel per counterparty chain), this takes explicit,
//! live-verified multi-hop channel paths from `public/ibc-accepted/routes.toml`
//! and hashes `transfer/<recv-chan>/.../<base>` per ICS-20 for every variant.
//! Data: public/ibc-accepted/{routes.toml,accepted-denoms.json}.
//! Regenerate: `just scripts-ibc-accepted` (python) — golden-checked in ibc_unit.

use crate::ibc::hash::compute_ibc_denom_hash;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Known denoms observed live on Terp (`/ibc/apps/transfer/v1/denoms/{hash}`, 2026-10-31)
/// and Osmosis (`denom_hashes`). (trace_path, base, expected)
pub const SELF_TEST_VECTORS: &[(&str, &str, &str)] = &[
    ("transfer/channel-9", "uakt", "ibc/ED3433FEFECE0B9EC584F0EBAB31E8AB6A7A187537F3E5B1D8A80998986E7ACF"),
    ("transfer/channel-11/transfer/channel-73", "ubtsg", "ibc/5FFFDD494CE1478FB87727208AE1F94EE80467037291F02AC3BA108CA8868028"),
    ("transfer/channel-11", "factory/osmo1z6r6qdknhgsc0zeracktgpcxf43j6sekq07nw8sxduc9lg0qjjlqfu25e3/alloyed/allBTC", "ibc/9FC418778218616ADEC741B8E402C856EBF2AADAD93A8D7EA322F798480F641C"),
    ("transfer/channel-750", "uusdc", "ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
    ("transfer/channel-79703", "upenumbra", "ibc/0FA9232B262B89E77D1335D54FB1E1F506A92A7E4B51524B400DC69C68D28372"),
];

#[derive(Debug, Deserialize)]
pub struct AcceptedRoutesFile {
    pub dest_chain: String,
    pub assets: BTreeMap<String, AcceptedAsset>,
}

#[derive(Debug, Deserialize)]
pub struct AcceptedAsset {
    pub canonical: String,
    pub origin: String,
    pub base: String,
    pub routes: Vec<AcceptedRoute>,
}

#[derive(Debug, Deserialize)]
pub struct AcceptedRoute {
    pub via: String,
    /// Receiving-side channel first (Terp's channel), origin-side last.
    pub hops: Vec<String>,
    /// live | terp-live | dead | legacy
    pub status: String,
    #[serde(default)]
    pub note: String,
}

pub fn trace_from_hops(hops: &[String]) -> String {
    hops.iter().map(|c| format!("transfer/{c}")).collect::<Vec<_>>().join("/")
}

pub fn ibc_denom_for(trace_path: &str, base: &str) -> String {
    if trace_path.is_empty() {
        base.to_string()
    } else {
        compute_ibc_denom_hash(&format!("{trace_path}/{base}"))
    }
}

/// Returns number of vectors checked; panics-free Err on mismatch.
pub fn self_test() -> Result<usize, String> {
    for (p, b, exp) in SELF_TEST_VECTORS {
        let got = ibc_denom_for(p, b);
        if got != *exp {
            return Err(format!("{p}/{b}: got {got}, want {exp}"));
        }
    }
    Ok(SELF_TEST_VECTORS.len())
}

/// Statuses whose variants belong in a shitstrap `accepted` list.
pub const ACCEPTABLE_STATUSES: &[&str] = &["live", "terp-live"];

pub fn build_accepted_denoms(routes_toml: &str, self_tests_passed: usize) -> Result<Value, String> {
    let file: AcceptedRoutesFile = toml::from_str(routes_toml).map_err(|e| e.to_string())?;
    let mut assets = serde_json::Map::new();
    let mut accepted = serde_json::Map::new();
    for (name, a) in &file.assets {
        let mut variants = Vec::new();
        let mut ok = Vec::new();
        for r in &a.routes {
            let p = trace_from_hops(&r.hops);
            let d = ibc_denom_for(&p, &a.base);
            if ACCEPTABLE_STATUSES.contains(&r.status.as_str()) {
                ok.push(Value::String(d.clone()));
            }
            variants.push(json!({"via": r.via, "trace_path": p, "base_denom": a.base,
                "ibc_denom": d, "status": r.status, "note": r.note}));
        }
        assets.insert(name.clone(), json!({"canonical": a.canonical, "origin": a.origin,
            "base_denom": a.base, "variants": variants}));
        accepted.insert(name.clone(), Value::Array(ok));
    }
    Ok(json!({"chain_id": file.dest_chain, "spec": "ICS-20 sha256(path/base)",
        "self_tests_passed": self_tests_passed, "assets": assets, "shitstrap_accepted": accepted}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_test_vectors_reproduce_live_denoms() {
        assert_eq!(self_test().unwrap(), SELF_TEST_VECTORS.len());
    }

    #[test]
    fn multi_hop_trace_order_is_receiving_side_first() {
        let hops = vec!["channel-16".to_string(), "channel-4".to_string()];
        assert_eq!(trace_from_hops(&hops), "transfer/channel-16/transfer/channel-4");
    }
}
