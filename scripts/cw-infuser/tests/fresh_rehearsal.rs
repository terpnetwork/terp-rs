//! Mock rehearsal of the whole `svg_fresh_plan` step list (codes 98/99 + whitelist), with
//! mock addresses standing in for the TBD roles. `cargo test -p cw-infuser-scripts --test fresh_rehearsal`
use cosmwasm_std::coin;
use cw_infuser_scripts::collection::SvgCollectionSuite;
use cw_infuser_scripts::fresh_plan::*;
use cw_orch::prelude::*;

mod wl {
    use cosmwasm_std::*;
    pub fn instantiate(d: DepsMut, _: Env, _: MessageInfo, m: whitelist_mtree::msg::InstantiateMsg) -> StdResult<Response> {
        d.storage.set(b"r", m.merkle_root.as_bytes()); Ok(Response::new())
    }
    pub fn execute(_: DepsMut, _: Env, _: MessageInfo, _: Empty) -> StdResult<Response> { Ok(Response::new()) }
    pub fn query(d: Deps, _: Env, m: whitelist_mtree::msg::QueryMsg) -> StdResult<Binary> {
        match m {
            whitelist_mtree::msg::QueryMsg::HasMember { member, proof_hashes } => {
                let mut cur = *blake3::hash(member.as_bytes()).as_bytes();
                for p in proof_hashes {
                    let sib: [u8; 32] = hex::decode(&p).map_err(|_| StdError::msg("bad proof"))?.try_into().map_err(|_| StdError::msg("bad proof"))?;
                    let mut pr = [cur, sib]; pr.sort_unstable();
                    let mut b = [0u8; 64]; b[..32].copy_from_slice(&pr[0]); b[32..].copy_from_slice(&pr[1]);
                    cur = *blake3::hash(&b).as_bytes();
                }
                to_json_binary(&whitelist_mtree::msg::HasMemberResponse { has_member: d.storage.get(b"r").unwrap_or_default() == hex::encode(cur).into_bytes() })
            }
            _ => Err(StdError::msg("unsupported")),
        }
    }
}


#[test]
fn rehearse_fresh_plan() {
    let chain = MockBech32::new("terp");
    let op = chain.sender_addr();
    let r = Roles { fractal_curator: chain.addr_make("fcur").to_string(), logos_curator: chain.addr_make("lcur").to_string(),
        payment_minter: PAYMENT_MINTER.to_string(), withdraw: chain.addr_make("dao").to_string(), operator: op.to_string(), salt: PAYMINT_SALT.into() };
    assert_eq!(cw_infuser_scripts::paymint::paymint_tree(PAYMENT_MINTER, 10_000, PAYMINT_SALT).root, PAYMINT_ROOT);
    r.require_real().unwrap();
    Roles::default().require_real().unwrap(); // curators = Zigner granter
    assert!(Roles { logos_curator: LOGOS_CURATOR_TBD.into(), ..Default::default() }.require_real().is_err());
    let buyer = chain.addr_make("buyer");
    chain.add_balance(&buyer, vec![coin(10_000_000_000, "uthiol")]).unwrap();
    let suite = SvgCollectionSuite::new(chain.clone());
    let f = suite.deploy_factory(Some(&op), &op).unwrap();
    let wl_code = chain.upload_custom("wl", Box::new(cw_orch::mock::cw_multi_test::ContractWrapper::new(wl::execute, wl::instantiate, wl::query))).unwrap().uploaded_code_id().unwrap();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("svgs/interchain-packed");

    // step 1 (msg from the plan), then rebuild the plan with the real whitelist address
    let s0 = plan(&r, &dir, "x", 42).unwrap();
    let wl_msg: whitelist_mtree::msg::InstantiateMsg = serde_json::from_value(s0[0].msg.clone()).unwrap();
    let wl_addr = chain.call_as(&Addr::unchecked(&s0[0].signer)).instantiate(wl_code, &wl_msg, s0[0].label.as_deref(), Some(&op), &[]).unwrap().instantiated_contract_address().unwrap();
    let steps = plan(&r, &dir, wl_addr.as_str(), 42).unwrap();
    assert_eq!(steps.len(), 17);

    let mut colls = vec![];
    for s in steps.iter().filter(|s| s.contract.as_deref() == Some(FACTORY)) {
        let m: cw_svg_minter::msg::ExecuteMsg = serde_json::from_value(s.msg.clone()).unwrap();
        let res = f.minter.call_as(&Addr::unchecked(&s.signer)).execute(&m, &[]).unwrap_or_else(|e| panic!("{}: {e}", s.id));
        if let Ok(a) = res.event_attr_value("wasm", "contract") { let k = s.msg.as_object().unwrap().values().next().unwrap(); colls.push((s.id.clone(), Addr::unchecked(a), k["instantiate_msg"]["collection_info_extension"]["price_tiers"][0]["price"].clone())); }
    }
    assert_eq!(colls.len(), 14, "fractal + 13 logos");

    // every collection: wasm admin = its curator; a paid mint works; renders
    for (id, a, p) in &colls {
        let info = chain.wasm_querier().contract_info(a).unwrap();
        let want = if id.starts_with('2') { &r.fractal_curator } else { &r.logos_curator };
        assert_eq!(info.admin.map(|a| a.to_string()), Some(want.clone()), "{id}");
        let c = suite.at(a).clone();
        let funds: Vec<cosmwasm_std::Coin> = if p.is_null() { vec![] } else { vec![serde_json::from_value(p.clone()).unwrap()] };
        if let Some(cf) = funds.first() { if cf.denom != "uthiol" { chain.add_balance(&buyer, vec![cf.clone()]).unwrap(); } }
        if !id.starts_with('2') {
            c.call_as(&buyer).execute(&cw721_svg::ExecuteMsg::UpdateExtension { msg: cw721_svg::SvgExecuteMsgExt::Mint(cw_svg::MintMsg { alloc: 0, amnt: 1, proof_hashes: vec![], ext: None }) }, &funds).unwrap_or_else(|e| panic!("{id} paid mint: {e}"));
            let _: serde_json::Value = c.query(&cw721_svg::QueryMsg::Extension { msg: cw721_svg::SvgQueryMsgExt::SvgTokenUri { token_id: "0".into() } }).unwrap();
        }
    }

    // step 5 on the fractal: payment minter mints free, transfers to buyer
    let fr = suite.at(&colls[0].1).clone();
    let pm = Addr::unchecked(&r.payment_minter);
    let m5: cw721_svg::ExecuteMsg = serde_json::from_value(steps[16].msg.clone()).unwrap();
    fr.call_as(&pm).execute(&m5, &[]).unwrap();
    fr.call_as(&pm).execute(&cw721_svg::ExecuteMsg::TransferNft { recipient: buyer.to_string(), token_id: "0".into() }, &[]).unwrap();
    let r: serde_json::Value = fr.query(&cw721_svg::QueryMsg::Extension { msg: cw721_svg::SvgQueryMsgExt::SvgTokenUri { token_id: "0".into() } }).unwrap();
    assert!(r["svg"].as_str().unwrap().contains("<animateTransform"));
}
