//! Unit tests: math, policy evaluation, settlement, config validation, wire JSON.

use cosmwasm_std::testing::MockApi;
use cosmwasm_std::{Coin, Decimal, Decimal256, Uint256, from_json, to_json_string};

use crate::error::OracleError;
use crate::math::*;
use crate::pricing::{evaluate, quote_from_price, settle_with_quote};
use crate::types::*;

fn d(s: &str) -> Decimal256 {
    s.parse().unwrap()
}
fn u(n: u128) -> Uint256 {
    Uint256::from(n)
}
fn oracle() -> String {
    MockApi::default().addr_make("registry").to_string()
}

fn alt(quote: Quote, combine: AggregateMethod, feeds: usize) -> AltAsset {
    AltAsset {
        asset: PriceAsset::native("uatom"),
        base_denom: "uterp".into(),
        feeds: (0..feeds)
            .map(|i| Feed {
                oracle: oracle(),
                kind: FeedKind::Spot {
                    market_id: format!("M{i}"),
                },
                weight: None,
            })
            .collect(),
        combine,
        quote,
        base_decimals: 6,
        alt_decimals: 6,
    }
}

fn pricing(enforce: bool, policy: PricingPolicy) -> OraclePricing {
    OraclePricing {
        enforce,
        alt_assets: vec![alt(Quote::AltInBase, AggregateMethod::Median, 1)],
        policy,
    }
}

fn pt(v: &str, h: Option<u64>) -> PricePoint {
    PricePoint {
        feed: format!("f{v}"),
        oracle: oracle(),
        value: d(v),
        height: h,
        observations: 1,
        weight: 1,
    }
}

// ---------- conversion ----------

#[test]
fn alt_in_base_basic() {
    // 1 ATOM = 12.5 TERP; 25 TERP -> 2 ATOM
    assert_eq!(
        base_to_alt(u(25_000_000), d("12.5"), Quote::AltInBase, 6, 6).unwrap(),
        u(2_000_000)
    );
}

#[test]
fn base_in_alt_basic() {
    // 1 TERP = 0.08 ATOM; 25 TERP -> 2 ATOM
    assert_eq!(
        base_to_alt(u(25_000_000), d("0.08"), Quote::BaseInAlt, 6, 6).unwrap(),
        u(2_000_000)
    );
}

#[test]
fn directions_agree_on_inverse() {
    for (base, p) in [
        (1u128, "3"),
        (999_999, "7.77"),
        (10_u128.pow(12), "0.000123"),
    ] {
        let a = base_to_alt(u(base), d(p), Quote::AltInBase, 6, 6).unwrap();
        let inv = invert(d(p)).unwrap();
        let b = base_to_alt(u(base), inv, Quote::BaseInAlt, 6, 6).unwrap();
        // inverse loses at most the 18th decimal; ceil keeps both >= exact
        let diff = if a > b { a - b } else { b - a };
        assert!(diff <= u(1), "{base} {p}: {a} vs {b}");
    }
}

#[test]
fn rounds_up() {
    // 1 uterp at 3 TERP/ATOM = 0.333.. uatom -> 1
    assert_eq!(
        base_to_alt(u(1), d("3"), Quote::AltInBase, 6, 6).unwrap(),
        u(1)
    );
    assert_eq!(
        base_to_alt(u(10), d("3"), Quote::AltInBase, 6, 6).unwrap(),
        u(4)
    );
    assert_eq!(
        base_to_alt(u(9), d("3"), Quote::AltInBase, 6, 6).unwrap(),
        u(3)
    );
    assert_eq!(
        base_to_alt(u(1), d("0.000001"), Quote::BaseInAlt, 6, 6).unwrap(),
        u(1)
    );
}

#[test]
fn decimals_scale() {
    // base 6 dp, alt 18 dp (e.g. aevmos). 1 TERP at 2 TERP/EVMOS = 0.5 EVMOS
    assert_eq!(
        base_to_alt(u(1_000_000), d("2"), Quote::AltInBase, 6, 18).unwrap(),
        u(500_000_000_000_000_000)
    );
    // base 18 dp, alt 6 dp
    assert_eq!(
        base_to_alt(u(10u128.pow(18)), d("2"), Quote::AltInBase, 18, 6).unwrap(),
        u(500_000)
    );
    // 0 decimals both sides
    assert_eq!(
        base_to_alt(u(7), d("2"), Quote::BaseInAlt, 0, 0).unwrap(),
        u(14)
    );
}

