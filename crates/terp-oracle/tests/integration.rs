//! Multi-test against the real cw-oracle-registry, cw-oracle-aggregator and
//! cw-oracle-twap-probe, through a tiny minter that uses `#[oracle_query]`
//! and `settle_payment` the same way raffle and cw721-svg do.

use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::{
    Addr, BankMsg, Binary, Coin, Decimal, Decimal256, Deps, DepsMut, Empty, Env, MessageInfo,
    Response, StdError, StdResult, Uint256, coins, from_json, to_json_binary,
};
use cw_multi_test::{BankSudo, BasicApp, ContractWrapper, Executor, SudoMsg};
use cw_storage_plus::Item;
use terp_oracle::*;

use cw_oracle_registry::msg as reg;
use cw_oracle_registry::state::SourceKind;

const BASE: &str = "uterp";
const ALT: &str = "uatom";

// ---------- mock minter ----------

#[cw_serde]
struct MInit {
    pricing: Option<OraclePricing>,
    price: Coin,
    payee: String,
}
#[cw_serde]
enum MExec {
    Mint {},
    SetPricing { pricing: Option<OraclePricing> },
}
#[terp_oracle::oracle_query]
#[cw_serde]
#[derive(QueryResponses)]
enum MQuery {
    #[returns(u64)]
    Minted {},
}

const PRICING: Item<Option<OraclePricing>> = Item::new("p");
const PRICE: Item<(Coin, Addr)> = Item::new("c");
const MINTED: Item<u64> = Item::new("m");

fn m_init(deps: DepsMut, _e: Env, _i: MessageInfo, m: MInit) -> StdResult<Response> {
    if let Some(p) = &m.pricing {
        p.validate(deps.api)?;
    }
    PRICING.save(deps.storage, &m.pricing)?;
    PRICE.save(deps.storage, &(m.price, deps.api.addr_validate(&m.payee)?))?;
    MINTED.save(deps.storage, &0)?;
    Ok(Response::new())
}
fn m_exec(deps: DepsMut, env: Env, info: MessageInfo, m: MExec) -> StdResult<Response> {
    match m {
        MExec::SetPricing { pricing } => {
            if let Some(p) = &pricing {
                p.validate(deps.api)?;
            }
            PRICING.save(deps.storage, &pricing)?;
            Ok(Response::new())
        }
        MExec::Mint {} => {
            let pricing = PRICING.load(deps.storage)?;
            let (price, payee) = PRICE.load(deps.storage)?;
            let s = settle_payment(&deps.querier, env.block.height, pricing.as_ref(), &price, &info.funds)?;
            let n = MINTED.load(deps.storage)?; MINTED.save(deps.storage, &(n + 1))?;
            let mut r = Response::new()
                .add_attribute("alt", s.alt.to_string())
                .add_message(BankMsg::Send {
                    to_address: payee.to_string(),
                    amount: vec![s.charged],
                });
            if let Some(refund) = s.refund {
                r = r.add_message(BankMsg::Send {
                    to_address: info.sender.to_string(),
                    amount: vec![refund],
                });
            }
            Ok(r)
        }
    }
}
fn m_query(deps: Deps, env: Env, m: MQuery) -> StdResult<Binary> {
    match m.into_oracle_query() {
        Ok(o) => handle_oracle_query(deps, &env, PRICING.load(deps.storage)?.as_ref(), o),
        Err(MQuery::Minted {}) => to_json_binary(&MINTED.load(deps.storage)?),
        Err(_) => Err(StdError::msg("unreachable")),
    }
}

// ---------- world ----------

struct W {
    app: BasicApp,
    admin: Addr,
    buyer: Addr,
    payee: Addr,
    registry: Addr,
    aggregator: Addr,
    probe: Addr,
}

