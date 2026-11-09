//! Connect-style multi-source price bounds — **stable library demo** (no network).
//!
//! Shows the Skip Connect mapping used by hash-market:
//! many attributed sources → aggregate → one mid per market (`role=bound_only`).
//!
//! ```bash
//! cargo run -p hash-market --example price_oracle
//! ```

use hash_market::oracle::{
    ALGO_PRICE_MEDIAN_V1, AggregationMethod, AggregationPolicy, AttributedPrice,
    OracleAttributeStore, aggregate_bounds, bound_to_vote_extension, decode_bound_root,
    encode_bound_root,
};

fn tick(source: &str, market: &str, price: f64, at: u64) -> AttributedPrice {
    AttributedPrice {
        source: source.into(),
        market_id: market.into(),
        mantissa: (price * 100.0).round() as i128,
        decimals: 2,
        observed_at: at,
        weight: 1.0,
    }
}

fn main() {
    let market = "ETH/USD";
    let now = 1_700_000_000u64;
    let policy = AggregationPolicy {
        method: AggregationMethod::Median,
        min_sources: 2,
        max_age_secs: 120,
    };

    // ── Pure aggregate (library) ───────────────────────────────────────────
    let observations = vec![
        tick("binance", market, 3450.10, now),
        tick("coinbase", market, 3451.00, now),
        tick("okx", market, 3449.50, now),
    ];
    let bound = aggregate_bounds(market, &observations, &policy, now).expect("aggregate");
    assert_eq!(bound.role, "bound_only");
    println!(
        "pure median: market={} mantissa={} decimals={} sources={:?}",
        bound.market_id, bound.mantissa, bound.decimals, bound.sources
    );

    // ── Attribute store (Connect in-process map) ───────────────────────────
    let store = OracleAttributeStore::new();
    for o in &observations {
        let _ = store.upsert_and_aggregate(o.clone(), &policy, now);
    }
    let stored = store.get_bound(market).expect("stored bound");
    assert_eq!(stored.mantissa, bound.mantissa);

    // ── VE packing (one mid = commitment; empty bag) ───────────────────────
    let root = encode_bound_root(bound.decimals, bound.mantissa);
    let (d, m) = decode_bound_root(&root).expect("decode");
    assert_eq!((d, m), (bound.decimals, bound.mantissa));

    let ve = bound_to_vote_extension(
        &bound,
        "demo-sidecar",
        &format!("price:{market}"),
        ALGO_PRICE_MEDIAN_V1,
    );
    assert_eq!(ve.algo, ALGO_PRICE_MEDIAN_V1);
    assert!(ve.ics23_proof.is_empty());
    println!(
        "VE: algo={} chain_uid={} root_hex={} foreign_height={}",
        ve.algo,
        ve.chain_uid,
        hex::encode(&ve.root),
        ve.foreign_height
    );

    println!("ok — library surface for Connect-style bounds is usable offline");
}
