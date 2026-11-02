//! Merge ibc-tools output into cw-orch `state.json` and stamp wasm provenance.
//!
//! Does not call cw-orch APIs. Extra keys (`ibc_registry`, `code_sources`) sit
//! beside `code_ids` / `ibc_data` so DaemonState still reads the usual maps.

use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Logical contract id → (wasm file, source crate relative to terp-rs).
pub const CODE_SOURCE_TABLE: &[(&str, &str, &str)] = &[
    ("oracle-registry", "cw_oracle_registry.wasm", "contracts/hash-merchant/oracle-registry"),
    ("oracle-aggregator", "cw_oracle_aggregator.wasm", "contracts/hash-merchant/oracle-aggregator"),
    ("oracle-twap-probe", "cw_oracle_twap_probe.wasm", "contracts/hash-merchant/oracle-twap-probe"),
    ("marketplace", "revenue_marketplace.wasm", "contracts/revenue/marketplace"),
    ("reserve-auction", "revenue_reserve_auction.wasm", "contracts/revenue/reserve-auction"),
    ("svg-auction", "svg_auction.wasm", "contracts/infuser/svg-auction"),
    ("raffle", "nftfi_raffle.wasm", "contracts/nfts/raffle"),
    ("nft-loan", "nftfi_loan.wasm", "contracts/nfts/nft-loan"),
    ("bounty-escrow", "terp_bounty_escrow.wasm", "contracts/revenue/bounty-escrow"),
    ("passkey", "terp_passkey.wasm", "contracts/smart-accounts/terp-passkey"),
    ("terp-passkey", "terp_passkey.wasm", "contracts/smart-accounts/terp-passkey"),
    ("recovery", "terp_recovery.wasm", "contracts/smart-accounts/terp-recovery"),
    ("terp-recovery", "terp_recovery.wasm", "contracts/smart-accounts/terp-recovery"),
    ("terp-ed25519", "terp_ed25519.wasm", "contracts/smart-accounts/terp-ed25519"),
    ("terp-eth", "terp_eth.wasm", "contracts/smart-accounts/terp-eth"),
    ("cw721-svg", "cw721_svg.wasm", "contracts/infuser/cw721-svg"),
    ("cw721_svg", "cw721_svg.wasm", "contracts/infuser/cw721-svg"),
    ("svg-minter", "cw_svg_minter.wasm", "contracts/infuser/cw721-svg-minter"),
    ("cw-svg-minter", "cw_svg_minter.wasm", "contracts/infuser/cw721-svg-minter"),
    ("cw-infuser", "cw_infuser.wasm", "contracts/infuser/cw-infuser"),
    ("whitelist-mtree", "whitelist_mtree.wasm", "contracts/infuser/whitelist-merkletree"),
];

/// Git HEAD + dirty flag for `dir` (terp-rs checkout).
pub fn git_build_stamp(dir: &Path) -> Value {
    let commit = git_out(dir, &["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let describe = git_out(dir, &["describe", "--always", "--dirty"]);
    let porcelain = git_out(dir, &["status", "--porcelain"]).unwrap_or_default();
    json!({
        "commit": commit.trim(),
        "describe": describe.as_deref().map(str::trim),
        "dirty": !porcelain.trim().is_empty(),
    })
}

fn git_out(dir: &Path, args: &[&str]) -> Option<String> {
    let o = Command::new("git").args(args).current_dir(dir).output().ok()?;
    if !o.status.success() {
        return None;
    }
    String::from_utf8(o.stdout).ok()
}

/// Parse `artifacts/checksums.txt` (`<sha256>  <file>`).
pub fn parse_checksums(text: &str) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(sha) = parts.next() else { continue };
        let Some(file) = parts.next() else { continue };
        m.insert(file.to_string(), sha.to_string());
    }
    m
}

/// One provenance record for a stored wasm.
pub fn code_source_entry(
    wasm: &str,
    crate_path: &str,
    checksum: Option<&str>,
    git: &Value,
) -> Value {
    json!({
        "wasm": wasm,
        "crate": crate_path,
        "checksum_sha256": checksum,
        "git": git,
    })
}

/// Stamp `code_sources` from the product table + checksums.txt.
pub fn stamp_code_sources(artifacts_dir: &Path, git: &Value) -> Value {
    let sums = fs::read_to_string(artifacts_dir.join("checksums.txt"))
        .ok()
        .map(|t| parse_checksums(&t))
        .unwrap_or_default();
    let mut out = Map::new();
    for (id, wasm, crate_path) in CODE_SOURCE_TABLE {
        let checksum = sums.get(*wasm).map(String::as_str);
        out.insert(
            (*id).to_string(),
            code_source_entry(wasm, crate_path, checksum, git),
        );
    }
    Value::Object(out)
}

/// Counterparty name from `akash-terp.json` stem.
pub fn counterparty_from_ibc_stem(stem: &str) -> Option<String> {
    let parts: Vec<&str> = stem.split('-').collect();
    if parts.len() < 2 {
        return None;
    }
    let names: Vec<&str> = parts.iter().copied().filter(|p| *p != "terp").collect();
    if names.is_empty() {
        return None;
    }
    Some(names.join("-"))
}

