# `cw-ics08-wasm-dregg`

CosmWasm **B2-first** light client for **dregg federation** settlement wrap, shaped for ibc-go **08-wasm** integration.

Design: `docs/agentics/DESIGN-IBC-WASM-V2-DREGG-FEDERATION-LC.md`

## What it proves (honest)

Under a **pinned** `settlement_vk_hash` and genesis lanes, the client advances a **claim-limited** head:

- `proven_lanes` / `proven_root_key`
- cumulative `proven_height`

**Does not claim:** STARK re-verify, BFT finality, principal authorization, multi-cell conservation, dregg kernel parity.

## Features

| Feature | Meaning |
|---------|---------|
| `mock-verify` (default) | Continuity + structural checks for CI/demo; **not** pairing |
| (future) `groth16-verify` | Port `crates/dregg/cosmos-settlement` verifier |

## Dual-use

Smart-account authenticator **`terp-dregg`** queries this contract (or embeds a twin head) so SA txs can gate on federation settlement height/root without a second trust plane.

## Run tests

```bash
cd crates/terp-rs
cargo test -p cw-ics08-wasm-dregg
```
