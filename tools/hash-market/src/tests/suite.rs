//! Modular **hashmerchant capability harness** — first-principles e2e without Docker.
//!
//! Evolves the old “headstash-shaped blob” e2e idea into **capability modules**
//! that can run alone or as a matrix. ICT-RS Docker e2e composes the same
//! scenarios with real sidecars (see `ict-rs` docker_sidecar + examples).
//!
//! ## Capabilities exercised
//!
//! | Capability | What it proves | Default path |
//! |------------|----------------|--------------|
//! | **trees** | Merkle build/verify for headstash whitelist | always |
//! | **oracle_bounds** | Multi-source source→attribute→aggregate; `role=bound_only` | opt-in elevated |
//! | **kind_separation** | `state_root` vs `price_bound` providers do not collapse | config |
//! | **ve_payload** | VoteExtensionHashData encode/decode wire contract | msg |
//!
//! Hard rule (Tacit): oracle aggregate is **bounds only**, never mint authority.
//!
//! ```bash
//! cargo test -p hash-market --lib tests::suite
//! ```

use crate::config::ProviderConfig;
use crate::hash::{build as build_tree, verify as verify_tree};
use crate::msg::VoteExtensionHashData;
use crate::oracle::{
    aggregate_bounds, AggregationMethod, AggregationPolicy, AttributedPrice, ProviderKind,
};

// ── Capability matrix ───────────────────────────────────────────────────────

/// Which hashmerchant surfaces a scenario must exercise.
#[derive(Debug, Clone, Copy, Default)]
pub struct CapabilityMatrix {
    pub trees: bool,
    pub oracle_bounds: bool,
    pub kind_separation: bool,
    pub ve_payload: bool,
}

impl CapabilityMatrix {
    /// Full in-process matrix (no Docker / no live HTTP).
    pub fn full_in_process() -> Self {
        Self {
            trees: true,
            oracle_bounds: true,
            kind_separation: true,
            ve_payload: true,
        }
    }

    /// Oracle / pricing-bounds demonstration only (Tacit cUSD-like).
    pub fn oracle_demo() -> Self {
        Self {
            oracle_bounds: true,
            kind_separation: true,
            ..Default::default()
        }
    }

    /// Headstash whitelist trees only (legacy headstash e2e slice).
    pub fn trees_only() -> Self {
        Self {
            trees: true,
            ..Default::default()
        }
    }
}

// ── Scenario results ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TreesResult {
    pub root_hex: String,
    pub member_count: usize,
    pub sample_addr: String,
    pub sample_ok: bool,
}

#[derive(Debug, Clone)]
pub struct OracleBoundsResult {
    pub market_id: String,
    pub method: String,
    pub mantissa: i128,
    pub decimals: u32,
    pub n_sources: u32,
    pub sources: Vec<String>,
    pub role: String,
}

#[derive(Debug, Clone)]
pub struct MatrixReport {
    pub trees: Option<TreesResult>,
    pub oracle: Option<OracleBoundsResult>,
    pub kind_separation_ok: Option<bool>,
    pub ve_roundtrip_ok: Option<bool>,
}

// ── Harness ─────────────────────────────────────────────────────────────────

/// In-process harness: no network, no Docker. Deterministic fixtures.
pub struct HashMarketHarness {
    pub now: u64,
}

impl Default for HashMarketHarness {
    fn default() -> Self {
        Self { now: 1_700_000_000 }
    }
}

impl HashMarketHarness {
    pub fn new() -> Self {
        Self::default()
    }

    /// Run selected capabilities; panics on hard failure (test style).
    pub fn run(&self, matrix: CapabilityMatrix) -> MatrixReport {
        let mut report = MatrixReport {
            trees: None,
            oracle: None,
            kind_separation_ok: None,
            ve_roundtrip_ok: None,
        };

        if matrix.trees {
            report.trees = Some(self.scenario_trees());
        }
        if matrix.oracle_bounds {
            report.oracle = Some(self.scenario_oracle_bounds());
        }
        if matrix.kind_separation {
            report.kind_separation_ok = Some(self.scenario_kind_separation());
        }
        if matrix.ve_payload {
            report.ve_roundtrip_ok = Some(self.scenario_ve_payload());
        }

        report
    }

    /// Headstash-style whitelist tree: build + verify one proof.
    pub fn scenario_trees(&self) -> TreesResult {
        let entries = vec![
            ("terp1aaa".into(), 100u32),
            ("terp1bbb".into(), 200u32),
            ("terp1ccc".into(), 50u32),
        ];
        let (root, members) = build_tree(&entries).expect("build tree");
        let (addr, (alloc, proof)) = members
            .iter()
            .next()
            .map(|(a, v)| (a.clone(), v.clone()))
            .expect("members");
        let ok = verify_tree(&addr, alloc, &proof, &root).expect("verify");
        assert!(ok, "merkle proof must verify against root");
        TreesResult {
            root_hex: root,
            member_count: members.len(),
            sample_addr: addr,
            sample_ok: ok,
        }
    }

