//! Observe backends for authenticity compare (fixture / snapshot / live).
//!
//! **Important:** FixtureBackend does **not** invert `known_hashes` into "observations"
//! by default — that closed loop created false confidence (Agent C F1/F2). Use explicit
//! `denom_traces` maps or `SnapshotBackend` loaded from external dumps. Optional
//! `invert_known_hashes` remains only for self-tests that need invertibility checks.

use crate::ibc::diff::{DiffReport, DiffSeverity};
use crate::ibc::hash::compute_ibc_denom_hash;
use crate::ibc::predict::PredictedWorld;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Observed denom trace on a chain.
#[derive(Debug, Clone)]
pub struct ObservedDenomTrace {
    pub path: String,
    pub base: String,
}

/// Observed bank balance for a denom.
#[derive(Debug, Clone)]
pub struct ObservedBalance {
    pub denom: String,
    pub amount: String,
}

/// Observed channel between two chains (simplified).
#[derive(Debug, Clone)]
pub struct ObservedChannel {
    pub chain_a: String,
    pub channel_a: String,
    pub chain_b: String,
    pub channel_b: String,
    pub preferred: bool,
    pub status: String,
}

/// Backend that can observe on-chain (or fixture) authenticity facts.
pub trait ObserveBackend {
    fn observe_denom_trace(
        &self,
        chain: &str,
        ibc_denom: &str,
    ) -> Result<Option<ObservedDenomTrace>, String>;

    fn observe_channels(
        &self,
        chain_a: &str,
        chain_b: &str,
    ) -> Result<Vec<ObservedChannel>, String>;

    /// Optional balance observation (Error-capable when used with compare balances).
    fn observe_balance(
        &self,
        _chain: &str,
        _address: &str,
        _denom: &str,
    ) -> Result<Option<ObservedBalance>, String> {
        Ok(None)
    }
}

/// Options for compare_predict_observe.
#[derive(Debug, Clone)]
pub struct CompareOptions {
    /// Treat missing observations for preferred routes as Error (default false = Info).
    pub unobserved_as_error: bool,
    /// Only compare preferred routes (default true).
    pub preferred_only: bool,
}

impl Default for CompareOptions {
    fn default() -> Self {
        Self {
            unobserved_as_error: false,
            preferred_only: true,
        }
    }
}

/// Fixture-backed observer with **explicit** denom_traces only (no hash invert by default).
#[derive(Debug, Clone)]
pub struct FixtureBackend {
    /// path → ibc/HASH (metadata / pin table — not automatic observation)
    pub known_hashes: HashMap<String, String>,
    /// chain → denom → ObservedDenomTrace (authoritative observations)
    pub denom_traces: HashMap<String, HashMap<String, ObservedDenomTrace>>,
    /// chain → address → denom → balance
    pub balances: HashMap<String, HashMap<String, HashMap<String, ObservedBalance>>>,
    /// When true, invert known_hashes as observations (test-only; default false).
    pub invert_known_hashes: bool,
    pub root: PathBuf,
}

impl FixtureBackend {
    pub fn from_known_hashes(known_hashes: HashMap<String, String>) -> Self {
        Self {
            known_hashes,
            denom_traces: HashMap::new(),
            balances: HashMap::new(),
            invert_known_hashes: false,
            root: PathBuf::new(),
        }
    }

    /// Build from known_hashes **and** explicit path→denom map as observations on `chain`.
    pub fn with_explicit_traces(
        known_hashes: HashMap<String, String>,
        chain: &str,
        path_to_denom: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        let mut denom_traces = HashMap::new();
        let mut chain_map = HashMap::new();
        for (path, denom) in path_to_denom {
            let base = path.rsplit('/').next().unwrap_or("").to_string();
            chain_map.insert(denom, ObservedDenomTrace { path, base });
        }
        denom_traces.insert(chain.to_string(), chain_map);
        Self {
            known_hashes,
            denom_traces,
            balances: HashMap::new(),
            invert_known_hashes: false,
            root: PathBuf::new(),
        }
    }

    pub fn load_from_golden_dir(root: impl AsRef<Path>) -> Result<Self, String> {
        let root = root.as_ref().to_path_buf();
        let path = root.join("known_hashes.json");
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let value: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("parse known_hashes: {e}"))?;
        let known_hashes = crate::ibc::fixtures::parse_known_hashes_value(&value)?;

        // Optional external observations: denom_traces.json
        // { "osmosis": { "ibc/ABC...": { "path": "transfer/...", "base": "uakt" } } }
        let mut denom_traces = HashMap::new();
        let traces_path = root.join("denom_traces.json");
        if traces_path.exists() {
            let t = std::fs::read_to_string(&traces_path)
                .map_err(|e| format!("read {}: {e}", traces_path.display()))?;
            let v: serde_json::Value =
                serde_json::from_str(&t).map_err(|e| format!("parse denom_traces: {e}"))?;
            if let Some(obj) = v.as_object() {
                for (chain, entries) in obj {
                    if let Some(map) = entries.as_object() {
                        let mut inner = HashMap::new();
                        for (denom, meta) in map {
                            let path = meta["path"]
                                .as_str()
                                .ok_or_else(|| format!("missing path for {chain}/{denom}"))?
                                .to_string();
                            let base = meta["base"]
                                .as_str()
                                .unwrap_or_else(|| path.rsplit('/').next().unwrap_or(""))
                                .to_string();
                            inner.insert(denom.clone(), ObservedDenomTrace { path, base });
                        }
                        denom_traces.insert(chain.clone(), inner);
                    }
                }
            }
        }

