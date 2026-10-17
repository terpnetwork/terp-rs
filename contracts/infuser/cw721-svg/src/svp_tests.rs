//! SVP1 packed-only tests: render parity, storage layout, corruption rejection.
use crate::contract::{SvgTemplateResponse, SvgTokenUriResponse, SVG_COLLECTION, SVG_PACKED_KEY};
use crate::entry::{execute, instantiate, query};
use crate::{ExecuteMsg, InstantiateMsg, QueryMsg, SvgCollectionMetadata, SvgExecuteMsgExt, SvgQueryMsgExt};
use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
use cosmwasm_std::{from_json, Binary, Storage};
use cw_svg::VariableDef;

#[derive(serde::Deserialize)]
struct Fx {
    svg_template: String,
    variables: Vec<VariableDef>,
}

fn fixture() -> Fx {
    from_json(include_str!("../../../../crates/svg-codec/fixtures/corpus/ex-shield-amber.json")).unwrap()
}

fn msg(packed: Vec<u8>) -> InstantiateMsg {
    InstantiateMsg {
        name: "t".into(),
        symbol: "T".into(),
        minter: None,
        creator: None,
        withdraw_address: None,
        collection_info_extension: SvgCollectionMetadata {
            seed: Binary::from(b"seed".as_slice()),
            packed: packed.into(),
            price_tiers: vec![],
            total: 10,
            mint_start_time: None,
            mint_end_time: None,
            whitelist: None,
            require_auth: false,
        },
    }
}

#[derive(Debug)]
struct Out {
    template: String,
    token0: String,
    params0: Vec<cw_svg::TokenParam>,
    packed_query: Binary,
}

fn run(packed: Vec<u8>) -> Result<Out, String> {
    let mut deps = mock_dependencies();
    let env = mock_env();
    let creator = deps.api.addr_make("creator");
    instantiate(deps.as_mut(), env.clone(), message_info(&creator, &[]), msg(packed.clone()))
        .map_err(|e| e.to_string())?;
    let mint = ExecuteMsg::UpdateExtension {
        msg: SvgExecuteMsgExt::Mint(cw_svg::MintMsg { alloc: 0, amnt: 1, proof_hashes: vec![], ext: None }),
    };
    execute(deps.as_mut(), env.clone(), message_info(&creator, &[]), mint).map_err(|e| e.to_string())?;
    let q = |m: SvgQueryMsgExt| query(deps.as_ref(), env.clone(), QueryMsg::Extension { msg: m }).unwrap();
    let t: SvgTemplateResponse = from_json(q(SvgQueryMsgExt::SvgTemplate {})).unwrap();
    let u: SvgTokenUriResponse = from_json(q(SvgQueryMsgExt::SvgTokenUri { token_id: "0".into() })).unwrap();
    let p: Binary = from_json(q(SvgQueryMsgExt::SvgPacked {})).unwrap();
    // Packed bytes live once, raw; config copies carry none.
    assert_eq!(deps.storage.get(SVG_PACKED_KEY).unwrap(), packed);
    assert!(SVG_COLLECTION.load(&deps.storage).unwrap().packed.is_empty());
    for k in [b"svg_template".as_slice(), b"template_slots", b"variables"] {
        assert!(deps.storage.get(k).is_none());
    }
    let info: cw721::msg::CollectionInfoAndExtensionResponse<SvgCollectionMetadata> =
        from_json(query(deps.as_ref(), env.clone(), QueryMsg::GetCollectionInfoAndExtension {}).unwrap()).unwrap();
    assert!(info.extension.packed.is_empty());
    let tok = crate::Cw721SvgContract::default().config.nft_info.load(&deps.storage, "0").unwrap();
    Ok(Out { template: t.template, token0: u.svg, params0: tok.extension.params, packed_query: p })
}

#[test]
fn packed_renders_like_direct_substitution() {
    let f = fixture();
    let packed = cw_svg_pack::pack(&f.svg_template, &f.variables).unwrap();
    let o = run(packed.clone()).unwrap();
    assert_eq!(o.template, f.svg_template);
    assert_eq!(o.packed_query.as_slice(), packed.as_slice());
    // Reference render: plain slot substitution on the source template.
    let slots = cw_svg::compute_template_slots(&f.svg_template, &f.variables);
    let mut want = String::new();
    let mut c = 0;
    for s in &slots {
        want.push_str(&f.svg_template[c..s.start as usize]);
        want.push_str(&o.params0[s.var_idx as usize].value);
        c = s.end as usize;
    }
    want.push_str(&f.svg_template[c..]);
    assert_eq!(o.token0, want);
}

#[test]
fn rejects_corruption() {
    let f = fixture();
    let good = cw_svg_pack::pack(&f.svg_template, &f.variables).unwrap();
    let mut b = good.clone();
    b[0] ^= 0xff;
    assert!(run(b).unwrap_err().contains("svp"));
    assert!(run(good[..good.len() / 2].to_vec()).is_err());
    let mut b = good.clone();
    b.push(0);
    assert!(run(b).is_err());
    assert!(run(vec![]).is_err());
    for i in 4..good.len() {
        let mut b = good.clone();
        b[i] ^= 0x5a;
        let _ = run(b); // must not panic
    }
    assert!(run(vec![0u8; crate::contract::MAX_SVG_SIZE + 1]).unwrap_err().contains("exceeds max size"));
}

#[test]
fn rejects_undefined_placeholder_in_literal() {
    let mut b = vec![0x53, 0x56, 0x50, 0x01, 0x00, 0x01, 7];
    b.extend_from_slice(b"${nope}");
    b.push(0x00);
    assert!(run(b).unwrap_err().contains("undefined variable"));
}

#[test]
fn runs_animated_paint_validator() {
    // Live DAO template: <circle> animates fill with no static fill.
    let f: Fx = from_json(include_str!("../../../../crates/svg-codec/fixtures/corpus/ex-dao-live.json")).unwrap();
    assert!(cw_svg::validate_animated_paint(&f.svg_template).is_err());
    let e = run(cw_svg_pack::pack(&f.svg_template, &f.variables).unwrap()).unwrap_err();
    assert!(e.contains("animates"), "{e}");
    // The fixed DAO template passes.
    let f: Fx = from_json(include_str!("../../../../crates/svg-codec/fixtures/corpus/ex-dao-fixed.json")).unwrap();
    run(cw_svg_pack::pack(&f.svg_template, &f.variables).unwrap()).unwrap();
}

#[test]
fn missing_packed_field_is_rejected() {
    let j = r#"{"name":"t","symbol":"T","collection_info_extension":{"seed":"AA==","svg_template":"<svg/>","variables":[],"template_slots":[],"price_tiers":[],"total":1,"mint_start_time":null,"mint_end_time":null,"whitelist":null},"minter":null,"creator":null,"withdraw_address":null}"#;
    assert!(from_json::<InstantiateMsg>(j).is_err());
}