    /// Multi-source price bounds: source → attribute → median aggregate.
    ///
    /// Demonstrates Tacit rule: `role == "bound_only"` (never mint).
    pub fn scenario_oracle_bounds(&self) -> OracleBoundsResult {
        let market = "ETH/USD";
        let observations = vec![
            self.tick("binance", market, 3400.00, self.now),
            self.tick("coinbase", market, 3410.50, self.now),
            self.tick("okx", market, 3395.25, self.now - 5),
        ];
        let policy = AggregationPolicy {
            method: AggregationMethod::Median,
            min_sources: 2,
            max_age_secs: 60,
        };
        let bound =
            aggregate_bounds(market, &observations, &policy, self.now).expect("aggregate bounds");

        assert_eq!(bound.role, "bound_only", "oracles never mint");
        assert_eq!(bound.n_sources, 3);
        assert_eq!(bound.market_id, market);
        // Sorted: 3395.25, 3400.00, 3410.50 → median 3400.00 @ 2 decimals → 340000
        assert_eq!(bound.mantissa, 340_000);
        assert_eq!(bound.decimals, 2);

        // Stale isolation: one fresh source fails min_sources=2
        let stale_only = vec![
            self.tick("stale", market, 1.0, self.now - 10_000),
            self.tick("fresh", market, 2.0, self.now),
        ];
        let strict = AggregationPolicy {
            method: AggregationMethod::Mean,
            min_sources: 2,
            max_age_secs: 30,
        };
        assert!(
            aggregate_bounds(market, &stale_only, &strict, self.now).is_err(),
            "stale sources must not satisfy quorum"
        );

        OracleBoundsResult {
            market_id: bound.market_id,
            method: bound.method.as_str().into(),
            mantissa: bound.mantissa,
            decimals: bound.decimals,
            n_sources: bound.n_sources,
            sources: bound.sources,
            role: bound.role,
        }
    }

    /// Config kind resolution: legacy (no kind) = state_root; explicit price_bound.
    pub fn scenario_kind_separation(&self) -> bool {
        let legacy = ProviderConfig {
            name: "eth_mainnet".into(),
            chain_uid: "ethereum-mainnet".into(),
            algo: "keccak256".into(),
            mode: "http_poll".into(),
            address: "http://127.0.0.1:8545".into(),
            interval_secs: 12,
            kind: None,
            market_id: None,
            weight: None,
            static_root: None,
            static_height: None,
            static_block_time: None,
        };
        let price = ProviderConfig {
            name: "binance".into(),
            chain_uid: "price-feeds".into(),
            algo: "price_median_v1".into(),
            mode: "http_poll".into(),
            address: "https://api.example/ticker".into(),
            interval_secs: 5,
            kind: Some("price_bound".into()),
            market_id: Some("ETH/USD".into()),
            weight: Some(1.0),
            static_root: None,
            static_height: None,
            static_block_time: None,
        };
        assert_eq!(legacy.provider_kind(), ProviderKind::StateRoot);
        assert_eq!(price.provider_kind(), ProviderKind::PriceBound);
        assert_ne!(
            legacy.provider_kind(),
            price.provider_kind(),
            "state_root and price_bound must remain separate concerns"
        );
        true
    }

    /// Wire-format contract for VE hash data (root path).
    pub fn scenario_ve_payload(&self) -> bool {
        let original = VoteExtensionHashData {
            runtime_id: "hm-e2e".into(),
            chain_uid: "ethereum-mainnet".into(),
            algo: "keccak256".into(),
            root: vec![0xab; 32],
            foreign_height: 18_000_000,
            foreign_block_time: self.now as i64,
            ics23_proof: vec![],
        };
        let bytes = original.encode();
        let decoded = VoteExtensionHashData::decode(&bytes).expect("decode");
        assert_eq!(decoded.chain_uid, original.chain_uid);
        assert_eq!(decoded.root, original.root);
        assert_eq!(decoded.foreign_height, original.foreign_height);
        true
    }

    fn tick(&self, source: &str, market: &str, price: f64, at: u64) -> AttributedPrice {
        AttributedPrice {
            source: source.into(),
            market_id: market.into(),
            mantissa: (price * 100.0).round() as i128,
            decimals: 2,
            observed_at: at,
            weight: 1.0,
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[test]
fn matrix_full_in_process() {
    let h = HashMarketHarness::new();
    let r = h.run(CapabilityMatrix::full_in_process());
    assert!(r.trees.as_ref().unwrap().sample_ok);
    assert_eq!(r.oracle.as_ref().unwrap().role, "bound_only");
    assert_eq!(r.kind_separation_ok, Some(true));
    assert_eq!(r.ve_roundtrip_ok, Some(true));
}

#[test]
fn matrix_oracle_demo_only() {
    let h = HashMarketHarness::new();
    let r = h.run(CapabilityMatrix::oracle_demo());
    assert!(r.trees.is_none());
    let o = r.oracle.expect("oracle");
    assert_eq!(o.n_sources, 3);
    assert_eq!(o.role, "bound_only");
    assert!(r.kind_separation_ok.unwrap());
}

#[test]
fn matrix_trees_only_legacy_headstash_slice() {
    let h = HashMarketHarness::new();
    let r = h.run(CapabilityMatrix::trees_only());
    let t = r.trees.expect("trees");
    assert_eq!(t.member_count, 3);
    assert!(t.sample_ok);
    assert!(r.oracle.is_none());
}