        Ok(Self {
            known_hashes,
            denom_traces,
            balances: HashMap::new(),
            invert_known_hashes: false,
            root,
        })
    }
}

impl ObserveBackend for FixtureBackend {
    fn observe_denom_trace(
        &self,
        chain: &str,
        ibc_denom: &str,
    ) -> Result<Option<ObservedDenomTrace>, String> {
        if let Some(by_chain) = self.denom_traces.get(chain) {
            if let Some(t) = by_chain.get(ibc_denom) {
                return Ok(Some(t.clone()));
            }
        }
        if self.invert_known_hashes {
            for (path, hash) in &self.known_hashes {
                if hash == ibc_denom {
                    let base = path.rsplit('/').next().unwrap_or("").to_string();
                    return Ok(Some(ObservedDenomTrace {
                        path: path.clone(),
                        base,
                    }));
                }
            }
        }
        Ok(None)
    }

    fn observe_channels(
        &self,
        _chain_a: &str,
        _chain_b: &str,
    ) -> Result<Vec<ObservedChannel>, String> {
        Ok(Vec::new())
    }

    fn observe_balance(
        &self,
        chain: &str,
        address: &str,
        denom: &str,
    ) -> Result<Option<ObservedBalance>, String> {
        Ok(self
            .balances
            .get(chain)
            .and_then(|a| a.get(address))
            .and_then(|d| d.get(denom))
            .cloned())
    }
}

/// Snapshot dump observer: loads a JSON file produced by harness or live capture.
///
/// Expected shape:
/// ```json
/// {
///   "denom_traces": { "chain": { "ibc/...": { "path": "...", "base": "..." } } },
///   "balances": { "chain": { "addr": { "ibc/...": "1000" } } }
/// }
/// ```
#[derive(Debug, Clone)]
pub struct SnapshotBackend {
    inner: FixtureBackend,
}

impl SnapshotBackend {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let v: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("parse snapshot: {e}"))?;
        let mut denom_traces = HashMap::new();
        if let Some(obj) = v["denom_traces"].as_object() {
            for (chain, entries) in obj {
                if let Some(map) = entries.as_object() {
                    let mut inner = HashMap::new();
                    for (denom, meta) in map {
                        let path = meta["path"].as_str().unwrap_or("").to_string();
                        let base = meta["base"]
                            .as_str()
                            .unwrap_or_else(|| path.rsplit('/').next().unwrap_or(""))
                            .to_string();
                        if !path.is_empty() {
                            inner.insert(denom.clone(), ObservedDenomTrace { path, base });
                        }
                    }
                    denom_traces.insert(chain.clone(), inner);
                }
            }
        }
        let mut balances = HashMap::new();
        if let Some(obj) = v["balances"].as_object() {
            for (chain, addrs) in obj {
                if let Some(am) = addrs.as_object() {
                    let mut a_map = HashMap::new();
                    for (addr, denoms) in am {
                        if let Some(dm) = denoms.as_object() {
                            let mut d_map = HashMap::new();
                            for (denom, amount) in dm {
                                let amount = amount
                                    .as_str()
                                    .map(|s| s.to_string())
                                    .or_else(|| amount.as_u64().map(|n| n.to_string()))
                                    .unwrap_or_default();
                                d_map.insert(
                                    denom.clone(),
                                    ObservedBalance {
                                        denom: denom.clone(),
                                        amount,
                                    },
                                );
                            }
                            a_map.insert(addr.clone(), d_map);
                        }
                    }
                    balances.insert(chain.clone(), a_map);
                }
            }
        }
        Ok(Self {
            inner: FixtureBackend {
                known_hashes: HashMap::new(),
                denom_traces,
                balances,
                invert_known_hashes: false,
                root: path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf(),
            },
        })
    }
}

impl ObserveBackend for SnapshotBackend {
    fn observe_denom_trace(
        &self,
        chain: &str,
        ibc_denom: &str,
    ) -> Result<Option<ObservedDenomTrace>, String> {
        self.inner.observe_denom_trace(chain, ibc_denom)
    }

    fn observe_channels(
        &self,
        chain_a: &str,
        chain_b: &str,
    ) -> Result<Vec<ObservedChannel>, String> {
        self.inner.observe_channels(chain_a, chain_b)
    }

    fn observe_balance(
        &self,
        chain: &str,
        address: &str,
        denom: &str,
    ) -> Result<Option<ObservedBalance>, String> {
        self.inner.observe_balance(chain, address, denom)
    }
}

