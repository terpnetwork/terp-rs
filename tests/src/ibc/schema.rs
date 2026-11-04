//! Structural validators for chain-registry shaped asset / ibc_data entries.

use serde_json::Value;

/// Validate an asset entry against asset_list.schema.json `$defs/asset` (structural).
pub fn validate_asset_entry(asset: &Value, path: &str) -> Vec<String> {
    let mut errs = Vec::new();

    for field in &[
        "denom_units",
        "type_asset",
        "base",
        "display",
        "name",
        "symbol",
    ] {
        if asset.get(*field).is_none() {
            errs.push(format!("{path}: missing required field '{field}'"));
        }
    }

    if let Some(ta) = asset["type_asset"].as_str() {
        let valid = [
            "sdk.coin",
            "cw20",
            "erc20",
            "ics20",
            "snip20",
            "snip25",
            "bitcoin-like",
            "evm-base",
            "svm-base",
            "substrate",
            "unknown",
        ];
        if !valid.contains(&ta) {
            errs.push(format!("{path}: invalid type_asset '{ta}'"));
        }
    }

    if let Some(units) = asset["denom_units"].as_array() {
        for (i, u) in units.iter().enumerate() {
            if u.get("denom").and_then(|d| d.as_str()).is_none() {
                errs.push(format!("{path}.denom_units[{i}]: missing 'denom'"));
            }
            if u.get("exponent").and_then(|e| e.as_i64()).is_none() {
                errs.push(format!("{path}.denom_units[{i}]: missing 'exponent'"));
            }
        }
    }

    if let Some(traces) = asset["traces"].as_array() {
        let valid_types = [
            "ibc",
            "ibc-cw20",
            "ibc-bridge",
            "bridge",
            "liquid-stake",
            "synthetic",
            "wrapped",
            "additional-mintage",
            "test-mintage",
            "legacy-mintage",
        ];
        for (i, t) in traces.iter().enumerate() {
            if let Some(tt) = t["type"].as_str() {
                if !valid_types.contains(&tt) {
                    errs.push(format!("{path}.traces[{i}]: invalid type '{tt}'"));
                }
                if tt == "ibc" || tt == "ibc-cw20" {
                    let cp = &t["counterparty"];
                    for f in &["chain_name", "base_denom", "channel_id"] {
                        if cp.get(*f).and_then(|v| v.as_str()).is_none() {
                            errs.push(format!("{path}.traces[{i}].counterparty: missing '{f}'"));
                        }
                    }
                    if let Some(ch) = cp["channel_id"].as_str()
                        && !ch.starts_with("channel-")
                    {
                        errs.push(format!(
                            "{path}.traces[{i}].counterparty.channel_id: invalid pattern '{ch}'"
                        ));
                    }
                    let ch = &t["chain"];
                    for f in &["channel_id", "path"] {
                        if ch.get(*f).and_then(|v| v.as_str()).is_none() {
                            errs.push(format!("{path}.traces[{i}].chain: missing '{f}'"));
                        }
                    }
                    if let Some(ch_id) = ch["channel_id"].as_str()
                        && !ch_id.starts_with("channel-")
                    {
                        errs.push(format!(
                            "{path}.traces[{i}].chain.channel_id: invalid pattern '{ch_id}'"
                        ));
                    }
                }
            }
        }
    }

    errs
}

