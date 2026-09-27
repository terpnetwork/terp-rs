# terp-ipa-params

No-op Halo2 circuit + deterministic **Vesta IPA** `Params::new(k)` for k = 1..=20.

This is a **terp-rs workspace** crate (`tests/suite/ipa-params`). It is not in the Halo2 fork.

```bash
cargo test -p terp-ipa-params -- --nocapture
# regenerate the full table (k=20 ≈ 64 MiB, minutes):
cargo test -p terp-ipa-params --release -- --ignored --nocapture
```

`SHA256SUMS` + `manifest.json` are the pin. `*.params.bin` are regenerated, not committed.

These blobs were produced with **zcash `halo2_proofs`** (`crates/zcash/halo2`). Headstash circuit now links zakura-common; recurate this table against zakura pasta before pinning those bytes in wasmvm.