/// Load `public/ibc-data/*.json` keyed by counterparty (matches morocco-1.ibc_data).
pub fn load_ibc_data_by_counterparty(public_dir: &Path) -> Map<String, Value> {
    let dir = public_dir.join("ibc-data");
    let mut map = Map::new();
    let Ok(rd) = fs::read_dir(&dir) else {
        return map;
    };
    for ent in rd.flatten() {
        let path = ent.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let Some(cp) = counterparty_from_ibc_stem(stem) else {
            continue;
        };
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                map.insert(cp, v);
            }
        }
    }
    map
}

/// Merge ibc-tools files + code provenance into `state` (mutates morocco-1).
pub fn merge_ibc_and_sources_into_state(
    state: &mut Value,
    public_dir: &Path,
    artifacts_dir: &Path,
    git: &Value,
) {
    let morocco = state
        .as_object_mut()
        .and_then(|o| o.get_mut("morocco-1"))
        .and_then(|v| v.as_object_mut());
    let Some(morocco) = morocco else {
        return;
    };

    let ibc_map = load_ibc_data_by_counterparty(public_dir);
    if !ibc_map.is_empty() {
        morocco.insert("ibc_data".into(), Value::Object(ibc_map));
    }

    let lookup = fs::read_to_string(public_dir.join("ibc_lookup_table.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let meta = fs::read_to_string(public_dir.join("ibc_generation_meta.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());
    morocco.insert(
        "ibc_registry".into(),
        json!({
            "source": "public/ibc-data via terp-ibc rebuild-from-public",
            "generation": meta,
            "lookup": lookup,
        }),
    );

    morocco.insert(
        "code_sources".into(),
        stamp_code_sources(artifacts_dir, git),
    );
}

/// Read, merge, write `public/state.json`.
pub fn update_public_state_file(
    public_dir: &Path,
    artifacts_dir: &Path,
    repo_dir: &Path,
) -> std::io::Result<()> {
    let path = public_dir.join("state.json");
    let text = fs::read_to_string(&path)?;
    let mut state: Value = serde_json::from_str(&text)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let git = git_build_stamp(repo_dir);
    merge_ibc_and_sources_into_state(&mut state, public_dir, artifacts_dir, &git);
    fs::write(&path, serde_json::to_string_pretty(&state).unwrap() + "\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn checksums_and_counterparty() {
        let m = parse_checksums("aa  cw_oracle_registry.wasm\nbb  other.wasm\n");
        assert_eq!(m.get("cw_oracle_registry.wasm").unwrap(), "aa");
        assert_eq!(counterparty_from_ibc_stem("akash-terp").as_deref(), Some("akash"));
        assert_eq!(
            counterparty_from_ibc_stem("osmosis-terp").as_deref(),
            Some("osmosis")
        );
    }

    #[test]
    fn merge_writes_ibc_registry_and_code_sources() {
        let tmp = tempfile::tempdir().unwrap();
        let public = tmp.path().join("public");
        let arts = tmp.path().join("artifacts");
        fs::create_dir_all(public.join("ibc-data")).unwrap();
        fs::create_dir_all(&arts).unwrap();
        fs::write(
            public.join("ibc-data/juno-terp.json"),
            r#"{"chain_1":{"chain_name":"juno"},"chain_2":{"chain_name":"terp"}}"#,
        )
        .unwrap();
        fs::write(
            public.join("ibc_lookup_table.json"),
            r#"{"terp":{"ibc/AA":{"symbol":"JUNO"}}}"#,
        )
        .unwrap();
        fs::write(
            public.join("ibc_generation_meta.json"),
            r#"{"mode":"offline"}"#,
        )
        .unwrap();
        fs::write(
            arts.join("checksums.txt"),
            "34c0eab85a32636a63072a6593794d8c9822ead3e4fba1eda97043a6c7352afb  cw_oracle_registry.wasm\n",
        )
        .unwrap();

        let mut state = json!({"morocco-1": {"code_ids": {"oracle-registry": 88}, "ibc_data": {}}});
        let git = json!({"commit": "deadbeef", "dirty": false});
        merge_ibc_and_sources_into_state(&mut state, &public, &arts, &git);
        assert_eq!(
            state["morocco-1"]["ibc_data"]["juno"]["chain_1"]["chain_name"],
            "juno"
        );
        assert_eq!(
            state["morocco-1"]["ibc_registry"]["lookup"]["terp"]["ibc/AA"]["symbol"],
            "JUNO"
        );
        assert_eq!(
            state["morocco-1"]["code_sources"]["oracle-registry"]["crate"],
            "contracts/hash-merchant/oracle-registry"
        );
        assert_eq!(
            state["morocco-1"]["code_sources"]["oracle-registry"]["checksum_sha256"],
            "34c0eab85a32636a63072a6593794d8c9822ead3e4fba1eda97043a6c7352afb"
        );
        assert_eq!(state["morocco-1"]["code_ids"]["oracle-registry"], 88);
    }
}