#[test]
fn zero_amount_and_zero_price() {
    assert_eq!(
        base_to_alt(u(0), d("5"), Quote::AltInBase, 6, 6).unwrap(),
        u(0)
    );
    assert_eq!(
        base_to_alt(u(1), d("0"), Quote::AltInBase, 6, 6).unwrap_err(),
        OracleError::ZeroPrice
    );
    assert_eq!(invert(d("0")).unwrap_err(), OracleError::ZeroPrice);
    assert_eq!(
        deviation(d("1"), d("0")).unwrap_err(),
        OracleError::ZeroPrice
    );
}

#[test]
fn tiny_price_huge_result_overflows() {
    let r = base_to_alt(
        Uint256::MAX,
        d("0.000000000000000001"),
        Quote::AltInBase,
        0,
        24,
    );
    assert_eq!(r.unwrap_err(), OracleError::Overflow);
    let r = base_to_alt(Uint256::MAX, Decimal256::MAX, Quote::BaseInAlt, 0, 24);
    assert_eq!(r.unwrap_err(), OracleError::Overflow);
}

#[test]
fn max_values_within_range_do_not_overflow() {
    // u128::MAX base at price 1, equal decimals -> identity
    let base = Uint256::from(u128::MAX);
    assert_eq!(
        base_to_alt(base, d("1"), Quote::AltInBase, 24, 24).unwrap(),
        base
    );
    assert_eq!(
        base_to_alt(base, d("1"), Quote::BaseInAlt, 24, 24).unwrap(),
        base
    );
}

#[test]
fn alt_to_base_rounds_down_and_inverts() {
    let alt = base_to_alt(u(10), d("3"), Quote::AltInBase, 6, 6).unwrap(); // 4
    assert_eq!(
        alt_to_base(alt, d("3"), Quote::AltInBase, 6, 6).unwrap(),
        u(12)
    );
    assert_eq!(
        alt_to_base(u(1), d("3"), Quote::BaseInAlt, 6, 6).unwrap(),
        u(0)
    );
    assert_eq!(
        alt_to_base(u(2_000_000), d("0.08"), Quote::BaseInAlt, 6, 6).unwrap(),
        u(25_000_000)
    );
    assert_eq!(
        alt_to_base(u(1), d("0"), Quote::AltInBase, 6, 6).unwrap_err(),
        OracleError::ZeroPrice
    );
}

#[test]
fn slippage() {
    assert_eq!(apply_slippage(u(1000), None).unwrap(), u(1000));
    assert_eq!(
        apply_slippage(u(1000), Some(Decimal::zero())).unwrap(),
        u(1000)
    );
    assert_eq!(
        apply_slippage(u(1000), Some(Decimal::percent(1))).unwrap(),
        u(990)
    );
    // ceil: 999 * 0.99 = 989.01 -> 990
    assert_eq!(
        apply_slippage(u(999), Some(Decimal::percent(1))).unwrap(),
        u(990)
    );
    assert!(apply_slippage(u(1), Some(Decimal::one())).is_err());
}

#[test]
fn parse_price_strings() {
    assert_eq!(parse_price("1.5").unwrap(), d("1.5"));
    assert_eq!(parse_price(" 2 ").unwrap(), d("2"));
    assert!(matches!(
        parse_price("x"),
        Err(OracleError::BadPrice { .. })
    ));
    assert!(matches!(
        parse_price("-1"),
        Err(OracleError::BadPrice { .. })
    ));
}

// ---------- aggregation ----------

fn vals(v: &[(&str, u64)]) -> Vec<(Decimal256, u64)> {
    v.iter().map(|(s, w)| (d(s), *w)).collect()
}

