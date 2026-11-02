# k19 lab (reference snapshot, not compiled)

Source from k19-lab worktree commit 35bd0e9 (terp-rs-headstash-membership-vk).
Files here are kept verbatim for reference and are NOT part of the zk-headstash
build: the k19 `secp256k1_chip.rs` wires a GLV fixed-base config
(`fixed_base.rs`) into the chip config, which changes the production circuit
and verifying key. Wiring them in needs a deliberate circuit/VK change.

- src/blind_k19.rs, src/lab_bench.rs, src/rawkey_browser/mod.rs: lab modules
- src/circuit/gadget/secp256k1_chip.rs + secp256k1_chip/fixed_base.rs: k19 chip
- src/lib.rs, Cargo.toml (browser-mt feature), justfile: k19 crate wiring
- Benchmarks: ../../../docs/k19-lab-*.{md,json,jsonl}, e2e/bench_wasm_rawkey*.mjs,
  scripts/build-rawkey-mt.sh
