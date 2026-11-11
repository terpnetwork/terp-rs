//! Gas baselines for the private auction on the **optimized wasm**, metered by
//! cosmwasm-vm (same method and columns as `cw-lc-zcash/tests/vm_gas.rs`). The payment
//! client is a querier stub serving the Confirmed inclusion from [`setup`].
//!
//! Run: `BENCH_WASM_DIR=<dir with svg_auction.wasm> BENCH_OUT=<file.md> cargo test
//! -p svg-auction --release --lib vm_gas -- --ignored --nocapture`
//!
//! `vm_gas` = CosmWasm gas (wasm + host imports: ed25519 FROST verify, blake2b).
//! `sdk_vm` = vm_gas / 140_000. `sdk_kv_w` = Σ(2000 + 30 × (|k|+|v|)) over keys written.
//! Reads, the payment client's own execution and tx ante are excluded: lower bound.
use super::*;
use cosmwasm_std::Order;
use cosmwasm_vm::testing::{
    MockApi as VmApi, MockInstanceOptions, MockQuerier as VmQuerier, MockStorage as VmStorage,
    execute as vm_execute, instantiate as vm_instantiate, mock_instance_with_options,
};
use cosmwasm_vm::{Instance, Storage};
use serde::Serialize;

type Inst = Instance<VmApi, VmStorage, VmQuerier>;
const CAPS: &str = "iterator,staking,stargate,bulk_memory,ibc2,hash_blake,hash_poseidon,bn254,redpallas,cosmwasm_1_1,cosmwasm_1_2,cosmwasm_1_3,cosmwasm_1_4,cosmwasm_2_0,cosmwasm_2_1,cosmwasm_2_2,cosmwasm_3_0";

fn kv(i: &mut Inst) -> BTreeMap<Vec<u8>, Vec<u8>> {
    i.with_storage(|s| {
        let id = s.scan(None, None, Order::Ascending).0.unwrap();
        Ok(s.all(id).0.unwrap().into_iter().collect())
    })
    .unwrap()
}
fn used(i: &mut Inst) -> u64 {
    let r = i.create_gas_report();
    r.used_internally + r.used_externally
}

#[derive(Default)]
struct Meter(Vec<(String, u64, usize, usize)>);
impl Meter {
    fn exec<M: Serialize>(&mut self, call: &str, i: &mut Inst, t: u64, who: &Addr, msg: &M) {
        let (k0, g0) = (kv(i), used(i));
        let r: ContractResult<Response> = if call == "instantiate" {
            vm_instantiate(i, env_at(t), message_info(who, &[]), msg)
        } else {
            vm_execute(i, env_at(t), message_info(who, &[]), msg)
        };
        r.into_result().unwrap_or_else(|e| panic!("{call}: {e}"));
        let (k1, g1) = (kv(i), used(i));
        let w: Vec<_> = k1.iter().filter(|(k, v)| k0.get(*k) != Some(*v)).collect();
        let bytes = w.iter().map(|(k, v)| k.len() + v.len()).sum();
        self.0.push((call.into(), g1 - g0, w.len(), bytes));
    }
    fn table(&self) -> String {
        let mut s = String::from(
            "| contract | call | vm_gas | sdk_vm | kv_w | kv_bytes | sdk_kv_w | sdk_est (lower bound) |\n|---|---|---:|---:|---:|---:|---:|---:|\n",
        );
        for (call, vm, n, b) in &self.0 {
            let (sdk_vm, sdk_kv) = (vm.div_ceil(140_000), *n as u64 * 2000 + *b as u64 * 30);
            s += &format!(
                "| svg-auction | {call} | {vm} | {sdk_vm} | {n} | {b} | {sdk_kv} | {} |\n",
                sdk_vm + sdk_kv
            );
        }
        s
    }
}