fn world() -> W {
    let mut app = BasicApp::default();
    let admin = app.api().addr_make("admin");
    let buyer = app.api().addr_make("buyer");
    let payee = app.api().addr_make("payee");
    for d in [BASE, ALT, "uosmo"] {
        app.sudo(SudoMsg::Bank(BankSudo::Mint {
            to_address: buyer.to_string(),
            amount: coins(1_000_000_000, d),
        }))
        .unwrap();
    }
    let rid = app.store_code(Box::new(ContractWrapper::new(
        cw_oracle_registry::contract::execute,
        cw_oracle_registry::contract::instantiate,
        cw_oracle_registry::contract::query,
    )));
    let registry = app
        .instantiate_contract(
            rid,
            admin.clone(),
            &reg::InstantiateMsg {
                admin: admin.to_string(),
                open_submit: true,
                bounty: None,
                note: None,
                ibc2_source_client: None,
            },
            &[],
            "registry",
            None,
        )
        .unwrap();
    let aid = app.store_code(Box::new(ContractWrapper::new(
        cw_oracle_aggregator::execute,
        cw_oracle_aggregator::instantiate,
        cw_oracle_aggregator::query,
    )));
    let aggregator = app
        .instantiate_contract(
            aid,
            admin.clone(),
            &cw_oracle_aggregator::InstantiateMsg {
                registry: registry.to_string(),
            },
            &[],
            "agg",
            None,
        )
        .unwrap();
    let pid = app.store_code(Box::new(ContractWrapper::new(
        cw_oracle_twap_probe::execute,
        cw_oracle_twap_probe::instantiate,
        cw_oracle_twap_probe::query,
    )));
    let probe = app
        .instantiate_contract(
            pid,
            admin.clone(),
            &cw_oracle_twap_probe::InstantiateMsg { mid: "12.5".into() },
            &[],
            "probe",
            None,
        )
        .unwrap();
    W {
        app,
        admin,
        buyer,
        payee,
        registry,
        aggregator,
        probe,
    }
}

impl W {
    fn height(&self) -> u64 {
        self.app.block_info().height
    }
    fn advance(&mut self, n: u64) {
        self.app.update_block(|b| {
            b.height += n;
            b.time = b.time.plus_seconds(5 * n);
        });
    }
    fn submit(&mut self, market: &str, height: u64, mid: &str) {
        self.app
            .execute_contract(
                self.admin.clone(),
                self.registry.clone(),
                &reg::ExecuteMsg::Submit {
                    market_id: market.into(),
                    source: SourceKind::Relayer,
                    source_chain: "terp-local".into(),
                    pool_id: Some(1),
                    height,
                    mid: mid.into(),
                },
                &[],
            )
            .unwrap();
    }
    /// Submit at the current local height, then advance one block.
    fn push(&mut self, market: &str, mid: &str) {
        let h = self.height();
        self.submit(market, h, mid);
        self.advance(1);
    }
    fn feed(&self, kind: FeedKind) -> Feed {
        let oracle = match kind {
            FeedKind::AggregatorMedian { .. } => &self.aggregator,
            FeedKind::ProbeTwap { .. } => &self.probe,
            _ => &self.registry,
        };
        Feed {
            oracle: oracle.to_string(),
            kind,
            weight: None,
        }
    }
    fn minter(&mut self, pricing: Option<OraclePricing>) -> Addr {
        let id = self
            .app
            .store_code(Box::new(ContractWrapper::new(m_exec, m_init, m_query)));
        self.app
            .instantiate_contract(
                id,
                self.admin.clone(),
                &MInit {
                    pricing,
                    price: Coin::new(25_000_000u128, BASE),
                    payee: self.payee.to_string(),
                },
                &[],
                "minter",
                None,
            )
            .unwrap()
    }
    fn mint(&mut self, minter: &Addr, funds: &[Coin]) -> Result<cw_multi_test::AppResponse, String> {
        self.app
            .execute_contract(self.buyer.clone(), minter.clone(), &MExec::Mint {}, funds)
            .map_err(|e| format!("{e:?} {e}"))
    }
    fn bal(&self, a: &Addr, d: &str) -> Uint256 {
        self.app.wrap().query_balance(a.as_str(), d).unwrap().amount
    }
}

fn spot(market: &str) -> FeedKind {
    FeedKind::Spot {
        market_id: market.into(),
    }
}

fn pricing(enforce: bool, feeds: Vec<Feed>, policy: PricingPolicy) -> OraclePricing {
    OraclePricing {
        enforce,
        alt_assets: vec![AltAsset {
            asset: PriceAsset::native(ALT),
            base_denom: BASE.into(),
            feeds,
            combine: AggregateMethod::Median,
            quote: Quote::AltInBase,
            base_decimals: 6,
            alt_decimals: 6,
        }],
        policy,
    }
}

fn err_str(r: Result<cw_multi_test::AppResponse, String>) -> String {
    r.unwrap_err()
}

// ---------- tests ----------

