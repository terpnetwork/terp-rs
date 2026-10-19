//! Premine routing tables and Terp-specific denom derivation.

use crate::ibc::graph::{ChannelHop, IBCChannelGraph};
use crate::ibc::hash::{compute_ibc_denom_hash, hop_count_from_trace_path};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Terp's channel info to a counterparty chain.
#[derive(Debug, Clone)]
pub struct TerpChannelInfo {
    pub terp_channel_id: String,
    pub counterparty_channel_id: String,
    pub counterparty_chain_name: String,
}

/// A fully resolved IBC asset route.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IBCAssetRoute {
    pub symbol: String,
    pub origin_chain: String,
    pub origin_denom: String,
    pub dest_chain: String,
    pub dest_denom: String,
    pub trace_path: String,
    pub route: Vec<ChannelHop>,
    pub hop_count: usize,
    pub preferred: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingTableMetadata {
    pub chains: Vec<String>,
    pub total_routes: usize,
    pub generated_at: String,
}

/// The full IBC Asset Routing Table — premined routes for all assets across all chains.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IBCAssetRoutingTable {
    pub routes: HashMap<String, Vec<IBCAssetRoute>>,
    pub metadata: RoutingTableMetadata,
}

impl IBCAssetRoutingTable {
    /// Premine all possible IBC asset routes from the graph and asset lists.
    pub fn premine(
        graph: &IBCChannelGraph,
        chain_assets: &HashMap<String, Vec<serde_json::Value>>,
        max_hops: usize,
    ) -> Self {
        let mut routes: HashMap<String, Vec<IBCAssetRoute>> = HashMap::new();
        let mut chains = HashSet::new();

        for (source_chain, assets) in chain_assets {
            chains.insert(source_chain.clone());

            for asset in assets {
                let symbol = asset["symbol"].as_str().unwrap_or("UNKNOWN");
                let base_denom = asset["base"].as_str().unwrap_or("");

                let is_native = !base_denom.starts_with("ibc/");

                if is_native {
                    for dest_chain in graph.edges.keys() {
                        if dest_chain == source_chain {
                            continue;
                        }

                        let found_routes = graph.find_routes(source_chain, dest_chain, max_hops);

                        for route in found_routes {
                            let (ibc_denom, trace_path) =
                                graph.compute_ibc_denom_for_route(base_denom, &route);

                            let preferred =
                                graph.route_is_preferred(source_chain, dest_chain, &route);
                            let hop_count = hop_count_from_trace_path(&trace_path);

                            let entry = IBCAssetRoute {
                                symbol: symbol.to_string(),
                                origin_chain: source_chain.clone(),
                                origin_denom: base_denom.to_string(),
                                dest_chain: dest_chain.clone(),
                                dest_denom: ibc_denom.clone(),
                                trace_path: trace_path.clone(),
                                route: route.clone(),
                                hop_count,
                                preferred,
                            };

                            routes.entry(dest_chain.clone()).or_default().push(entry);
                        }
                    }
                } else {
                    // IBC asset on this chain — trace further
                    if let Some(traces) = asset["traces"].as_array() {
                        if let Some(ibc_trace) = traces.iter().find(|t| {
                            matches!(
                                t["type"].as_str().unwrap_or(""),
                                "ibc" | "ibc-cw20" | "ibc-bridge"
                            )
                        }) {
                            let cp_chain = ibc_trace["counterparty"]["chain_name"]
                                .as_str()
                                .unwrap_or("");
                            let cp_denom = ibc_trace["counterparty"]["base_denom"]
                                .as_str()
                                .unwrap_or("");
                            let source_trace = ibc_trace["chain"]["path"].as_str().unwrap_or("");

                            for dest_chain in graph.edges.keys() {
                                if dest_chain == source_chain {
                                    continue;
                                }

                                let found_routes =
                                    graph.find_routes(source_chain, dest_chain, max_hops);

                                for route in found_routes {
                                    // Nest transfer prefixes for every hop (dest looking back)
                                    let mut prefix = String::new();
                                    for hop in route.iter().rev() {
                                        prefix.push_str(&format!("transfer/{}/", hop.to_channel));
                                    }
                                    let full_trace = if source_trace.is_empty() {
                                        format!("{prefix}{cp_denom}")
                                    } else {
                                        format!("{prefix}{source_trace}")
                                    };

                                    let ibc_denom = compute_ibc_denom_hash(&full_trace);
                                    // Prefer-direct relative to the physical transfer origin of this leg
                                    let preferred =
                                        graph.route_is_preferred(source_chain, dest_chain, &route);
                                    let hop_count = hop_count_from_trace_path(&full_trace);

                                    let entry = IBCAssetRoute {
                                        symbol: symbol.to_string(),
                                        origin_chain: cp_chain.to_string(),
                                        origin_denom: cp_denom.to_string(),
                                        dest_chain: dest_chain.clone(),
                                        dest_denom: ibc_denom.clone(),
                                        trace_path: full_trace,
                                        route: route.clone(),
                                        hop_count,
                                        preferred,
                                    };

                                    routes.entry(dest_chain.clone()).or_default().push(entry);
                                }
                            }
                        }
                    }
                }
            }
        }

        let total_routes: usize = routes.values().map(|v| v.len()).sum();

        Self {
            routes,
            metadata: RoutingTableMetadata {
                chains: chains.into_iter().collect(),
                total_routes,
                generated_at: String::new(),
            },
        }
    }