#[test]
fn combine_modes() {
    let v = vals(&[("1", 1), ("5", 3), ("3", 1)]);
    assert_eq!(combine(&v, AggregateMethod::Median).unwrap(), d("3"));
    assert_eq!(combine(&v, AggregateMethod::Min).unwrap(), d("1"));
    assert_eq!(combine(&v, AggregateMethod::Max).unwrap(), d("5"));
    // mean weighted by weight: (1 + 15 + 3) / 5
    assert_eq!(combine(&v, AggregateMethod::Mean).unwrap(), d("3.8"));
    assert_eq!(combine(&v, AggregateMethod::Weighted).unwrap(), d("3.8"));
}

#[test]
fn median_even_and_single() {
    assert_eq!(
        combine(
            &vals(&[("4", 1), ("1", 1), ("2", 1), ("3", 1)]),
            AggregateMethod::Median
        )
        .unwrap(),
        d("2.5")
    );
    assert_eq!(
        combine(&vals(&[("7", 1)]), AggregateMethod::Median).unwrap(),
        d("7")
    );
}

#[test]
fn combine_empty_and_zero_weights() {
    assert!(matches!(
        combine(&[], AggregateMethod::Median),
        Err(OracleError::NoObservations { .. })
    ));
    // Mean treats weight 0 as 1; Weighted skips it
    assert_eq!(
        combine(&vals(&[("2", 0), ("4", 0)]), AggregateMethod::Mean).unwrap(),
        d("3")
    );
    assert!(combine(&vals(&[("2", 0)]), AggregateMethod::Weighted).is_err());
}

#[test]
fn history_weighted_is_block_gap_twap() {
    // heights 10 (v=1), 13 (v=4), 14 (v=10): weights 3, 1, 1 -> (3+4+10)/5
    let rows = vals(&[("10", 14), ("1", 10), ("4", 13)]);
    assert_eq!(
        combine_history(&rows, AggregateMethod::Weighted).unwrap(),
        d("3.4")
    );
    assert_eq!(
        combine_history(&rows, AggregateMethod::Median).unwrap(),
        d("4")
    );
    assert_eq!(
        combine_history(&rows, AggregateMethod::Mean).unwrap(),
        d("5")
    );
}

#[test]
fn deviation_math() {
    assert_eq!(deviation(d("105"), d("100")).unwrap(), d("0.05"));
    assert_eq!(deviation(d("95"), d("100")).unwrap(), d("0.05"));
    assert_eq!(deviation(d("100"), d("100")).unwrap(), d("0"));
}

// ---------- policy ----------

#[test]
fn evaluate_happy_path() {
    let a = alt(Quote::AltInBase, AggregateMethod::Median, 3);
    let r = evaluate(
        &a,
        &PricingPolicy::default(),
        100,
        vec![
            Ok(pt("10", Some(90))),
            Ok(pt("12", Some(95))),
            Ok(pt("11", Some(99))),
        ],
    )
    .unwrap();
    assert_eq!(r.price, d("11"));
    assert_eq!(r.alt_in_base, d("11"));
    assert_eq!(r.sources, 3);
    assert_eq!(r.observations, 3);
    assert_eq!(r.oldest_height, Some(90));
    assert_eq!(r.newest_height, Some(99));
    assert_eq!(r.max_deviation, deviation(d("10"), d("11")).unwrap());
}

#[test]
fn evaluate_base_in_alt_inverts_display() {
    let a = alt(Quote::BaseInAlt, AggregateMethod::Median, 1);
    let r = evaluate(
        &a,
        &PricingPolicy::default(),
        1,
        vec![Ok(pt("0.08", Some(1)))],
    )
    .unwrap();
    assert_eq!(r.price, d("0.08"));
    assert_eq!(r.alt_in_base, d("12.5"));
}

#[test]
fn stale_rejected_and_boundary_ok() {
    let a = alt(Quote::AltInBase, AggregateMethod::Median, 1);
    let p = PricingPolicy {
        max_staleness_blocks: Some(10),
        ..Default::default()
    };
    assert!(evaluate(&a, &p, 100, vec![Ok(pt("1", Some(90)))]).is_ok());
    assert_eq!(
        evaluate(&a, &p, 100, vec![Ok(pt("1", Some(89)))]).unwrap_err(),
        OracleError::Stale {
            feed: "f1".into(),
            observed: 89,
            current: 100,
            max_age: 10
        }
    );
}