#[test]
fn no_pricing_default_asset_unchanged() {
    let mut w = world();
    let m = w.minter(None);
    w.mint(&m, &coins(25_000_000, BASE)).unwrap();
    assert_eq!(w.bal(&w.payee.clone(), BASE), Uint256::from(25_000_000u128));
    assert!(err_str(w.mint(&m, &coins(24_999_999, BASE))).contains("below the required"));
    assert!(err_str(w.mint(&m, &coins(2_000_000, ALT))).contains("not configured"));
    assert!(err_str(w.mint(&m, &[])).contains("exactly one coin"));
    let p: Option<OraclePricing> = w
        .app
        .wrap()
        .query_wasm_smart(&m, &MQuery::OraclePricing {})
        .unwrap();
    assert!(p.is_none());
    let e = w
        .app
        .wrap()
        .query_wasm_smart::<PriceResponse>(
            &m,
            &MQuery::OraclePrice {
                asset: PriceAsset::native(ALT),
                base_denom: BASE.into(),
            },
        )
        .unwrap_err();
    assert!(e.to_string().contains("not configured"));
}

#[test]
fn not_enforced_quotes_but_rejects_alt() {
    let mut w = world();
    w.push("ATOM/TERP", "12.5");
    let f = w.feed(spot("ATOM/TERP"));
    let m = w.minter(Some(pricing(false, vec![f], PricingPolicy::default())));
    let q: QuoteResponse = w
        .app
        .wrap()
        .query_wasm_smart(
            &m,
            &MQuery::OracleQuote {
                base: Coin::new(25_000_000u128, BASE),
                asset: PriceAsset::native(ALT),
            },
        )
        .unwrap();
    assert_eq!(q.required, Uint256::from(2_000_000u128));
    assert!(!q.enforced);
    assert!(err_str(w.mint(&m, &coins(2_000_000, ALT))).contains("not enforced"));
    w.mint(&m, &coins(25_000_000, BASE)).unwrap();
}

#[test]
fn enforced_alt_exact_overpay_underpay() {
    let mut w = world();
    w.push("ATOM/TERP", "12.5");
    let f = w.feed(spot("ATOM/TERP"));
    let m = w.minter(Some(pricing(true, vec![f], PricingPolicy::default())));
    let before = w.bal(&w.buyer.clone(), ALT);
    let r = w.mint(&m, &coins(2_000_000, ALT)).unwrap();
    assert!(r.events.iter().any(|e| e.attributes.iter().any(|a| a.key == "alt" && a.value == "true")));
    assert_eq!(w.bal(&w.payee.clone(), ALT), Uint256::from(2_000_000u128));
    // overpay: refund
    w.mint(&m, &coins(3_000_000, ALT)).unwrap();
    assert_eq!(w.bal(&w.payee.clone(), ALT), Uint256::from(4_000_000u128));
    assert_eq!(before - w.bal(&w.buyer.clone(), ALT), Uint256::from(4_000_000u128));
    // underpay
    assert!(err_str(w.mint(&m, &coins(1_999_999, ALT))).contains("below the required"));
    // default asset still exact
    w.mint(&m, &coins(25_000_000, BASE)).unwrap();
    // unknown alt
    assert!(err_str(w.mint(&m, &coins(1_000, "uosmo"))).contains("not accepted"));
    let n: u64 = w.app.wrap().query_wasm_smart(&m, &MQuery::Minted {}).unwrap();
    assert_eq!(n, 3);
}

#[test]
fn enforced_price_moves_with_oracle() {
    let mut w = world();
    w.push("ATOM/TERP", "12.5");
    let f = w.feed(spot("ATOM/TERP"));
    let m = w.minter(Some(pricing(true, vec![f], PricingPolicy::default())));
    w.push("ATOM/TERP", "25");
    assert!(err_str(w.mint(&m, &coins(999_999, ALT))).contains("below"));
    w.mint(&m, &coins(1_000_000, ALT)).unwrap();
}

#[test]
fn stale_rejected_then_fresh_ok() {
    let mut w = world();
    w.push("ATOM/TERP", "12.5");
    let f = w.feed(spot("ATOM/TERP"));
    let m = w.minter(Some(pricing(
        true,
        vec![f],
        PricingPolicy {
            max_staleness_blocks: Some(10),
            ..Default::default()
        },
    )));
    w.mint(&m, &coins(2_000_000, ALT)).unwrap();
    w.advance(20);
    assert!(err_str(w.mint(&m, &coins(2_000_000, ALT))).contains("stale"));
    // default asset unaffected by oracle staleness
    w.mint(&m, &coins(25_000_000, BASE)).unwrap();
    w.push("ATOM/TERP", "12.5");
    w.mint(&m, &coins(2_000_000, ALT)).unwrap();
}

#[test]
fn foreign_height_is_future_observation() {
    let mut w = world();
    let h = w.height();
    w.submit("ATOM/TERP", h + 1_000_000, "12.5");
    let f = w.feed(spot("ATOM/TERP"));
    let m = w.minter(Some(pricing(
        true,
        vec![f],
        PricingPolicy {
            max_staleness_blocks: Some(10),
            ..Default::default()
        },
    )));
    assert!(err_str(w.mint(&m, &coins(2_000_000, ALT))).contains("above the current height"));
}

