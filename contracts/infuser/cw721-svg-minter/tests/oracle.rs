//! Factory default oracle pricing propagates to newly created collections.
use cosmwasm_std::{Addr, Binary, Coin};
use cw_multi_test::{BasicApp, ContractWrapper, Executor};
use cw_svg::{VariableDef, VariableKind};
use cw_svg_minter::{ExecuteMsg, InstantiateMsg, QueryMsg};
use cw721_svg::{PriceTier, SvgCollectionMetadata, SvgQueryMsgExt};
use terp_oracle::*;

fn pricing(oracle: &Addr) -> OraclePricing {
    OraclePricing {
        enforce: true,
        alt_assets: vec![AltAsset {
            asset: PriceAsset::native("uatom"),
            base_denom: "uterp".into(),
            feeds: vec![Feed {
                oracle: oracle.to_string(),
                kind: FeedKind::Spot { market_id: "ATOM/TERP".into() },
                weight: None,
            }],
            combine: AggregateMethod::Median,
            quote: Quote::AltInBase,
            base_decimals: 6,
            alt_decimals: 6,
        }],
        policy: PricingPolicy::default(),
    }
}

fn svg_msg(creator: &Addr) -> cw721_svg::InstantiateMsg {
    let v = vec![VariableDef { name: "a".into(), kind: VariableKind::Options(vec!["x".into(), "y".into()]) }];
    let t = "<svg xmlns=\"http://www.w3.org/2000/svg\"><text>${a}</text></svg>";
    cw721_svg::InstantiateMsg {
        name: "orb".into(),
        symbol: "ORB".into(),
        minter: None,
        creator: Some(creator.to_string()),
        withdraw_address: None,
        collection_info_extension: SvgCollectionMetadata {
            seed: Binary::from(b"seed".as_slice()),
            packed: cw721_svg::pack_svg(t, &v),
            price_tiers: vec![PriceTier { until_count: 10, price: Coin::new(1_000u128, "uterp") }],
            total: 10,
            mint_start_time: None,
            mint_end_time: None,
            whitelist: None,
            require_auth: false,
        },
    }
}

/// Returns (app, factory, owner, last created collection).
fn create(default: Option<OraclePricing>, update_after: Option<Option<OraclePricing>>) -> (BasicApp, Addr, Addr, Addr) {
    let mut app = BasicApp::default();
    let owner = app.api().addr_make("owner");
    let svg = app.store_code(Box::new(ContractWrapper::new_with_empty(
        cw721_svg::entry::execute,
        cw721_svg::entry::instantiate,
        cw721_svg::entry::query,
    )));
    let fid = app.store_code(Box::new(ContractWrapper::new(
        cw_svg_minter::contract::execute,
        cw_svg_minter::contract::instantiate,
        cw_svg_minter::contract::query,
    )));
    let factory = app
        .instantiate_contract(
            fid,
            owner.clone(),
            &InstantiateMsg {
                owner: Some(owner.to_string()),
                svg_code_id: svg,
                dregg_ibc_only: false,
                runtime_minters: vec![],
                oracle_pricing: default,
            },
            &[],
            "factory",
            None,
        )
        .map_err(|e| format!("{e:?} {e}"))
        .unwrap();
    if let Some(p) = update_after {
        app.execute_contract(owner.clone(), factory.clone(), &ExecuteMsg::UpdateOraclePricing { pricing: p }, &[])
            .map_err(|e| format!("{e:?} {e}"))
            .unwrap();
    }
    let res = app
        .execute_contract(
            owner.clone(),
            factory.clone(),
            &ExecuteMsg::CreateSvgCollection { instantiate_msg: svg_msg(&owner), label: "orb".into() },
            &[],
        )
        .map_err(|e| format!("{e:?} {e}"))
        .unwrap();
    let coll = res
        .events
        .iter()
        .flat_map(|e| e.attributes.iter())
        .find(|a| a.key == "contract")
        .map(|a| Addr::unchecked(a.value.clone()))
        .unwrap();
    (app, factory, owner, coll)
}

fn coll_pricing(app: &BasicApp, coll: &Addr) -> Option<OraclePricing> {
    app.wrap()
        .query_wasm_smart(coll, &cw721_svg::QueryMsg::Extension { msg: SvgQueryMsgExt::OraclePricing {} })
        .unwrap()
}

#[test]
fn no_default_collection_has_no_pricing() {
    let (app, factory, _, coll) = create(None, None);
    assert_eq!(coll_pricing(&app, &coll), None);
    let f: Option<OraclePricing> = app.wrap().query_wasm_smart(&factory, &QueryMsg::OraclePricing {}).unwrap();
    assert_eq!(f, None);
}

#[test]
fn default_pricing_propagates_to_new_collection() {
    let oracle = BasicApp::default().api().addr_make("oracle");
    let p = pricing(&oracle);
    let (app, factory, _, coll) = create(Some(p.clone()), None);
    assert_eq!(coll_pricing(&app, &coll), Some(p.clone()));
    let f: Option<OraclePricing> = app.wrap().query_wasm_smart(&factory, &QueryMsg::OraclePricing {}).unwrap();
    assert_eq!(f, Some(p));
}

#[test]
fn update_sets_and_clears_default() {
    let oracle = BasicApp::default().api().addr_make("oracle");
    let p = pricing(&oracle);
    let (app, _, _, coll) = create(None, Some(Some(p.clone())));
    assert_eq!(coll_pricing(&app, &coll), Some(p.clone()));
    let (app, _, _, coll) = create(Some(p), Some(None));
    assert_eq!(coll_pricing(&app, &coll), None);
}

#[test]
fn update_owner_only_and_validated() {
    let (mut app, factory, owner, _) = create(None, None);
    let rando = app.api().addr_make("rando");
    let p = pricing(&app.api().addr_make("oracle"));
    assert!(app
        .execute_contract(rando, factory.clone(), &ExecuteMsg::UpdateOraclePricing { pricing: Some(p.clone()) }, &[])
        .is_err());
    let mut bad = p;
    bad.alt_assets[0].feeds.clear();
    assert!(app
        .execute_contract(owner, factory, &ExecuteMsg::UpdateOraclePricing { pricing: Some(bad) }, &[])
        .is_err());
}

#[test]
fn invalid_default_rejected_at_instantiate() {
    let mut app = BasicApp::default();
    let owner = app.api().addr_make("owner");
    let fid = app.store_code(Box::new(ContractWrapper::new(
        cw_svg_minter::contract::execute,
        cw_svg_minter::contract::instantiate,
        cw_svg_minter::contract::query,
    )));
    let mut bad = pricing(&app.api().addr_make("oracle"));
    bad.alt_assets[0].feeds.clear();
    let r = app.instantiate_contract(
        fid,
        owner.clone(),
        &InstantiateMsg { owner: None, svg_code_id: 1, dregg_ibc_only: false, runtime_minters: vec![], oracle_pricing: Some(bad) },
        &[],
        "f",
        None,
    );
    assert!(r.is_err());
}