    /// Look up all IBC assets available on a specific chain.
    pub fn lookup_by_dest_chain(&self, chain: &str) -> Vec<&IBCAssetRoute> {
        self.routes
            .get(chain)
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// Look up the IBC denom for a specific asset on a specific chain.
    pub fn lookup_ibc_denom(&self, symbol: &str, dest_chain: &str) -> Option<&IBCAssetRoute> {
        self.routes
            .get(dest_chain)?
            .iter()
            .find(|r| r.symbol == symbol && r.preferred)
            .or_else(|| {
                self.routes
                    .get(dest_chain)?
                    .iter()
                    .find(|r| r.symbol == symbol)
            })
    }

    /// Look up the origin asset info from an IBC denom on a chain.
    pub fn reverse_lookup(&self, ibc_denom: &str, chain: &str) -> Option<&IBCAssetRoute> {
        self.routes
            .get(chain)?
            .iter()
            .find(|r| r.dest_denom == ibc_denom)
    }

    /// Export the routing table as JSON.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    /// Export a simplified lookup table (dest_chain → ibc_denom → origin info).
    /// Only preferred routes are included.
    pub fn to_simplified_lookup(&self) -> serde_json::Value {
        let mut lookup = serde_json::Map::new();

        for (dest_chain, routes) in &self.routes {
            let mut chain_lookup = serde_json::Map::new();

            for route in routes {
                if !route.preferred {
                    continue;
                }

                chain_lookup.insert(
                    route.dest_denom.clone(),
                    serde_json::json!({
                        "symbol": route.symbol,
                        "origin_chain": route.origin_chain,
                        "origin_denom": route.origin_denom,
                        "trace_path": route.trace_path,
                        "hop_count": route.hop_count,
                    }),
                );
            }

            lookup.insert(dest_chain.clone(), serde_json::Value::Object(chain_lookup));
        }

        serde_json::Value::Object(lookup)
    }
}

/// Derive the IBC denom hash and trace path for a foreign asset on Terp.
///
/// Logic:
/// 1. If the asset has an IBC trace, extract counterparty chain and base denom
/// 2. If Terp has a direct channel to the counterparty chain → single hop
/// 3. If Terp has NO direct channel but has channel to source chain → route via source
pub fn derive_terp_ibc_denom(
    asset: &serde_json::Value,
    terp_channels: &HashMap<String, TerpChannelInfo>,
) -> Option<(String, String, String)> {
    let traces = asset["traces"].as_array()?;
    let ibc_trace = traces.iter().find(|t| {
        matches!(
            t["type"].as_str().unwrap_or(""),
            "ibc" | "ibc-cw20" | "ibc-bridge"
        )
    })?;

    let counterparty_chain = ibc_trace["counterparty"]["chain_name"].as_str()?;
    let counterparty_base = ibc_trace["counterparty"]["base_denom"].as_str()?;
    let source_trace_path = ibc_trace["chain"]["path"].as_str().unwrap_or("");
    let source_chain = asset["_source_chain"].as_str().unwrap_or("");

    // Try direct channel to counterparty first
    if let Some(info) = terp_channels.get(counterparty_chain) {
        let full_path = format!("transfer/{}/{}", info.terp_channel_id, counterparty_base);
        let hash = compute_ibc_denom_hash(&full_path);
        return Some((hash, full_path, counterparty_chain.to_string()));
    }

    // No direct channel — route through the source chain
    if !source_chain.is_empty() {
        if let Some(info) = terp_channels.get(source_chain) {
            let full_path = if source_trace_path.is_empty() {
                format!("transfer/{}/{}", info.terp_channel_id, counterparty_base)
            } else {
                format!("transfer/{}/{}", info.terp_channel_id, source_trace_path)
            };
            let hash = compute_ibc_denom_hash(&full_path);
            return Some((hash, full_path, counterparty_chain.to_string()));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_terp_channels() -> HashMap<String, TerpChannelInfo> {
        let mut map = HashMap::new();
        map.insert(
            "chain-b".to_string(),
            TerpChannelInfo {
                terp_channel_id: "channel-0".to_string(),
                counterparty_channel_id: "channel-0".to_string(),
                counterparty_chain_name: "chain-b".to_string(),
            },
        );
        map.insert(
            "chain-c".to_string(),
            TerpChannelInfo {
                terp_channel_id: "channel-1".to_string(),
                counterparty_channel_id: "channel-0".to_string(),
                counterparty_chain_name: "chain-c".to_string(),
            },
        );
        map
    }

    #[test]
    fn test_single_hop_derivation() {
        let channels = make_terp_channels();
        let asset = json!({
            "symbol": "UTERP",
            "base": "uterp",
            "traces": [{
                "type": "ibc",
                "counterparty": {
                    "chain_name": "chain-b",
                    "base_denom": "uterp",
                    "channel_id": "channel-0"
                },
                "chain": {
                    "channel_id": "channel-0",
                    "path": "transfer/channel-0/uterp"
                }
            }],
            "_source_chain": "chain-b"
        });

        let (hash, path, cp) = derive_terp_ibc_denom(&asset, &channels).unwrap();
        assert_eq!(path, "transfer/channel-0/uterp");
        assert_eq!(cp, "chain-b");
        assert_eq!(hash, compute_ibc_denom_hash("transfer/channel-0/uterp"));
    }

    #[test]
    fn hop_count_honest_for_premine_routes() {
        let mut graph = IBCChannelGraph::new();
        graph.add_channel("a", "channel-0", "b", "channel-0", true, "ACTIVE".into());
        graph.add_channel("b", "channel-1", "c", "channel-0", true, "ACTIVE".into());

        let mut assets = HashMap::new();
        assets.insert("a".to_string(), vec![json!({"symbol": "A", "base": "ua"})]);

        let table = IBCAssetRoutingTable::premine(&graph, &assets, 3);
        let routes = table.lookup_by_dest_chain("c");
        assert!(!routes.is_empty());
        for r in routes {
            assert_eq!(
                r.hop_count,
                hop_count_from_trace_path(&r.trace_path),
                "hop_count must match transfer/ segments: {:?}",
                r.trace_path
            );
            assert_eq!(
                r.dest_denom,
                compute_ibc_denom_hash(&r.trace_path),
                "hash binding"
            );
        }
    }

    #[test]
    fn prefer_direct_in_premine_lookup() {
        let mut graph = IBCChannelGraph::new();
        graph.add_channel("a", "channel-ab", "b", "channel-ba", true, "ACTIVE".into());
        graph.add_channel("b", "channel-bc", "c", "channel-cb", true, "ACTIVE".into());
        graph.add_channel("a", "channel-ac", "c", "channel-ca", true, "ACTIVE".into());

        let mut assets = HashMap::new();
        assets.insert(
            "a".to_string(),
            vec![json!({"symbol": "TOKEN", "base": "utoken"})],
        );

        let table = IBCAssetRoutingTable::premine(&graph, &assets, 3);
        let preferred = table.lookup_ibc_denom("TOKEN", "c").expect("lookup");
        assert_eq!(
            preferred.hop_count, 1,
            "preferred must be direct single hop"
        );
        assert!(preferred.preferred);
        assert_eq!(preferred.trace_path, "transfer/channel-ca/utoken");

        // Multi-hop still present as non-preferred alternate
        let all = table.lookup_by_dest_chain("c");
        let multi = all.iter().find(|r| r.hop_count == 2);
        assert!(multi.is_some());
        assert!(!multi.unwrap().preferred);
    }

    /// Regression: a world that lies with hop_count=1 but two transfer/ prefixes fails honesty check.
    #[test]
    fn hop_count_lie_is_detectable() {
        let lying = IBCAssetRoute {
            symbol: "X".into(),
            origin_chain: "a".into(),
            origin_denom: "ux".into(),
            dest_chain: "c".into(),
            dest_denom: compute_ibc_denom_hash("transfer/channel-0/transfer/channel-1/ux"),
            trace_path: "transfer/channel-0/transfer/channel-1/ux".into(),
            route: vec![],
            hop_count: 1, // lie
            preferred: true,
        };
        assert_ne!(
            lying.hop_count,
            hop_count_from_trace_path(&lying.trace_path)
        );
    }
}