#[test]
fn missing_feed_rejected() {
    let mut w = world();
    let f = w.feed(spot("NOPE"));
    let m = w.minter(Some(pricing(true, vec![f], PricingPolicy::default())));
    assert!(err_str(w.mint(&m, &coins(2_000_000, ALT))).contains("spot:NOPE"));
}

#[test]
fn every_feed_kind_against_real_contracts() {
    let mut w = world();
    for mid in ["10", "12", "14", "12"] {
        w.push("ATOM/TERP", mid);
    }
    let kinds = vec![
        (spot("ATOM/TERP"), "12"),
        (
            FeedKind::History {
                market_id: "ATOM/TERP".into(),
                limit: 4,
                method: AggregateMethod::Mean,
            },
            "12",
        ),
        (
            FeedKind::History {
                market_id: "ATOM/TERP".into(),
                limit: 4,
                method: AggregateMethod::Max,
            },
            "14",
        ),
        (
            FeedKind::Twap {
                market_id: "ATOM/TERP".into(),
                window_blocks: 4,
            },
            "12",
        ),
        (
            FeedKind::AggregatorMedian {
                market_id: "ATOM/TERP".into(),
                window: 4,
            },
            "12",
        ),
        (FeedKind::ProbeTwap { pool_id: 1 }, "12.5"),
    ];
    for (kind, want) in kinds {
        let label = kind.label();
        let f = w.feed(kind);
        let m = w.minter(Some(pricing(true, vec![f], PricingPolicy::default())));
        let p: PriceResponse = w
            .app
            .wrap()
            .query_wasm_smart(
                &m,
                &MQuery::OraclePrice {
                    asset: PriceAsset::native(ALT),
                    base_denom: BASE.into(),
                },
            )
            .unwrap_or_else(|e| panic!("{label}: {e}"));
        assert_eq!(p.price, want.parse::<Decimal256>().unwrap(), "{label}");
        assert_eq!(p.points[0].feed, label);
    }
}

#[test]
fn probe_has_no_age_under_staleness_policy() {
    let mut w = world();
    let f = w.feed(FeedKind::ProbeTwap { pool_id: 1 });
    let m = w.minter(Some(pricing(
        true,
        vec![f],
        PricingPolicy {
            max_staleness_blocks: Some(10),
            ..Default::default()
        },
    )));
    assert!(err_str(w.mint(&m, &coins(2_000_000, ALT))).contains("no height"));
}

#[test]
fn multi_source_deviation_and_min_sources() {
    let mut w = world();
    w.push("ATOM/TERP", "12.5");
    w.push("ATOM/TERP:B", "12.6");
    let feeds = vec![
        w.feed(spot("ATOM/TERP")),
        w.feed(spot("ATOM/TERP:B")),
        w.feed(FeedKind::ProbeTwap { pool_id: 1 }),
    ];
    let pol = PricingPolicy {
        min_sources: Some(3),
        max_deviation: Some(Decimal::percent(2)),
        ..Default::default()
    };
    let m = w.minter(Some(pricing(true, feeds.clone(), pol.clone())));
    let p: PriceResponse = w
        .app
        .wrap()
        .query_wasm_smart(
            &m,
            &MQuery::OraclePrice {
                asset: PriceAsset::native(ALT),
                base_denom: BASE.into(),
            },
        )
        .unwrap();
    assert_eq!(p.sources, 3);
    assert_eq!(p.price, "12.5".parse::<Decimal256>().unwrap());
    w.mint(&m, &coins(2_000_000, ALT)).unwrap();

    // probe jumps: deviation guard trips
    w.app
        .execute_contract(
            w.admin.clone(),
            w.probe.clone(),
            &cw_oracle_twap_probe::ExecuteMsg::SetMid { mid: "20".into() },
            &[],
        )
        .unwrap();
    assert!(err_str(w.mint(&m, &coins(2_000_000, ALT))).contains("deviates"));

    // one feed missing: min_sources trips
    let mut feeds2 = feeds;
    feeds2[1] = w.feed(spot("GONE"));
    let m2 = w.minter(Some(pricing(true, feeds2, pol)));
    assert!(err_str(w.mint(&m2, &coins(2_000_000, ALT))).contains("need 3"));
}

