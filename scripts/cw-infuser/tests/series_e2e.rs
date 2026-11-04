//! Contract-enforced series cadence (cw-svg-minter factory) + feeshare msg encoding.
//! `cargo test -p cw-infuser-scripts --test series_e2e`
use cosmwasm_std::Binary;
use cw_infuser_scripts::collection::{
    SvgCollectionConfig, SvgCollectionSuite, benders_series, feeshare, marain_series,
};
use cw_orch::prelude::*;

const PACKED: &[u8] = b"SVP1\x00<svg xmlns=\"http://www.w3.org/2000/svg\"/>";

fn cfg(creator: &Addr, i: u8, window: u64) -> SvgCollectionConfig {
    SvgCollectionConfig {
        name: format!("t{i}"),
        symbol: "TST".into(),
        seed: Binary::from([i; 32].to_vec()),
        packed: Binary::from(PACKED.to_vec()),
        total: 10_000,
        price_tiers: vec![],
        mint_start_time: None,
        mint_end_time: None,
        mint_end_after_secs: Some(window),
        creator: creator.clone(),
        minter: None,
        withdraw_address: Some(creator.clone()),
        require_auth: false,
        label: format!("t{i}"),
    }
}

fn err_has(e: anyhow::Error, s: &str) {
    let m = format!("{e:#}");
    assert!(m.contains(s), "expected {s:?} in {m}");
}

#[test]
fn benders_one_per_day_enforced_by_factory() {
    let chain = MockBech32::new("terp");
    let op = chain.sender_addr();
    let mut suite = SvgCollectionSuite::new(chain.clone());
    let f = suite.deploy_factory(Some(&op), &op).unwrap();
    suite
        .set_series(&f, "latent-benders", Some(benders_series(vec![])))
        .unwrap();

    // fresh creator key per collection
    let c1 = chain.addr_make("bender-day-1");
    let a1 = suite
        .instantiate_in_series(&f, "latent-benders", &cfg(&c1, 1, 86_400))
        .unwrap();
    let st = suite.query(&a1).unwrap();
    assert_eq!((st.total, st.mint_start_time), (10_000, None));
    assert_eq!(
        st.mint_end_time,
        Some(chain.block_info().unwrap().time.plus_seconds(86_400))
    );

    // free mint by anyone inside the window
    let collector = chain.addr_make("collector");
    SvgCollectionSuite::new(chain.call_as(&collector))
        .mint(&a1, 1, &[])
        .unwrap();

    // second entry inside the interval is rejected, even for the owner
    chain.wait_seconds(3600).unwrap();
    let c2 = chain.addr_make("bender-day-2");
    err_has(
        suite
            .instantiate_in_series(&f, "latent-benders", &cfg(&c2, 2, 86_400))
            .unwrap_err(),
        "next collection allowed at",
    );
    // mint window longer than 24h is rejected by the series rule
    chain.wait_seconds(23 * 3600).unwrap();
    err_has(
        suite
            .instantiate_in_series(&f, "latent-benders", &cfg(&c2, 2, 86_401))
            .unwrap_err(),
        "mint_end_time required",
    );
    // after the interval it succeeds
    let a2 = suite
        .instantiate_in_series(&f, "latent-benders", &cfg(&c2, 2, 86_400))
        .unwrap();
    assert_ne!(a1, a2);
    let info = suite.series_info(&f, "latent-benders").unwrap();
    assert_eq!(
        info.recent.len(),
        1,
        "older entry pruned: {:?}",
        info.recent
    );
    assert!(info.next_allowed_at.is_some());

    // unknown series and non-owner on an open series are rejected
    err_has(
        suite
            .instantiate_in_series(&f, "nope", &cfg(&c2, 3, 10))
            .unwrap_err(),
        "unknown series",
    );
    let stranger = chain.addr_make("stranger");
    let mut s2 = SvgCollectionSuite::new(chain.call_as(&stranger));
    let f2 = cw_infuser_scripts::collection::SvgFactory {
        minter: f.minter.clone().call_as(&stranger),
    };
    chain.wait_seconds(86_400).unwrap();
    assert!(
        s2.instantiate_in_series(&f2, "latent-benders", &cfg(&c2, 4, 10))
            .is_err()
    );

    // backward compat: legacy CreateSvgCollection (no series) still works for the owner
    let now = chain.block_info().unwrap().time;
    f.minter
        .execute(
            &cw_svg_minter::msg::ExecuteMsg::CreateSvgCollection {
                instantiate_msg: cfg(&c2, 5, 10).to_init_msg(now).unwrap(),
                label: "legacy".into(),
            },
            &[],
        )
        .unwrap();
}

#[test]
fn marain_three_per_day_then_reject() {
    let chain = MockBech32::new("terp");
    let op = chain.sender_addr();
    let suite = SvgCollectionSuite::new(chain.clone());
    let f = suite.deploy_factory(Some(&op), &op).unwrap();
    // a listed operator (not the owner) runs the series
    let bot = chain.addr_make("marain-bot");
    suite
        .set_series(
            &f,
            "marain-orbs",
            Some(marain_series(vec![bot.to_string()])),
        )
        .unwrap();
    let mut bs = SvgCollectionSuite::new(chain.call_as(&bot));
    let bf = cw_infuser_scripts::collection::SvgFactory {
        minter: f.minter.clone().call_as(&bot),
    };

    let t0 = chain.block_info().unwrap().time.seconds();
    for i in 0..3u8 {
        bs.instantiate_in_series(
            &bf,
            "marain-orbs",
            &cfg(&chain.addr_make(format!("orb{i}")), i, 3600),
        )
        .unwrap();
        chain.wait_seconds(600).unwrap();
    }
    let e = bs
        .instantiate_in_series(&bf, "marain-orbs", &cfg(&chain.addr_make("orb3"), 3, 3600))
        .unwrap_err();
    err_has(e, &format!("next at {}", t0 + 86_400));
    assert_eq!(
        suite
            .series_info(&f, "marain-orbs")
            .unwrap()
            .next_allowed_at,
        Some(t0 + 86_400)
    );
    // the first slot frees up exactly one rolling day after the first creation
    let now = chain.block_info().unwrap().time.seconds();
    chain.wait_seconds(t0 + 86_400 - now).unwrap();
    bs.instantiate_in_series(&bf, "marain-orbs", &cfg(&chain.addr_make("orb3"), 3, 3600))
        .unwrap();
    err_has(
        bs.instantiate_in_series(&bf, "marain-orbs", &cfg(&chain.addr_make("orb4"), 4, 3600))
            .unwrap_err(),
        "max 3",
    );
}

#[test]
fn feeshare_msg_is_built_correctly() {
    use prost::Message;
    let chain = MockBech32::new("terp");
    let (c, d, w) = (
        chain.addr_make("c"),
        chain.addr_make("d"),
        chain.addr_make("w"),
    );
    let any = feeshare::msg(c.as_str(), d.as_str(), w.as_str());
    assert_eq!(any.type_url, "/terp.feeshare.v1.MsgRegisterFeeShare");
    let expect = terp_rs::terp::feeshare::v1::MsgRegisterFeeShare {
        contract_address: c.to_string(),
        deployer_address: d.to_string(),
        withdrawer_address: w.to_string(),
    };
    assert_eq!(any.value, expect.encode_to_vec());
    assert_eq!(
        terp_rs::terp::feeshare::v1::MsgRegisterFeeShare::decode(any.value.as_slice()).unwrap(),
        expect
    );
    assert_eq!(
        <terp_rs::terp::feeshare::v1::MsgRegisterFeeShare as prost::Name>::type_url(),
        any.type_url
    );
}
