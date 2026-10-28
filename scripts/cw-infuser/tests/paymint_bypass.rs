//! Fee bypass for the off-chain payment minter on the deployed code (98/99 sources):
//! dedicated whitelist-mtree (code 73) + collection `whitelist`, no contract change.
//! `cargo test -p cw-infuser-scripts --test paymint_bypass`
use cosmwasm_std::{Binary, coin};
use cw_infuser_scripts::collection::SvgCollectionSuite;
use cw_infuser_scripts::paymint::*;
use cw_orch::prelude::*;

// MockApi has no blake3_256, so stand in for whitelist-mtree (code 73) with the same
// query_has_member algorithm (contracts/infuser/whitelist-merkletree/src/contract.rs:120).
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

const T: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\"><text>${a}</text></svg>";

#[test]
fn payment_minter_mints_free_then_transfers() {
    let chain = MockBech32::new("terp");
    let owner = chain.sender_addr();
    let curator = chain.addr_make("curator");
    let pm = chain.addr_make("payminter");
    let buyer = chain.addr_make("buyer");
    chain.add_balance(&buyer, vec![coin(1_000_000_000, "uthiol")]).unwrap();
    let suite = SvgCollectionSuite::new(chain.clone());
    let f = suite.deploy_factory(Some(&owner), &owner).unwrap();

    let tree = paymint_tree(pm.as_str(), 10_000, "fractal");
    let wl_code = chain.upload_custom("wl", Box::new(cw_orch::mock::cw_multi_test::ContractWrapper::new(wl::execute, wl::instantiate, wl::query))).unwrap().uploaded_code_id().unwrap();
    let wl_addr = chain.instantiate(wl_code, &whitelist_init(&tree, owner.as_str()), Some("wl"), Some(&owner), &[]).unwrap().instantiated_contract_address().unwrap();

    let vars = vec![cw_svg::VariableDef { name: "a".into(), kind: cw_svg::VariableKind::Options(vec!["x".into(), "y".into()]) }];
    let init = cw721_svg::InstantiateMsg {
        name: "f".into(), symbol: "F".into(), minter: None,
        creator: Some(curator.to_string()), withdraw_address: Some(owner.to_string()),
        collection_info_extension: cw721_svg::SvgCollectionMetadata {
            seed: Binary::from([7u8; 32].to_vec()),
            packed: cw721_svg::pack_svg(T, &vars),
            price_tiers: vec![cw721_svg::PriceTier { until_count: 10_000, price: coin(100_000_000, "uthiol") }],
            total: 10_000, mint_start_time: None, mint_end_time: None,
            whitelist: Some(wl_addr.to_string()),
            require_auth: false,
        },
    };
    let res = f.minter.execute(&cw_svg_minter::msg::ExecuteMsg::CreateSvgCollection { instantiate_msg: init, label: "f".into() }, &[]).unwrap();
    let coll = Addr::unchecked(res.event_attr_value("wasm", "contract").unwrap());
    let c = suite.at(&coll).clone();

    // payment minter: fee-free mint of 2, no funds
    let r = c.call_as(&pm).execute(&paymint_msg(&tree, 2), &[]).unwrap();
    assert!(r.events.iter().all(|e| e.ty != "transfer"), "no bank transfer on bypass");
    // hand token 0 to the buyer
    c.call_as(&pm).execute(&cw721_svg::ExecuteMsg::TransferNft { recipient: buyer.to_string(), token_id: "0".into() }, &[]).unwrap();
    let o: cw721::msg::OwnerOfResponse = c.query(&cw721_svg::QueryMsg::OwnerOf { token_id: "0".into(), include_expired: None }).unwrap();
    assert_eq!(o.owner, buyer.to_string());
    assert_eq!(chain.query_balance(&pm, "uthiol").unwrap().is_zero(), true);

    // buyer cannot use the payment minter's proof, and pays for a normal mint
    assert!(c.call_as(&buyer).execute(&paymint_msg(&tree, 1), &[]).is_err());
    assert!(c.call_as(&buyer).execute(&cw721_svg::ExecuteMsg::UpdateExtension { msg: cw721_svg::SvgExecuteMsgExt::Mint(cw_svg::MintMsg { alloc: 0, amnt: 1, proof_hashes: vec![], ext: None }) }, &[]).is_err());
    c.call_as(&buyer).execute(&cw721_svg::ExecuteMsg::UpdateExtension { msg: cw721_svg::SvgExecuteMsgExt::Mint(cw_svg::MintMsg { alloc: 0, amnt: 1, proof_hashes: vec![], ext: None }) }, &[coin(100_000_000, "uthiol")]).unwrap();
    // alloc cap
    let small = paymint_tree(pm.as_str(), 10_000, "fractal");
    assert!(c.call_as(&pm).execute(&paymint_msg(&small, 9_999), &[]).is_err(), "alloc cap");
}

#[test]
fn placeholders_refuse() {
    assert!(require_real(&[("payment_minter", PAYMENT_MINTER_TBD)]).is_err());
    assert!(require_real(&[("x", "terp1abc")]).is_ok());
}