#[test]
fn future_and_unknown_age() {
    let a = alt(Quote::AltInBase, AggregateMethod::Median, 1);
    let p = PricingPolicy {
        max_staleness_blocks: Some(10),
        ..Default::default()
    };
    assert!(matches!(
        evaluate(&a, &p, 100, vec![Ok(pt("1", Some(101)))]),
        Err(OracleError::FutureObservation { .. })
    ));
    assert!(matches!(
        evaluate(&a, &p, 100, vec![Ok(pt("1", None))]),
        Err(OracleError::UnknownAge { .. })
    ));
    // no staleness policy: heightless feed is fine
    assert!(evaluate(&a, &PricingPolicy::default(), 100, vec![Ok(pt("1", None))]).is_ok());
}

#[test]
fn stale_feed_dropped_when_others_meet_min_sources() {
    let a = alt(Quote::AltInBase, AggregateMethod::Median, 3);
    let p = PricingPolicy {
        max_staleness_blocks: Some(5),
        min_sources: Some(2),
        ..Default::default()
    };
    let r = evaluate(
        &a,
        &p,
        100,
        vec![
            Ok(pt("10", Some(99))),
            Ok(pt("50", Some(1))),
            Ok(pt("12", Some(98))),
        ],
    )
    .unwrap();
    assert_eq!(r.sources, 2);
    assert_eq!(r.price, d("11"));
}

#[test]
fn not_enough_sources() {
    let a = alt(Quote::AltInBase, AggregateMethod::Median, 3);
    let p = PricingPolicy {
        min_sources: Some(2),
        ..Default::default()
    };
    let e = evaluate(
        &a,
        &p,
        1,
        vec![
            Ok(pt("1", Some(1))),
            Err(OracleError::NoObservations { feed: "x".into() }),
            Err(OracleError::ZeroPrice),
        ],
    )
    .unwrap_err();
    assert_eq!(e, OracleError::NotEnoughSources { got: 1, min: 2 });
}

#[test]
fn all_failed_returns_first_error() {
    let a = alt(Quote::AltInBase, AggregateMethod::Median, 2);
    let e = evaluate(
        &a,
        &PricingPolicy::default(),
        1,
        vec![
            Err(OracleError::NoObservations { feed: "x".into() }),
            Err(OracleError::ZeroPrice),
        ],
    )
    .unwrap_err();
    assert_eq!(e, OracleError::NoObservations { feed: "x".into() });
}

#[test]
fn min_observations() {
    let a = alt(Quote::AltInBase, AggregateMethod::Median, 1);
    let p = PricingPolicy {
        min_observations: Some(3),
        ..Default::default()
    };
    let mut point = pt("1", Some(1));
    assert_eq!(
        evaluate(&a, &p, 1, vec![Ok(point.clone())]).unwrap_err(),
        OracleError::NotEnoughObservations { got: 1, min: 3 }
    );
    point.observations = 3;
    assert!(evaluate(&a, &p, 1, vec![Ok(point)]).is_ok());
}

#[test]
fn deviation_guard() {
    let a = alt(Quote::AltInBase, AggregateMethod::Median, 3);
    let p = PricingPolicy {
        max_deviation: Some(Decimal::percent(5)),
        ..Default::default()
    };
    let ok = vec![
        Ok(pt("100", Some(1))),
        Ok(pt("104", Some(1))),
        Ok(pt("101", Some(1))),
    ];
    assert!(evaluate(&a, &p, 1, ok).is_ok());
    let bad = vec![
        Ok(pt("100", Some(1))),
        Ok(pt("120", Some(1))),
        Ok(pt("101", Some(1))),
    ];
    assert!(matches!(
        evaluate(&a, &p, 1, bad),
        Err(OracleError::Deviation { .. })
    ));
}

