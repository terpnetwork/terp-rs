//! VM-gas bench: legacy cw721-svg (code 71, fetched from chain) vs packed cw721-svg
//! (code 98 = artifacts/cw721_svg.wasm) on the real wasm, via cosmwasm-vm testing.
//! Reports CosmWasm VM gas (internal + external) and an SDK-gas estimate:
//! VM gas / 140_000 (wasmd DefaultGasMultiplier) + KV write cost (2000 + 30/byte).
//! This is NOT chain gas: it omits ante/tx-size/signature costs and wasmd's fixed
//! instance cost. Also writes one rendered token SVG per (logo, version) to out/.
use cosmwasm_std::{Coin, Empty, Order, coins};
use cosmwasm_vm::testing::{
    MockInstanceOptions, MockStorage, execute, instantiate, mock_env, mock_instance_with_options, query,
};
use cosmwasm_vm::{Storage, capabilities_from_csv};
use serde_json::{Value, json};
use std::fs;

const CAPS: &str = "iterator,staking,stargate,cosmwasm_1_1,cosmwasm_1_2,cosmwasm_1_3,cosmwasm_1_4,cosmwasm_2_0,cosmwasm_2_1,cosmwasm_2_2,cosmwasm_3_0,zk,bn254,hash-blake,hash-poseidon,redpallas,ibc2,bulk_memory";
/// wasmd DefaultGasMultiplier (CosmWasm gas per 1 SDK gas).
const MULT: u64 = 140_000;
/// SDK KV costs (store/types/gas.go defaults).
fn kv_write_gas(k: usize, v: usize) -> u64 { 2000 + 30 * (k + v) as u64 }

fn kv_dump(s: &mut MockStorage) -> Vec<(Vec<u8>, Vec<u8>)> {
    let id = s.scan(None, None, Order::Ascending).0.unwrap();
    let mut out = vec![];
    while let (Ok(Some(r)), _) = s.next(id) { out.push(r); }
    out
}

struct Run { inst_vm: u64, inst_ext: u64, inst_kv: u64, kv_bytes: usize, mint_vm: u64, mint_ext: u64, mint_kv: u64, render_vm: u64, svg: String }