/// A fresh VM instance with one open private auction (#1), the payment client stubbed
/// from `w.lc`.
fn auction(m: &mut Meter, w: &W, wasm: &[u8], label: bool) -> Inst {
    let mut i = mock_instance_with_options(
        wasm,
        MockInstanceOptions {
            gas_limit: u64::MAX / 2,
            available_capabilities: cosmwasm_vm::capabilities_from_csv(CAPS),
            ..Default::default()
        },
    );
    let lc = w.lc.clone();
    let client = w.d.api.addr_make("zcash-lc").to_string();
    i.with_querier(|q| {
        q.update_wasm(move |r| match r {
            WasmQuery::Smart { contract_addr, msg } if *contract_addr == client => {
                let PaymentClientQuery::PaymentInclusion { app_data_hash_hex } =
                    from_json(msg).unwrap();
                let v = lc.borrow().get(&app_data_hash_hex).cloned();
                SystemResult::Ok(ContractResult::Ok(to_json_binary(&v).unwrap()))
            }
            _ => panic!("unexpected query {r:?}"),
        });
        Ok(())
    })
    .unwrap();
    let mut scratch = Meter::default();
    let m = if label { m } else { &mut scratch };
    let admin = w.d.api.addr_make("admin");
    m.exec(
        "instantiate",
        &mut i,
        T0,
        &admin,
        &InstantiateMsg {
            protocol_fee_bps: 250,
            min_increment_bps: 500,
            min_reserve_price: Uint128::new(100),
            duration: DUR,
            extension: 300,
            accepted_denom: vec!["uterp".into()],
            protocol_addr: w.d.api.addr_make("protocol").to_string(),
        },
    );
    let hook = Cw721HookMsg::CreatePrivateAuction(CreatePrivateAuction {
        reclaim_grace: Some(GRACE),
        seller_view_pk: HexBinary::from([5u8; 32].to_vec()),
        payment_client: w.d.api.addr_make("zcash-lc").to_string(),
    });
    m.exec(
        "ReceiveNft → CreatePrivateAuction",
        &mut i,
        T0,
        &w.nft,
        &ExecuteMsg::ReceiveNft(Cw721ReceiveMsg {
            sender: w.seller.to_string(),
            token_id: "orb-7".into(),
            msg: to_json_binary(&hook).unwrap(),
        }),
    );
    let fb = w
        .fin
        .sign(&dig(w, PrivateAction::BindFinalize, None, None), &[B, P, S])
        .unwrap();
    m.exec(
        "PlacePrivateBid (2 FROST bind sigs, 144 B envelope)",
        &mut i,
        T0 + 10,
        &w.bidder,
        &bid_msg(w, fb),
    );
    i
}

#[test]
#[ignore = "needs BENCH_WASM_DIR (optimized svg_auction.wasm)"]
fn private_auction_gas() {
    let dir = std::env::var("BENCH_WASM_DIR").expect("BENCH_WASM_DIR");
    let wasm = std::fs::read(format!("{dir}/svg_auction.wasm")).unwrap();
    let w = setup();
    let mut m = Meter::default();
    let relayer = w.d.api.addr_make("platform");

    // Win: 6/6 finalize gated on the Confirmed zap-1 payment, then optional OVK disclosure.
    let mut i = auction(&mut m, &w, &wasm, true);
    let rel = payment(&w, 1);
    let sig = w
        .fin
        .sign(&dig(&w, PrivateAction::Finalize, Some(1), Some(&rel)), &[B, P, S])
        .unwrap();
    m.exec(
        "FinalizePrivateAuction (6/6 FROST + PaymentInclusion query)",
        &mut i,
        T0 + DUR + 1,
        &relayer,
        &ExecuteMsg::FinalizePrivateAuction {
            auction_id: 1,
            bid_id: 1,
            release: rel,
            signature: sig,
            payment_ref: HexBinary::from(PAYREF.to_vec()),
        },
    );
    m.exec(
        "DisclosePrivateRelease",
        &mut i,
        T0 + DUR + 2,
        &relayer,
        &ExecuteMsg::DisclosePrivateRelease {
            auction_id: 1,
            bid_id: 1,
            ovk: HexBinary::from(OVK.to_vec()),
        },
    );

    // Cooperative reclaim (5/6 FROST, any time) and solo reclaim (bidder, after grace).
    let (rel, _) = release(&w, 3);
    let sig = w
        .reclaim
        .sign(&dig(&w, PrivateAction::Reclaim, Some(1), Some(&rel)), &[B, P])
        .unwrap();
    let mut i = auction(&mut m, &w, &wasm, false);
    m.exec(
        "ReclaimPrivateBid (5/6 FROST)",
        &mut i,
        T0 + 20,
        &relayer,
        &ExecuteMsg::ReclaimPrivateBid { auction_id: 1, bid_id: 1, release: rel.clone(), signature: Some(sig) },
    );
    let mut i = auction(&mut m, &w, &wasm, false);
    m.exec(
        "ReclaimPrivateBid (solo, after grace)",
        &mut i,
        T0 + DUR + GRACE,
        &w.bidder,
        &ExecuteMsg::ReclaimPrivateBid { auction_id: 1, bid_id: 1, release: rel, signature: None },
    );

    let t = m.table();
    println!("{t}");
    if let Ok(out) = std::env::var("BENCH_OUT") {
        std::fs::write(out, &t).unwrap();
    }
}
