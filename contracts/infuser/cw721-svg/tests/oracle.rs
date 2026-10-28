//! cw721-svg paid mint with optional oracle pricing for an alt asset.
use cosmwasm_std::{Addr, Binary, Coin, Uint256, coins};
use cw_multi_test::{AppResponse, BankSudo, BasicApp, ContractWrapper, Executor, SudoMsg};
use cw_oracle_registry::msg as reg;
use cw_oracle_registry::state::SourceKind;
use cw_svg::{VariableDef, VariableKind};
use cw721_svg::{
    ExecuteMsg, InstantiateMsg, PriceTier, QueryMsg, SvgCollectionMetadata, SvgExecuteMsgExt,
    SvgQueryMsgExt,
};
use terp_oracle::*;

const BASE: &str = "uterp";
const ALT: &str = "uatom";
const PRICE: u128 = 25_000_000;
const START: u128 = 1_000_000_000;

struct W {
    app: BasicApp,
    factory: Addr,
    creator: Addr,
    buyer: Addr,
    payee: Addr,
    registry: Addr,
    nft: Addr,
}

fn pricing(enforce: bool, registry: &Addr, policy: PricingPolicy) -> OraclePricing {
    OraclePricing {
        enforce,
        alt_assets: vec![AltAsset {
            asset: PriceAsset::native(ALT),
            base_denom: BASE.into(),
            feeds: vec![Feed {
                oracle: registry.to_string(),
                kind: FeedKind::Spot { market_id: "ATOM/TERP".into() },
                weight: None,
            }],
            combine: AggregateMethod::Median,
            quote: Quote::AltInBase,
            base_decimals: 6,
            alt_decimals: 6,
        }],
        policy,
    }
}

fn world(tiers: Vec<PriceTier>) -> W {
    let mut app = BasicApp::default();
    let factory = app.api().addr_make("factory");
    let creator = app.api().addr_make("creator");
    let buyer = app.api().addr_make("buyer");
    let payee = app.api().addr_make("payee");
    for d in [BASE, ALT] {
        app.sudo(SudoMsg::Bank(BankSudo::Mint { to_address: buyer.to_string(), amount: coins(START, d) }))
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
            creator.clone(),
            &reg::InstantiateMsg { admin: creator.to_string(), open_submit: true, bounty: None, note: None, ibc2_source_client: None },
            &[],
            "registry",
            None,
        )
        .unwrap();
    let code = app.store_code(Box::new(ContractWrapper::new_with_empty(
        cw721_svg::entry::execute,
        cw721_svg::entry::instantiate,
        cw721_svg::entry::query,
    )));
    let v = vec![VariableDef { name: "a".into(), kind: VariableKind::Options(vec!["x".into(), "y".into()]) }];
    let t = "<svg xmlns=\"http://www.w3.org/2000/svg\"><text>${a}</text></svg>";
    let nft = app
        .instantiate_contract(
            code,
            factory.clone(),
            &InstantiateMsg {
                name: "orb".into(),
                symbol: "ORB".into(),
                minter: None,
                creator: Some(creator.to_string()),
                withdraw_address: Some(payee.to_string()),
                collection_info_extension: SvgCollectionMetadata {
                    seed: Binary::from(b"seed".as_slice()),
                    packed: cw721_svg::pack_svg(t, &v),
                    price_tiers: tiers,
                    total: 20,
                    mint_start_time: None,
                    mint_end_time: None,
                    whitelist: None,
                    require_auth: false,
                },
            },
            &[],
            "orb",
            None,
        )
        .unwrap();
    let mut w = W { app, factory, creator, buyer, payee, registry, nft };
    w.push("12.5");
    w
}

fn tier(until: u64, amount: u128) -> PriceTier {
    PriceTier { until_count: until, price: Coin::new(amount, BASE) }
}

impl W {
    fn push(&mut self, mid: &str) {
        let h = self.app.block_info().height;
        self.app
            .execute_contract(
                self.creator.clone(),
                self.registry.clone(),
                &reg::ExecuteMsg::Submit {
                    market_id: "ATOM/TERP".into(),
                    source: SourceKind::Relayer,
                    source_chain: "terp-local".into(),
                    pool_id: Some(1),
                    height: h,
                    mid: mid.into(),
                },
                &[],
            )
            .unwrap();
        self.advance(1);
    }
    fn advance(&mut self, n: u64) {
        self.app.update_block(|b| {
            b.height += n;
            b.time = b.time.plus_seconds(5 * n);
        });
    }
    fn set(&mut self, who: &Addr, pricing: Option<OraclePricing>) -> Result<AppResponse, String> {
        self.app
            .execute_contract(
                who.clone(),
                self.nft.clone(),
                &ExecuteMsg::UpdateExtension { msg: SvgExecuteMsgExt::SetOraclePricing { pricing } },
                &[],
            )
            .map_err(|e| format!("{e:?} {e}"))
    }
    fn mint(&mut self, amnt: u64, funds: &[Coin]) -> Result<AppResponse, String> {
        self.app
            .execute_contract(
                self.buyer.clone(),
                self.nft.clone(),
                &ExecuteMsg::UpdateExtension {
                    msg: SvgExecuteMsgExt::Mint(cw_svg::MintMsg { alloc: 0, amnt, proof_hashes: vec![], ext: None }),
                },
                funds,
            )
            .map_err(|e| format!("{e:?} {e}"))
    }
    fn bal(&self, a: &Addr, d: &str) -> u128 {
        let v: Uint256 = self.app.wrap().query_balance(a.as_str(), d).unwrap().amount;
        v.to_string().parse().unwrap()
    }
    fn q<T: serde::de::DeserializeOwned>(&self, m: SvgQueryMsgExt) -> Result<T, String> {
        self.app
            .wrap()
            .query_wasm_smart(&self.nft, &QueryMsg::Extension { msg: m })
            .map_err(|e| e.to_string())
    }
}

