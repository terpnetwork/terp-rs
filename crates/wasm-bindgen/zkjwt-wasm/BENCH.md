# zkjwt.passkey.p256.v1 bench

**host:** groot-wan (`groot.local`)  
**date:** 2026-08-16  
**command:** `cd crates/terp-rs/crates/wasm-bindgen/zkjwt-wasm && cargo run --release --offline --bin bench_zkjwt`  
**profile:** release (`opt-level=s`, LTO)  
**witness:** `dummy_witness()` valid P-256 ECDSA (same family as keygen)

| Metric | Value |
|--------|--------|
| k | 17 |
| keygen_ms (once) | 7080 |
| prove_ms (3 rounds) | 2837, 2776, 2804 |
| **prove_ms median** | **2804** |
| verify_ms (3 rounds) | 3, 2, 2 |
| **verify_ms median** | **2** |
| proof_bytes | 2304 |
| n_constraints | n/a (flex-gate) |
| config_params | `k: 17, num_advice_per_phase: [5], num_fixed: 1, num_lookup_advice_per_phase: [1, 0, 0], lookup_bits: Some(16), num_instance_columns: 1` |

G4 uses native SHPLONK + Blake2b transcript. Not a browser wasm-bindgen timing.