/// Compare predicted routes against an observe backend (hash + path binding).
pub fn compare_predict_observe(world: &PredictedWorld, observe: &dyn ObserveBackend) -> DiffReport {
    compare_predict_observe_with(world, observe, &CompareOptions::default())
}

pub fn compare_predict_observe_with(
    world: &PredictedWorld,
    observe: &dyn ObserveBackend,
    opts: &CompareOptions,
) -> DiffReport {
    let mut report = DiffReport::new();

    for (dest, routes) in &world.routes.routes {
        for r in routes {
            if opts.preferred_only && !r.preferred {
                continue;
            }
            match observe.observe_denom_trace(dest, &r.dest_denom) {
                Ok(Some(obs)) => {
                    if obs.path != r.trace_path {
                        report.push_at(
                            DiffSeverity::Error,
                            "path_mismatch",
                            format!("predicted path {} != observed {}", r.trace_path, obs.path),
                            format!("{}/{}", dest, r.dest_denom),
                        );
                    }
                    let expected = compute_ibc_denom_hash(&obs.path);
                    if expected != r.dest_denom {
                        report.push_at(
                            DiffSeverity::Error,
                            "hash_mismatch",
                            format!("observed path hash {expected} != denom {}", r.dest_denom),
                            format!("{}/{}", dest, r.dest_denom),
                        );
                    }
                }
                Ok(None) => {
                    let sev = if opts.unobserved_as_error {
                        DiffSeverity::Error
                    } else {
                        DiffSeverity::Info
                    };
                    report.push_at(
                        sev,
                        "unobserved",
                        format!("no observation for {}", r.dest_denom),
                        format!("{}/{}", dest, r.symbol),
                    );
                }
                Err(e) => {
                    report.push_at(
                        DiffSeverity::Error,
                        "observe_error",
                        e,
                        format!("{}/{}", dest, r.symbol),
                    );
                }
            }
        }
    }

    report
}

/// Assert a predicted denom has positive (or any) balance on chain.
pub fn compare_balance(
    chain: &str,
    address: &str,
    expected_denom: &str,
    observe: &dyn ObserveBackend,
) -> DiffReport {
    let mut report = DiffReport::new();
    match observe.observe_balance(chain, address, expected_denom) {
        Ok(Some(bal)) => {
            if bal.amount == "0" || bal.amount.is_empty() {
                report.push_at(
                    DiffSeverity::Error,
                    "balance_zero",
                    format!("balance is zero for {expected_denom}"),
                    format!("{chain}/{address}"),
                );
            }
        }
        Ok(None) => {
            report.push_at(
                DiffSeverity::Error,
                "balance_missing",
                format!("no balance observation for {expected_denom}"),
                format!("{chain}/{address}"),
            );
        }
        Err(e) => {
            report.push_at(
                DiffSeverity::Error,
                "observe_error",
                e,
                format!("{chain}/{address}"),
            );
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_backend_does_not_invert_by_default() {
        let mut map = HashMap::new();
        map.insert(
            "transfer/channel-1/uakt".into(),
            "ibc/1480B8FD20AD5FCAE81EA87584D269547DD4D436843C1D20F15E00EB64743EF4".into(),
        );
        let backend = FixtureBackend::from_known_hashes(map);
        let obs = backend
            .observe_denom_trace(
                "osmosis",
                "ibc/1480B8FD20AD5FCAE81EA87584D269547DD4D436843C1D20F15E00EB64743EF4",
            )
            .unwrap();
        assert!(
            obs.is_none(),
            "must not invent observations from known_hashes"
        );
    }

    #[test]
    fn explicit_traces_are_observed() {
        let denom = "ibc/1480B8FD20AD5FCAE81EA87584D269547DD4D436843C1D20F15E00EB64743EF4";
        let backend = FixtureBackend::with_explicit_traces(
            HashMap::new(),
            "osmosis",
            [("transfer/channel-1/uakt".into(), denom.to_string())],
        );
        let obs = backend
            .observe_denom_trace("osmosis", denom)
            .unwrap()
            .unwrap();
        assert_eq!(obs.path, "transfer/channel-1/uakt");
    }

    #[test]
    fn wrong_explicit_path_errors_on_compare() {
        let denom = compute_ibc_denom_hash("transfer/channel-1/uakt");
        let wrong = FixtureBackend::with_explicit_traces(
            HashMap::new(),
            "osmosis",
            [("transfer/channel-99/uakt".into(), denom.clone())],
        );
        let ibc_data = crate::ibc::fixtures::load_golden_ibc_data().unwrap();
        let world =
            PredictedWorld::from_inputs(ibc_data, &crate::ibc::fixtures::golden_chain_assets(), 3);
        let report = compare_predict_observe(&world, &wrong);
        assert!(
            report.errors().any(|e| e.code == "path_mismatch"),
            "wrong fixture path must Error: {:?}",
            report.items
        );
    }
}