#[test]
fn weighted_uses_observations_mean_uses_weight() {
    let mut a = alt(Quote::AltInBase, AggregateMethod::Weighted, 2);
    let mut p1 = pt("10", Some(1));
    p1.observations = 3;
    p1.weight = 1;
    let mut p2 = pt("20", Some(1));
    p2.observations = 1;
    p2.weight = 3;
    let r = evaluate(
        &a,
        &PricingPolicy::default(),
        1,
        vec![Ok(p1.clone()), Ok(p2.clone())],
    )
    .unwrap();
    assert_eq!(r.price, d("12.5"));
    a.combine = AggregateMethod::Mean;
    let r = evaluate(&a, &PricingPolicy::default(), 1, vec![Ok(p1), Ok(p2)]).unwrap();
    assert_eq!(r.price, d("17.5"));
}

// ---------- quote + settlement ----------

fn quote_at(p: &OraclePricing, price: &str, base: u128) -> QuoteResponse {
    let a = &p.alt_assets[0];
    let pr = evaluate(a, &p.policy, 1, vec![Ok(pt(price, Some(1)))]).unwrap();
    quote_from_price(p, a, &Coin::new(base, "uterp"), pr).unwrap()
}

#[test]
fn quote_with_slippage() {
    let p = pricing(
        true,
        PricingPolicy {
            slippage: Some(Decimal::percent(2)),
            ..Default::default()
        },
    );
    let q = quote_at(&p, "12.5", 25_000_000);
    assert_eq!(q.required, u(2_000_000));
    assert_eq!(q.minimum, u(1_960_000));
    assert!(q.enforced);
    assert_eq!(q.asset, PriceAsset::native("uatom"));
}

#[test]
fn settle_exact_over_under() {
    let p = pricing(
        true,
        PricingPolicy {
            slippage: Some(Decimal::percent(2)),
            ..Default::default()
        },
    );
    let q = quote_at(&p, "12.5", 25_000_000);

    let s = settle_with_quote(&Coin::new(2_000_000u128, "uatom"), q.clone()).unwrap();
    assert_eq!(s.charged, Coin::new(2_000_000u128, "uatom"));
    assert_eq!(s.refund, None);
    assert!(s.alt);

    let s = settle_with_quote(&Coin::new(2_500_000u128, "uatom"), q.clone()).unwrap();
    assert_eq!(s.charged.amount, u(2_000_000));
    assert_eq!(s.refund, Some(Coin::new(500_000u128, "uatom")));

    // inside slippage: charged what was paid
    let s = settle_with_quote(&Coin::new(1_970_000u128, "uatom"), q.clone()).unwrap();
    assert_eq!(s.charged.amount, u(1_970_000));
    assert_eq!(s.refund, None);

    let e = settle_with_quote(&Coin::new(1_959_999u128, "uatom"), q).unwrap_err();
    assert_eq!(
        e,
        OracleError::Underpaid {
            paid: u(1_959_999),
            required: u(1_960_000),
            denom: "uatom".into()
        }
    );
}

#[test]
fn settle_zero_paid_rejected() {
    let p = pricing(true, PricingPolicy::default());
    let mut q = quote_at(&p, "12.5", 0);
    q.minimum = u(0);
    assert!(settle_with_quote(&Coin::new(0u128, "uatom"), q).is_err());
}

// ---------- config validation ----------

#[test]
fn validate_ok() {
    let api = MockApi::default();
    pricing(true, PricingPolicy::default())
        .validate(&api)
        .unwrap();
    OraclePricing {
        enforce: false,
        alt_assets: vec![],
        policy: PricingPolicy::default(),
    }
    .validate(&api)
    .unwrap();
}

