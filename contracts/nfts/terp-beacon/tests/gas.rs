//! KV gas for terp-beacon transactions.
//!
//! Same meter as `contracts/nfts/raffle/tests/gas.rs`: the workspace
//! cw-multi-test fork reports `gas_used: 0`, so this charges the Cosmos SDK
//! KV schedule on the shared `MockStorage` (read 1000 + 3 per byte of key and
//! value, write 2000 + 30 per byte, delete 1000). The figure covers the whole
//! transaction, including bank and wasm-keeper writes. BLS verification CPU is
//! not metered here; wasmd charges it through the bls12_381 host functions.
//!
//! Run with `cargo test -p terp-beacon --features fixtures --test gas -- --nocapture`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cosmwasm_std::testing::MockStorage;
use cosmwasm_std::{HexBinary, Order, Record, Storage, Timestamp, coin};
use cw_multi_test::{AppBuilder, ContractWrapper, Executor};
use terp_beacon::fixtures::QUICKNET_ROUND;
use terp_beacon::msg::{BeaconResponse, ExecuteMsg, QueryMsg};
use terp_beacon::networks::QUICKNET;

struct GasMeter {
    inner: MockStorage,
    gas: Arc<AtomicU64>,
}

impl Storage for GasMeter {
    fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        let value = self.inner.get(key);
        let bytes = key.len() + value.as_ref().map(|v| v.len()).unwrap_or(0);
        self.charge(1_000 + 3 * bytes as u64);
        value
    }

    fn set(&mut self, key: &[u8], value: &[u8]) {
        self.charge(2_000 + 30 * (key.len() + value.len()) as u64);
        self.inner.set(key, value);
    }

    fn remove(&mut self, key: &[u8]) {
        self.charge(1_000);
        self.inner.remove(key);
    }

    fn range<'a>(
        &'a self,
        start: Option<&[u8]>,
        end: Option<&[u8]>,
        order: Order,
    ) -> Box<dyn Iterator<Item = Record> + 'a> {
        let gas = Arc::clone(&self.gas);
        Box::new(self.inner.range(start, end, order).map(move |(k, v)| {
            let bytes = (k.len() + v.len()) as u64;
            gas.fetch_add(1_000 + 3 * bytes, Ordering::Relaxed);
            (k, v)
        }))
    }
}

impl GasMeter {
    fn charge(&self, amount: u64) {
        self.gas.fetch_add(amount, Ordering::Relaxed);
    }
}

fn take(gas: &AtomicU64) -> u64 {
    gas.swap(0, Ordering::SeqCst)
}

#[test]
fn beacon_mock_gas_bench() {
    let counter = Arc::new(AtomicU64::new(0));
    let mut app = AppBuilder::new()
        .with_storage(GasMeter {
            inner: MockStorage::new(),
            gas: Arc::clone(&counter),
        })
        .build(|_, _, _| {});
    app.update_block(|b| {
        b.time = Timestamp::from_seconds(QUICKNET_ROUND.published_unix + 10);
    });

    let admin = app.api().addr_make("admin");
    let relayer = app.api().addr_make("relayer");
    let code = app.store_code(Box::new(
        ContractWrapper::new(
            terp_beacon::contract::execute,
            terp_beacon::contract::instantiate,
            terp_beacon::contract::query,
        )
        .with_reply(terp_beacon::contract::reply),
    ));

    let mut rows: Vec<(&str, u64)> = Vec::new();

    take(&counter);
    let beacon = app
        .instantiate_contract(
            code,
            admin.clone(),
            &QUICKNET.instantiate_msg(admin.as_str(), vec![], 0),
            &[],
            "terp-beacon",
            Some(admin.to_string()),
        )
        .unwrap();
    rows.push(("instantiate (quicknet)", take(&counter)));

    let attestor = HexBinary::from_hex(&format!("02{}", "ab".repeat(32))).unwrap();
    app.execute_contract(
        admin.clone(),
        beacon.clone(),
        &ExecuteMsg::AddAttestor { pubkey: attestor },
        &[],
    )
    .unwrap();
    rows.push(("add_attestor", take(&counter)));

    app.execute_contract(
        admin.clone(),
        beacon.clone(),
        &ExecuteMsg::UpdateConfig {
            required_attestations: Some(0),
            prices: Some(vec![coin(100, "uterp")]),
            callback_gas_limit: None,
        },
        &[],
    )
    .unwrap();
    rows.push(("update_config", take(&counter)));

    app.execute_contract(
        relayer.clone(),
        beacon.clone(),
        &ExecuteMsg::AddRound {
            round: QUICKNET_ROUND.round,
            signature: HexBinary::from_hex(QUICKNET_ROUND.signature_hex).unwrap(),
            previous_signature: None,
        },
        &[],
    )
    .unwrap();
    rows.push(("add_round (quicknet 33655511)", take(&counter)));

    let got: BeaconResponse = app
        .wrap()
        .query_wasm_smart(
            &beacon,
            &QueryMsg::Beacon {
                round: QUICKNET_ROUND.round,
            },
        )
        .unwrap();
    assert_eq!(got.randomness.to_hex(), QUICKNET_ROUND.randomness_hex);
    take(&counter);

    println!("| contract | action | kv gas |");
    println!("|---|---|---:|");
    for (action, gas) in &rows {
        assert!(*gas > 0, "{action} charged no gas");
        println!("| terp-beacon | {action} | {gas} |");
    }
}