#[test]
fn slippage_window() {
    let mut w = world();
    w.push("ATOM/TERP", "12.5");
    let f = w.feed(spot("ATOM/TERP"));
    let m = w.minter(Some(pricing(
        true,
        vec![f],
        PricingPolicy {
            slippage: Some(Decimal::percent(1)),
            ..Default::default()
        },
    )));
    w.mint(&m, &coins(1_980_000, ALT)).unwrap();
    assert!(err_str(w.mint(&m, &coins(1_979_999, ALT))).contains("below"));
}

#[test]
fn aggregate_and_raw_queries() {
    let mut w = world();
    w.push("ATOM/TERP", "12.5");
    let f = w.feed(spot("ATOM/TERP"));
    let mut p = pricing(true, vec![f], PricingPolicy::default());
    let mut osmo = p.alt_assets[0].clone();
    osmo.asset = PriceAsset::native("uosmo");
    osmo.feeds = vec![w.feed(spot("OSMO/TERP"))];
    p.alt_assets.push(osmo);
    let m = w.minter(Some(p));
    let a: AggregateResponse = w
        .app
        .wrap()
        .query_wasm_smart(
            &m,
            &MQuery::OracleAggregate {
                pairs: vec![
                    AssetPair {
                        asset: PriceAsset::native(ALT),
                        base_denom: BASE.into(),
                    },
                    AssetPair {
                        asset: PriceAsset::native("uosmo"),
                        base_denom: BASE.into(),
                    },
                    AssetPair {
                        asset: PriceAsset::native("ufoo"),
                        base_denom: BASE.into(),
                    },
                ],
            },
        )
        .unwrap();
    assert_eq!(a.entries.len(), 3);
    assert!(a.entries[0].price.is_some());
    assert!(a.entries[1].error.as_ref().unwrap().contains("OSMO/TERP"));
    assert!(a.entries[2].error.as_ref().unwrap().contains("not accepted"));

    // raw: passthrough to a configured oracle
    let raw: Binary = w
        .app
        .wrap()
        .query_wasm_smart(
            &m,
            &MQuery::OracleRaw {
                oracle: w.registry.to_string(),
                msg: to_json_binary(&reg::QueryMsg::Latest {
                    market_id: "ATOM/TERP".into(),
                })
                .unwrap(),
            },
        )
        .unwrap();
    let obs: reg::ObservationResponse = from_json(&raw).unwrap();
    assert_eq!(obs.mid, Decimal::from_ratio(25u128, 2u128));
    // raw: refuses unknown contracts
    let e = w
        .app
        .wrap()
        .query_wasm_smart::<Binary>(
            &m,
            &MQuery::OracleRaw {
                oracle: w.probe.to_string(),
                msg: Binary::default(),
            },
        )
        .unwrap_err();
    assert!(e.to_string().contains("not referenced"));
}

#[test]
fn wire_types_match_real_contracts() {
    let mut w = world();
    w.push("ATOM/TERP", "1.5");
    w.push("ATOM/TERP", "2.5");
    let wrap = w.app.wrap();
    let q = OracleQuerier::new(&wrap);
    let l = q.latest(w.registry.as_str(), "ATOM/TERP").unwrap();
    assert_eq!(l.mid, Decimal::percent(250));
    assert_eq!(l.source, "relayer");
    let h = q.history(w.registry.as_str(), "ATOM/TERP", 10).unwrap();
    assert_eq!(h.observations.len(), 2);
    let t = q
        .twap(w.registry.as_str(), "ATOM/TERP", l.height - 1, l.height)
        .unwrap();
    assert_eq!(t.n, 2);
    assert_eq!(q.aggregator_registry(w.aggregator.as_str()).unwrap(), w.registry);
    assert_eq!(q.median(w.aggregator.as_str(), "ATOM/TERP", 2).unwrap().n, 2);
    assert_eq!(q.probe(w.probe.as_str(), 9).unwrap().pool_id, 9);
    // generic raw typed query
    let c: reg::ConfigResponse = q.raw(w.registry.as_str(), &reg::QueryMsg::Config {}).unwrap();
    assert!(c.open_submit);
}

#[test]
fn bad_config_rejected_on_instantiate() {
    let mut w = world();
    let mut f = w.feed(spot("ATOM/TERP"));
    f.oracle = "nope".into();
    let id = w
        .app
        .store_code(Box::new(ContractWrapper::new(m_exec, m_init, m_query)));
    let r = w.app.instantiate_contract(
        id,
        w.admin.clone(),
        &MInit {
            pricing: Some(pricing(true, vec![f], PricingPolicy::default())),
            price: Coin::new(1u128, BASE),
            payee: w.payee.to_string(),
        },
        &[],
        "m",
        None,
    );
    assert!(r.is_err());
    let _ = Empty {};
}