fn run(wasm: &[u8], mut init: Value, price: Vec<Coin>) -> Result<Run, String> {
    let creator = cosmwasm_vm::testing::MockApi::default().addr_make("creator");
    let user = cosmwasm_vm::testing::MockApi::default().addr_make("user");
    init["creator"] = json!(creator);
    init["withdraw_address"] = json!(creator);
    init["minter"] = Value::Null;
    let mut inst = mock_instance_with_options(wasm, MockInstanceOptions {
        available_capabilities: capabilities_from_csv(CAPS), gas_limit: 500_000_000_000_000, ..Default::default() });
    let env = mock_env();
    let g0 = inst.create_gas_report();
    let info = cosmwasm_std::testing::message_info(&cosmwasm_std::Addr::unchecked(&creator), &[]);
    let r: cosmwasm_std::ContractResult<cosmwasm_std::Response<Empty>> = instantiate(&mut inst, env.clone(), info, init);
    r.into_result().map_err(|e| format!("instantiate: {e}"))?;
    let g1 = inst.create_gas_report();
    let kv0 = inst.with_storage(|s| Ok(kv_dump(s))).unwrap();
    let inst_kv: u64 = kv0.iter().map(|(k, v)| kv_write_gas(k.len(), v.len())).sum();
    let kv_bytes = kv0.iter().map(|(k, v)| k.len() + v.len()).sum();
    // mint 1 (try with and without the newer `ext` field)
    let mut mint_ok = false; let mut last = String::new();
    for m in [json!({"update_extension":{"msg":{"mint":{"amnt":1,"proof_hashes":[],"alloc":0,"ext":null}}}}),
              json!({"update_extension":{"msg":{"mint":{"amnt":1,"proof_hashes":[],"alloc":0}}}})] {
        let info = cosmwasm_std::testing::message_info(&cosmwasm_std::Addr::unchecked(&user), &price);
        let r: cosmwasm_std::ContractResult<cosmwasm_std::Response<Empty>> = execute(&mut inst, env.clone(), info, m);
        match r.into_result() { Ok(_) => { mint_ok = true; break } Err(e) => last = e }
    }
    if !mint_ok { return Err(format!("mint: {last}")) }
    let g2 = inst.create_gas_report();
    let kv1 = inst.with_storage(|s| Ok(kv_dump(s))).unwrap();
    let mint_kv: u64 = kv1.iter().filter(|kv| !kv0.contains(kv)).map(|(k, v)| kv_write_gas(k.len(), v.len())).sum();
    let toks: Value = serde_json::from_slice(&query(&mut inst, env.clone(), json!({"all_tokens":{}})).unwrap()).unwrap();
    let tid = toks["tokens"][0].as_str().unwrap_or("1").to_string();
    let g3 = inst.create_gas_report();
    let q = query(&mut inst, env, json!({"extension":{"msg":{"svg_token_uri":{"token_id":tid}}}}))
        .into_result().map_err(|e| format!("render: {e}"))?;
    let g4 = inst.create_gas_report();
    let v: Value = serde_json::from_slice(&q).unwrap();
    let svg = v.get("svg").or(v.get("token_uri")).and_then(|x| x.as_str()).map(str::to_string).unwrap_or_else(|| v.to_string());
    let d = |a: &cosmwasm_vm::GasReport, b: &cosmwasm_vm::GasReport| (b.used_internally - a.used_internally, b.used_externally - a.used_externally);
    let (iv, ie) = d(&g0, &g1); let (mv, me) = d(&g1, &g2); let (rv, _) = d(&g3, &g4);
    Ok(Run { inst_vm: iv, inst_ext: ie, inst_kv, kv_bytes, mint_vm: mv, mint_ext: me, mint_kv, render_vm: rv, svg })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let logos: Vec<String> = if args.is_empty() { vec!["terp".into(), "akash".into(), "penumbra".into()] } else { args };
    let old_wasm = fs::read("wasm/cw721_svg_code71.wasm").expect("run ./fetch-wasm.sh first");
    let new_wasm = fs::read("../../../artifacts/cw721_svg.wasm").expect("artifacts/cw721_svg.wasm");
    fs::create_dir_all("out/rendered").unwrap();
    let mut rows = vec![];
    println!("| logo | ver | init msg B | inst VM gas | inst SDK est | KV B stored | mint VM gas | mint SDK est | render VM gas |");
    println!("|---|---|---|---|---|---|---|---|---|");
    for logo in &logos {
        for (ver, wasm, path) in [("code71", &old_wasm, format!("../svgs/interchain/{logo}/init.json")),
                                  ("code98", &new_wasm, format!("../svgs/interchain-packed/{logo}/init.json"))] {
            let mut init: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            let ext = &mut init["collection_info_extension"];
            let price: Vec<Coin> = ext["price_tiers"][0]["price"].as_object()
                .map(|p| coins(p["amount"].as_str().unwrap().parse().unwrap(), p["denom"].as_str().unwrap())).unwrap_or_default();
            ext["mint_start_time"] = Value::Null; ext["mint_end_time"] = Value::Null; ext["whitelist"] = Value::Null;
            let msg_b = serde_json::to_vec(&init).unwrap().len();
            match run(wasm, init, price) {
                Ok(r) => {
                    let sdk_i = (r.inst_vm + r.inst_ext) / MULT + r.inst_kv;
                    let sdk_m = (r.mint_vm + r.mint_ext) / MULT + r.mint_kv;
                    println!("| {logo} | {ver} | {msg_b} | {} | {sdk_i} | {} | {} | {sdk_m} | {} |", r.inst_vm + r.inst_ext, r.kv_bytes, r.mint_vm + r.mint_ext, r.render_vm);
                    fs::write(format!("out/rendered/{logo}.{ver}.svg"), &r.svg).unwrap();
                    rows.push(json!({"logo":logo,"ver":ver,"init_msg_bytes":msg_b,"instantiate_vm_gas":r.inst_vm+r.inst_ext,"instantiate_kv_write_gas":r.inst_kv,"instantiate_sdk_gas_est":sdk_i,"kv_bytes_after_instantiate":r.kv_bytes,"mint_vm_gas":r.mint_vm+r.mint_ext,"mint_kv_write_gas":r.mint_kv,"mint_sdk_gas_est":sdk_m,"render_query_vm_gas":r.render_vm}));
                }
                Err(e) => { println!("| {logo} | {ver} | {msg_b} | ERR {e} |"); rows.push(json!({"logo":logo,"ver":ver,"error":e})); }
            }
        }
    }
    fs::write("out/gas.json", serde_json::to_vec_pretty(&rows).unwrap()).unwrap();
}
