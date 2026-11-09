# Light clients (CosmWasm 08-wasm)

| Crate | Role |
|-------|------|
| `cw-ics08-wasm-crosslink` | Zcash Crosslink trailing finality |
| `cw-ics08-wasm-dregg` | Dregg federation B2 settlement wrap |
| `cw-ics08-wasm-zap1` | ZAP1 Merkle + IBC v2 packet attestation |
| `cw-ics08-wasm-terp` | Terp hybrid IAVL (SHA-256 IBC + BLAKE3 app stores) |
| `terp-iavl-lc` | ICS-23 membership (HashOp → spec). Feature `bench` for DIY IAVL + wrong-alg check |

**Wasm output:** always `crates/terp-rs/artifacts/*.wasm` — never `contracts/**/artifacts/`.

`cw-ics08-wasm-terp` is stored with **`tx ibc-wasm store-code`**, not x/wasm. Build via `./scripts/build-ics08-wasm-terp.sh` (Docker optimizer, rustc 1.86).
