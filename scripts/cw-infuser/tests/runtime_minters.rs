//! Deployed codes 98/99 (be3b34e): can the payment minter be a runtime minter of a new collection?
//! - `runtime_minters` in CreateSvgCollection's instantiate_msg is accepted but silently dropped
//!   (SvgCollectionMetadata / Cw721InstantiateMsg have no such field; the factory re-serializes
//!   the typed msg), so the payment minter still has to pay.
//! - The only writer of DREGG_RUNTIME_MINTERS is the collection's `migrate` entry point
//!   (lib.rs:305-308). A same-code-98 migrate by the collection's wasm admin sets it; after that
//!   the payment minter mints fee-free (to itself), non-minters still pay, IBC-only stays off.
use cosmwasm_std::coin;
use cw_infuser_scripts::collection::SvgCollectionSuite;
use cw_infuser_scripts::fresh_plan::*;
use cw_orch::prelude::*;

fn mint() -> cw721_svg::ExecuteMsg {
    cw721_svg::ExecuteMsg::UpdateExtension {
        msg: cw721_svg::SvgExecuteMsgExt::Mint(cw_svg::MintMsg {
            alloc: 0,
            amnt: 1,
            proof_hashes: vec![],
            ext: None,
        }),
    }
}

#[test]
fn runtime_minters_on_deployed_codes() {
    let chain = MockBech32::new("terp");
    let op = chain.sender_addr();
    let cur = chain.addr_make("fcur");
    let r = Roles {
        fractal_curator: cur.to_string(),
        logos_curator: chain.addr_make("lcur").to_string(),
        payment_minter: PAYMENT_MINTER.to_string(),
        withdraw: chain.addr_make("dao").to_string(),
        operator: op.to_string(),
        salt: PAYMINT_SALT.into(),
    };
    let pm = Addr::unchecked(PAYMENT_MINTER);
    let buyer = chain.addr_make("buyer");
    chain
        .add_balance(&buyer, vec![coin(10_000_000_000, "uthiol")])
        .unwrap();
    let suite = SvgCollectionSuite::new(chain.clone());
    let f = suite.deploy_factory(Some(&op), &op).unwrap();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("svgs/interchain-packed");
    let steps = plan(&r, &dir, "x", 42).unwrap();
    let mut m = steps
        .iter()
        .find(|s| s.id == "2-fractal-collection")
        .unwrap()
        .msg
        .clone();
    let k = m.as_object().unwrap().keys().next().unwrap().clone();
    let im = &mut m[&k]["instantiate_msg"];
    im["collection_info_extension"]
        .as_object_mut()
        .unwrap()
        .remove("whitelist");
    im["collection_info_extension"]["runtime_minters"] = serde_json::json!([PAYMENT_MINTER]);
    im["runtime_minters"] = serde_json::json!([PAYMENT_MINTER]);
    let em: cw_svg_minter::msg::ExecuteMsg = serde_json::from_value(m).unwrap();
    let res = f.minter.call_as(&op).execute(&em, &[]).unwrap();
    let a = Addr::unchecked(res.event_attr_value("wasm", "contract").unwrap());
    let c = suite.at(&a).clone();

    // 1) init-time runtime_minters is dropped: an unpaid payment-minter mint is refused
    assert!(
        c.call_as(&pm).execute(&mint(), &[]).is_err(),
        "runtime_minters in init must not take effect on code 98"
    );

    // 2) same-code migrate by the wasm admin (the curator) sets it
    let info = chain.wasm_querier().contract_info(&a).unwrap();
    assert_eq!(info.admin.map(|x| x.to_string()), Some(cur.to_string()));
    // Mock-only: the in-repo MockContract wrapper omits `migrate`, so store the same cw721-svg
    // entry points with migrate attached (on chain this is a same-code-98 migrate, no upload).
    let w = cw_orch::mock::cw_multi_test::ContractWrapper::new_with_empty(
        cw721_svg::entry::execute,
        cw721_svg::entry::instantiate,
        cw721_svg::entry::query,
    )
    .with_migrate(
        |d: cosmwasm_std::DepsMut,
         e: cosmwasm_std::Env,
         m: cw721_svg::MigrateMsg,
         _: cosmwasm_std::MigrateInfo| cw721_svg::entry::migrate(d, e, m),
    );
    let code98 = chain.app.borrow_mut().store_code(Box::new(w));
    let mm = cw721_svg::MigrateMsg {
        dregg_ibc_only: None,
        runtime_minters: Some(vec![PAYMENT_MINTER.into()]),
    };
    chain
        .call_as(&chain.addr_make("rando"))
        .migrate(&mm, code98, &a)
        .unwrap_err();
    chain.call_as(&cur).migrate(&mm, code98, &a).unwrap();

    // fee-free mint by the payment minter; token owned by the sender, then transferred
    c.call_as(&pm).execute(&mint(), &[]).unwrap();
    let o: serde_json::Value = c
        .query(&cw721_svg::QueryMsg::OwnerOf {
            token_id: "0".into(),
            include_expired: None,
        })
        .unwrap();
    assert_eq!(o["owner"], PAYMENT_MINTER);
    c.call_as(&pm)
        .execute(
            &cw721_svg::ExecuteMsg::TransferNft {
                recipient: buyer.to_string(),
                token_id: "0".into(),
            },
            &[],
        )
        .unwrap();
    // non-minter unpaid refused; paid works (IBC-only gate stays off)
    assert!(
        c.call_as(&chain.addr_make("rando"))
            .execute(&mint(), &[])
            .is_err()
    );
    let tier: cosmwasm_std::Coin = serde_json::from_value(
        steps
            .iter()
            .find(|s| s.id == "2-fractal-collection")
            .unwrap()
            .msg[&k]["instantiate_msg"]["collection_info_extension"]["price_tiers"][0]["price"]
            .clone(),
    )
    .unwrap();
    chain.add_balance(&buyer, vec![tier.clone()]).unwrap();
    c.call_as(&buyer).execute(&mint(), &[tier]).unwrap();
    // pause applies to the runtime minter too (checked before the payment skip, contract.rs:211)
    c.call_as(&cur)
        .execute(
            &cw721_svg::ExecuteMsg::UpdateExtension {
                msg: cw721_svg::SvgExecuteMsgExt::Pause { pause: true },
            },
            &[],
        )
        .ok();
}
