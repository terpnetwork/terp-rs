//! Verify deployed whitelist-mtree (code 73 wasm, fetched from LCD) accepts a paymint tree.
//! usage: wl73 <root> <member> <proof_hex>   (from `cargo run --bin paymint_tree`)
use cosmwasm_vm::testing::{MockInstanceOptions, instantiate, mock_env, mock_instance_with_options, query};
use cosmwasm_vm::capabilities_from_csv;
use serde_json::{Value, json};
const CAPS: &str = "iterator,staking,stargate,cosmwasm_1_1,cosmwasm_1_2,cosmwasm_1_3,cosmwasm_1_4,cosmwasm_2_0,cosmwasm_2_1,cosmwasm_2_2,cosmwasm_3_0,zk,bn254,hash-blake,hash-poseidon,redpallas,ibc2,bulk_memory";
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let wasm = std::fs::read("wasm/whitelist_mtree_code73.wasm").unwrap();
    let mut inst = mock_instance_with_options(&wasm, MockInstanceOptions {
        available_capabilities: capabilities_from_csv(CAPS), gas_limit: 500_000_000_000_000, ..Default::default() });
    let admin = cosmwasm_vm::testing::MockApi::default().addr_make("admin");
    let info = cosmwasm_std::testing::message_info(&cosmwasm_std::Addr::unchecked(&admin), &[]);
    let r: cosmwasm_std::ContractResult<cosmwasm_std::Response> = instantiate(&mut inst, mock_env(), info,
        json!({"merkle_root": a[0], "merkle_tree_uri": null, "admins": [admin], "admins_mutable": true}));
    r.into_result().expect("instantiate");
    for (m, p) in [(a[1].clone(), vec![a[2].clone()]), (format!("{}x", a[1]), vec![a[2].clone()]), (a[1].clone(), vec![])] {
        let g0 = inst.create_gas_report().used_internally;
        let q: Value = serde_json::from_slice(&query(&mut inst, mock_env(), json!({"has_member":{"member": m, "proof_hashes": p}})).unwrap()).unwrap();
        println!("member={m} proof_len={} -> {q} (vm gas {})", p.len(), inst.create_gas_report().used_internally - g0);
    }
}
