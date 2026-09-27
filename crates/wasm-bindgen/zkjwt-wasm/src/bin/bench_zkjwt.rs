//! Native bench for zkjwt.passkey.p256.v1. Run on groot-wan only:
//!   cargo run --release --offline --bin bench_zkjwt
use std::time::Instant;
use zkjwt_wasm::{
    circuit_config_debug, dummy_witness, prove_passkey, verify_passkey, warmup_keys, K,
};

fn median(mut v: Vec<u128>) -> u128 {
    v.sort_unstable();
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2
    }
}

fn main() {
    let rounds: usize = std::env::var("BENCH_ROUNDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    eprintln!("circuit_id=zkjwt.passkey.p256.v1 k={K} rounds={rounds}");

    let t0 = Instant::now();
    warmup_keys().expect("keygen");
    let keygen_ms = t0.elapsed().as_millis();
    println!("keygen_ms={keygen_ms} (once, process-cached)");
    match circuit_config_debug() {
        Ok(s) => println!("config_params={s}"),
        Err(e) => eprintln!("config_params_err={e}"),
    }

    let w = dummy_witness();
    let mut prove_ms = Vec::with_capacity(rounds);
    let mut verify_ms = Vec::with_capacity(rounds);
    let mut proof_bytes = 0usize;
    for i in 0..rounds {
        let t = Instant::now();
        let proof = prove_passkey(&w).expect("prove");
        let pms = t.elapsed().as_millis();
        proof_bytes = proof.len();
        prove_ms.push(pms);
        let t = Instant::now();
        assert!(verify_passkey(&proof, &w).expect("verify"));
        let vms = t.elapsed().as_millis();
        verify_ms.push(vms);
        println!("round={i} prove_ms={pms} verify_ms={vms} proof_bytes={proof_bytes}");
    }
    println!("---");
    println!("k={K}");
    println!("proof_bytes={proof_bytes}");
    println!("prove_ms_median={}", median(prove_ms));
    println!("verify_ms_median={}", median(verify_ms));
    println!("n_constraints=n/a (halo2-axiom flex-gate; see config_params)");
}