#[test]
fn no_pricing_default_mint_unchanged() {
    let mut w = world(vec![tier(20, PRICE)]);
    let e = w.mint(1, &coins(2_000_000, ALT)).unwrap_err();
    assert!(!e.contains("oracle"), "{e}");
    assert!(w.mint(1, &coins(PRICE - 1, BASE)).is_err());
    w.mint(2, &coins(2 * PRICE, BASE)).unwrap();
    assert_eq!(w.bal(&w.payee.clone(), BASE), 2 * PRICE);
    let p: Option<OraclePricing> = w.q(SvgQueryMsgExt::OraclePricing {}).unwrap();
    assert!(p.is_none());
}

#[test]
fn set_pricing_auth_owner_or_instantiator() {
    let mut w = world(vec![tier(20, PRICE)]);
    let (buyer, factory, creator, reg) = (w.buyer.clone(), w.factory.clone(), w.creator.clone(), w.registry.clone());
    let p = pricing(true, &reg, PricingPolicy::default());
    assert!(w.set(&buyer, Some(p.clone())).unwrap_err().contains("nauthorized"));
    w.set(&factory, Some(p.clone())).unwrap();
    w.set(&creator, None).unwrap();
    let mut bad = p.clone();
    bad.alt_assets[0].feeds.clear();
    assert!(w.set(&creator, Some(bad)).is_err());
    w.set(&creator, Some(p.clone())).unwrap();
    let got: Option<OraclePricing> = w.q(SvgQueryMsgExt::OraclePricing {}).unwrap();
    assert_eq!(got, Some(p));
}

#[test]
fn enforced_alt_mint_with_refund_and_base_still_exact() {
    let mut w = world(vec![tier(20, PRICE)]);
    let (creator, reg, buyer, payee) = (w.creator.clone(), w.registry.clone(), w.buyer.clone(), w.payee.clone());
    w.set(&creator, Some(pricing(true, &reg, PricingPolicy::default()))).unwrap();
    let q: QuoteResponse = w
        .q(SvgQueryMsgExt::OracleQuote { base: Coin::new(2 * PRICE, BASE), asset: PriceAsset::native(ALT) })
        .unwrap();
    assert_eq!(q.required, Uint256::from(4_000_000u128));
    w.mint(2, &coins(4_500_000, ALT)).unwrap();
    assert_eq!(w.bal(&payee, ALT), 4_000_000);
    assert_eq!(w.bal(&buyer, ALT), START - 4_000_000);
    assert_eq!(w.bal(&w.nft.clone(), ALT), 0);
    assert!(w.mint(1, &coins(1_999_999, ALT)).unwrap_err().contains("below"));
    assert!(w.mint(1, &coins(PRICE + 1, BASE)).is_err());
    w.mint(1, &coins(PRICE, BASE)).unwrap();
    assert_eq!(w.bal(&payee, BASE), PRICE);
}

#[test]
fn not_enforced_rejects_alt() {
    let mut w = world(vec![tier(20, PRICE)]);
    let (creator, reg) = (w.creator.clone(), w.registry.clone());
    w.set(&creator, Some(pricing(false, &reg, PricingPolicy::default()))).unwrap();
    assert!(w.mint(1, &coins(2_000_000, ALT)).unwrap_err().contains("not enforced"));
    w.mint(1, &coins(PRICE, BASE)).unwrap();
}

#[test]
fn stale_rejected_and_tiers_drive_base() {
    // Tier 1: first 1 at 25 TERP, then 50 TERP.
    let mut w = world(vec![tier(1, PRICE), tier(20, 2 * PRICE)]);
    let (creator, reg) = (w.creator.clone(), w.registry.clone());
    let policy = PricingPolicy { max_staleness_blocks: Some(5), ..Default::default() };
    w.set(&creator, Some(pricing(true, &reg, policy))).unwrap();
    // Spans both tiers: 25 + 50 = 75 TERP = 6 ATOM.
    assert!(w.mint(2, &coins(5_999_999, ALT)).is_err());
    w.mint(2, &coins(6_000_000, ALT)).unwrap();
    w.advance(10);
    assert!(w.mint(1, &coins(4_000_000, ALT)).unwrap_err().to_lowercase().contains("stale"));
    w.push("12.5");
    w.mint(1, &coins(4_000_000, ALT)).unwrap();
}

#[test]
fn free_mint_ignores_pricing() {
    let mut w = world(vec![]);
    let (creator, reg) = (w.creator.clone(), w.registry.clone());
    w.set(&creator, Some(pricing(true, &reg, PricingPolicy::default()))).unwrap();
    assert!(w.mint(1, &coins(1, ALT)).is_err());
    w.mint(1, &[]).unwrap();
}