#[test]
fn validate_rejects() {
    let api = MockApi::default();
    let base = pricing(true, PricingPolicy::default());
    let check = |f: &dyn Fn(&mut OraclePricing)| {
        let mut p = base.clone();
        f(&mut p);
        p.validate(&api).unwrap_err()
    };
    check(&|p| p.alt_assets[0].asset = PriceAsset::native(""));
    check(&|p| p.alt_assets[0].asset = PriceAsset::native("uterp"));
    check(&|p| p.alt_assets[0].asset = PriceAsset::cw20("not-an-addr"));
    check(&|p| p.alt_assets[0].base_denom = String::new());
    check(&|p| {
        let a = p.alt_assets[0].clone();
        p.alt_assets.push(a)
    });
    check(&|p| p.alt_assets[0].feeds.clear());
    check(&|p| {
        let f = p.alt_assets[0].feeds[0].clone();
        p.alt_assets[0].feeds = vec![f; MAX_FEEDS + 1]
    });
    check(&|p| p.alt_assets[0].alt_decimals = MAX_DECIMALS + 1);
    check(&|p| p.alt_assets[0].feeds[0].oracle = "bad".into());
    check(&|p| p.alt_assets[0].feeds[0].weight = Some(0));
    check(&|p| {
        p.alt_assets[0].feeds[0].kind = FeedKind::Spot {
            market_id: String::new(),
        }
    });
    check(&|p| {
        p.alt_assets[0].feeds[0].kind = FeedKind::Twap {
            market_id: "M".into(),
            window_blocks: 0,
        }
    });
    check(&|p| {
        p.alt_assets[0].feeds[0].kind = FeedKind::History {
            market_id: "M".into(),
            limit: 0,
            method: AggregateMethod::Mean,
        }
    });
    check(&|p| {
        p.alt_assets[0].feeds[0].kind = FeedKind::AggregatorMedian {
            market_id: "M".into(),
            window: 0,
        }
    });
    check(&|p| p.policy.min_sources = Some(2));
    check(&|p| p.policy.min_sources = Some(0));
    check(&|p| p.policy.max_deviation = Some(Decimal::percent(101)));
    check(&|p| p.policy.slippage = Some(Decimal::one()));
    check(&|p| {
        let a = p.alt_assets[0].clone();
        p.alt_assets = (0..=MAX_ALT_ASSETS)
            .map(|i| AltAsset {
                asset: PriceAsset::native(format!("u{i}")),
                ..a.clone()
            })
            .collect()
    });
}

#[test]
fn find_and_oracles() {
    let mut p = pricing(true, PricingPolicy::default());
    let mut b = p.alt_assets[0].clone();
    b.asset = PriceAsset::native("uosmo");
    b.feeds[0].oracle = MockApi::default().addr_make("other").to_string();
    p.alt_assets.push(b);
    assert!(p.find(&PriceAsset::native("uosmo"), "uterp").is_some());
    assert!(p.find(&PriceAsset::native("uosmo"), "ujuno").is_none());
    assert_eq!(p.oracles().len(), 2);
}

#[test]
fn labels() {
    assert_eq!(
        FeedKind::Spot {
            market_id: "A".into()
        }
        .label(),
        "spot:A"
    );
    assert_eq!(FeedKind::ProbeTwap { pool_id: 7 }.label(), "probe:7");
    assert_eq!(
        FeedKind::History {
            market_id: "a".into(),
            limit: 3,
            method: AggregateMethod::Weighted
        }
        .label(),
        "history:a:3:weighted"
    );
}

// ---------- JSON ----------

#[test]
fn json_defaults_and_shape() {
    let j = format!(
        r#"{{"enforce":true,"alt_assets":[{{"asset":{{"native":"uatom"}},"base_denom":"uterp","feeds":[{{"oracle":"{}","kind":{{"spot":{{"market_id":"ATOM/TERP"}}}}}}],"quote":"alt_in_base","base_decimals":6,"alt_decimals":6}}]}}"#,
        oracle()
    );
    let p: OraclePricing = from_json(j.as_bytes()).unwrap();
    assert_eq!(p.alt_assets[0].combine, AggregateMethod::Median);
    assert_eq!(p.policy, PricingPolicy::default());
    assert_eq!(p.alt_assets[0].feeds[0].weight, None);
    let back: OraclePricing = from_json(to_json_string(&p).unwrap().as_bytes()).unwrap();
    assert_eq!(back, p);
}

#[test]
fn error_into_std() {
    let e: cosmwasm_std::StdError = OracleError::NotEnforced.into();
    assert!(e.to_string().contains("not enforced"));
}