/// Validate ibc_data entry against ibc_data.schema.json (structural).
pub fn validate_ibc_data_entry(entry: &Value, key: &str) -> Vec<String> {
    let mut errs = Vec::new();

    if entry.get("$schema").and_then(|s| s.as_str()).is_none() {
        errs.push(format!("ibc_data.{key}: missing '$schema'"));
    }

    for side in &["chain_1", "chain_2"] {
        let c = match entry.get(*side) {
            Some(v) => v,
            None => {
                errs.push(format!("ibc_data.{key}: missing '{side}'"));
                continue;
            }
        };
        for f in &["chain_name", "chain_id", "client_id", "connection_id"] {
            if c.get(*f).and_then(|v| v.as_str()).is_none() {
                errs.push(format!("ibc_data.{key}.{side}: missing '{f}'"));
            }
        }
    }

    let channels = match entry["channels"].as_array() {
        Some(a) => a,
        None => {
            errs.push(format!("ibc_data.{key}: missing or non-array 'channels'"));
            return errs;
        }
    };

    for (i, ch) in channels.iter().enumerate() {
        let cp = format!("ibc_data.{key}.channels[{i}]");

        for f in &["chain_1", "chain_2", "ordering", "version"] {
            if ch.get(*f).is_none() {
                errs.push(format!("{cp}: missing '{f}'"));
            }
        }

        match ch["ordering"].as_str() {
            Some("ordered") | Some("unordered") => {}
            Some(s) => errs.push(format!(
                "{cp}.ordering: must be 'ordered' or 'unordered', got '{s}'"
            )),
            None => errs.push(format!(
                "{cp}.ordering: missing or invalid type (must be string)"
            )),
        }

        for side in &["chain_1", "chain_2"] {
            let s = &ch[side];
            if s.get("channel_id").and_then(|v| v.as_str()).is_none() {
                errs.push(format!("{cp}.{side}: missing 'channel_id'"));
            }
            if s.get("port_id").and_then(|v| v.as_str()).is_none() {
                errs.push(format!("{cp}.{side}: missing 'port_id'"));
            }
        }

        for side in &["chain_1", "chain_2"] {
            if let Some(cid) = ch[side]["channel_id"].as_str()
                && cid != "*"
                && !cid.starts_with("channel-")
            {
                errs.push(format!("{cp}.{side}.channel_id: invalid pattern '{cid}'"));
            }
        }

        if let Some(tags) = ch.get("tags") {
            if tags.get("preferred").and_then(|p| p.as_bool()).is_none() {
                errs.push(format!("{cp}.tags: missing or non-bool 'preferred'"));
            }
            if let Some(status) = tags.get("status").and_then(|s| s.as_str()) {
                let valid = ["ACTIVE", "INACTIVE", "CLOSED", "PENDING"];
                if !valid.contains(&status) {
                    errs.push(format!("{cp}.tags.status: invalid '{status}'"));
                }
            }
        }
    }

    errs
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_int_ordering() {
        let entry = json!({
            "$schema": "../ibc_data.schema.json",
            "chain_1": { "chain_name": "terp", "chain_id": "x", "client_id": "y", "connection_id": "z" },
            "chain_2": { "chain_name": "osmo", "chain_id": "x", "client_id": "y", "connection_id": "z" },
            "channels": [{
                "chain_1": { "channel_id": "channel-0", "port_id": "transfer" },
                "chain_2": { "channel_id": "channel-1", "port_id": "transfer" },
                "ordering": 1,
                "version": "ics20-1",
            }],
        });
        let errs = validate_ibc_data_entry(&entry, "terp-osmo");
        assert!(
            errs.iter().any(|e| e.contains("ordering")),
            "expected ordering error, got {errs:?}"
        );
    }

    #[test]
    fn accepts_schema_compliant_entry() {
        let entry = json!({
            "$schema": "../ibc_data.schema.json",
            "chain_1": {
                "chain_name": "akash",
                "chain_id": "akashnet-2",
                "client_id": "07-tendermint-210",
                "connection_id": "connection-207",
            },
            "chain_2": {
                "chain_name": "terp",
                "chain_id": "morocco-1",
                "client_id": "07-tendermint-32",
                "connection_id": "connection-11",
            },
            "channels": [{
                "chain_1": { "channel_id": "channel-139", "port_id": "transfer" },
                "chain_2": { "channel_id": "channel-9", "port_id": "transfer" },
                "ordering": "unordered",
                "version": "ics20-1",
                "tags": { "preferred": true, "status": "ACTIVE" },
            }],
        });
        let errs = validate_ibc_data_entry(&entry, "akash-terp");
        assert!(errs.is_empty(), "{errs:?}");
    }
}
