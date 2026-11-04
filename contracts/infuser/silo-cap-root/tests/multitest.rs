//! cw-multi-test: admin-only rotation, admin hand-over, queries, and the raw
//! storage layout silo-auth proves. The root fixture is computed the same way
//! as dregg silo-auth / silo-cap-tree (blake3 leaves, blake3 sorted pairs,
//! odd node promoted) from synthetic member strings.
use cosmwasm_std::{Addr, Empty};
use cw_multi_test::{App, Contract, ContractWrapper, Executor};
use silo_cap_root::*;

fn contract() -> Box<dyn Contract<Empty>> { Box::new(ContractWrapper::new(execute, instantiate, query)) }

fn node(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let (l, r) = if a <= b { (a, b) } else { (b, a) };
    let mut h = blake3::Hasher::new(); h.update(l); h.update(r); *h.finalize().as_bytes()
}
/// gen_merkle-format root (rs_merkle with the sorted-pair blake3 hasher).
fn root_of(members: &[&str]) -> String {
    let mut lvl: Vec<[u8; 32]> = members.iter().map(|m| *blake3::hash(m.as_bytes()).as_bytes()).collect();
    while lvl.len() > 1 { lvl = lvl.chunks(2).map(|c| if c.len() == 2 { node(&c[0], &c[1]) } else { c[0] }).collect(); }
    hex::encode(lvl[0])
}

fn setup(app: &mut App) -> (Addr, Addr, String) {
    let deployer = app.api().addr_make("deployer");
    let id = app.store_code(contract());
    let r = root_of(&["silo-cap/v1|a", "silo-cap/v1|b", "silo-admin/v1|c"]);
    let c = app.instantiate_contract(id, deployer.clone(), &InstantiateMsg { admin: None, merkle_root: r.clone(), metadata: Some("ipfs://tree-manifest".into()) }, &[], "silo-cap-root", None).unwrap();
    (deployer, c, r)
}

#[test]
fn query_and_rotate_by_admin_only() {
    let mut app = App::default();
    let (deployer, c, r) = setup(&mut app);
    let q: RootResponse = app.wrap().query_wasm_smart(&c, &QueryMsg::Root {}).unwrap();
    assert_eq!(q.root, r);
    let cfg: Config = app.wrap().query_wasm_smart(&c, &QueryMsg::Config {}).unwrap();
    assert_eq!(cfg.admin, deployer);
    assert_eq!(cfg.metadata.as_deref(), Some("ipfs://tree-manifest"));

    let eve = app.api().addr_make("eve");
    let r2 = root_of(&["silo-cap/v1|a", "silo-cap/v1|d"]);
    assert!(app.execute_contract(eve.clone(), c.clone(), &ExecuteMsg::UpdateRoot { root: r2.clone() }, &[]).is_err());
    assert!(app.execute_contract(deployer.clone(), c.clone(), &ExecuteMsg::UpdateRoot { root: "zz".into() }, &[]).is_err());
    app.execute_contract(deployer.clone(), c.clone(), &ExecuteMsg::UpdateRoot { root: r2.clone() }, &[]).unwrap();
    let q: RootResponse = app.wrap().query_wasm_smart(&c, &QueryMsg::Root {}).unwrap();
    assert_eq!(q.root, r2);
    // raw key layout proven by silo-auth: "merkle_root" -> JSON string
    assert_eq!(app.wrap().query_wasm_raw(&c, ROOT_KEY.as_bytes()).unwrap().unwrap(), format!("\"{r2}\"").into_bytes());
}

#[test]
fn admin_handover_e_g_to_a_dao() {
    let mut app = App::default();
    let (deployer, c, _) = setup(&mut app);
    let dao = app.api().addr_make("foundation-dao");
    assert!(app.execute_contract(dao.clone(), c.clone(), &ExecuteMsg::UpdateAdmin { admin: dao.to_string() }, &[]).is_err());
    assert!(app.execute_contract(deployer.clone(), c.clone(), &ExecuteMsg::UpdateAdmin { admin: "not-an-address".into() }, &[]).is_err());
    app.execute_contract(deployer.clone(), c.clone(), &ExecuteMsg::UpdateAdmin { admin: dao.to_string() }, &[]).unwrap();
    let r = root_of(&["x"]);
    assert!(app.execute_contract(deployer, c.clone(), &ExecuteMsg::UpdateRoot { root: r.clone() }, &[]).is_err());
    app.execute_contract(dao, c.clone(), &ExecuteMsg::UpdateRoot { root: r }, &[]).unwrap();
}

#[test]
fn explicit_admin_at_instantiate() {
    let mut app = App::default();
    let deployer = app.api().addr_make("deployer");
    let other = app.api().addr_make("other");
    let id = app.store_code(contract());
    let c = app.instantiate_contract(id, deployer, &InstantiateMsg { admin: Some(other.to_string()), merkle_root: root_of(&["a", "b"]), metadata: None }, &[], "l", None).unwrap();
    let cfg: Config = app.wrap().query_wasm_smart(&c, &QueryMsg::Config {}).unwrap();
    assert_eq!(cfg.admin, other);
}
