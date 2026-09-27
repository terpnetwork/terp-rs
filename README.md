# Terp-RS: Standard Development Kit for Terp Network

Rust client library for Terp Network. Includes IBC asset derivation, CosmWasm contract deployment, test orchestration, schema-validated output generation, and language-agnostic protobuf definitions.

---

## Three layers (do not confuse)

| Layer | Package / path | Role |
|-------|----------------|------|
| **SDK** | `terp-rs` (`crates/sdk`) | protos + clients |
| **Scripts** | **`terp-scripts`** (`tests/`) | orchestration + bins + pure IBC lib |
| **Harness** | `ict-rs` (sibling) | Docker multi-chain framework |

## Seams + ZAP1 (system of record)

**Private DEX / bridge / corridor math:** only **`terp-seams`** (`crates/terp-seams`). Features: `dex`, `bridge`, `corridor`, `compose` (all default). There is no `private_dex_seams` crate. `crates/dex` is Juno/Astroport, not this product.

**ZAP1 08-wasm light client:** `contracts/light-clients/cw-ics08-wasm-zap1`.  
**ZAP1 PIR attestation module (not ICS-08):** `private-inference-rent/cw-zap1-ibcv2`.  
**Verify lib:** crates.io `zap1-verify` 0.2.1.  
**IAVL membership lib:** `contracts/light-clients/terp-iavl-lc` (used by `cw-ics08-wasm-terp`).

Agents: always load **`tests/agent/COMMANDS.md`** first.

---

## Workspace structure

```
crates/terp-rs/
├── Cargo.toml                  # Workspace root
├── crates/sdk/                 # terp-rs SDK crate
├── public/                     # Generated IBC artifacts (repo root)
│   ├── ibc-data/*.json
│   ├── assetlist.json
│   ├── ibc_routing_table.json
│   └── ibc_lookup_table.json
├── tests/                      # Package name: terp-scripts
│   ├── agent/COMMANDS.md       # Agent catalog (start here)
│   ├── src/ibc/                # Pure predict → observe → diff
│   ├── src/report.rs           # RunReport (JSON automation)
│   ├── bin/ibc_info.rs         # bin name: terp-ibc (alias: ibc)
│   ├── tests/
│   │   ├── ibc_unit.rs
│   │   ├── ibc_golden.rs
│   │   └── ibc_multihop_harness.rs  # #[ignore], Docker
│   └── README.md
├── docs/superpowers/           # Specs / plans / agent prompts
└── justfile                    # Proto gen + scripts-ibc-* + ci-* recipes
```

CI is tiered (see [`docs/ci.md`](docs/ci.md)): **Core** on every PR, **Extended** on path/label, **Heavy** (Docker) only via maintainer dispatch / label `ci-heavy` / weekly schedule.

---

## Quick Start

### Offline (no mainnet, preferred for agents)

```sh
cd crates/terp-rs
just ci-core                  # same gate as GitHub CI Core
just scripts-ibc-offline      # unit+golden + rebuild-from-public
just scripts-ibc-validate     # JSON RunReport
just scripts-ibc-preflight offline
```

Equivalent cargo:

```sh
cargo test -p terp-scripts --lib --test ibc_unit --test ibc_golden
cargo run -p terp-scripts --bin terp-ibc -- rebuild-from-public --format json
cargo run -p terp-scripts --bin terp-ibc -- validate --format json
```

### Live generate (needs MAIN_MNEMONIC + gRPC)

```sh
cargo run -p terp-scripts --bin terp-ibc -- preflight --mode live-query --format json   # exit 2 if env missing
cargo run -p terp-scripts --bin terp-ibc -- generate --mode live-query
```

### Docker multihop harness

```sh
just scripts-ibc-harness
# or: cargo test -p terp-scripts --test ibc_multihop_harness -- --ignored --nocapture
```

---

## IBC Asset Derivation

- **Live:** `generate` queries chains, writes `public/`, fail-closed on hard invariants.
- **Offline:** `rebuild-from-public` rebuilds routing/lookup from `public/ibc-data` (atomic staging).
- **Lib:** `terp_scripts::ibc` — pure hash, graph, routes, predict, observe, diff (`DiffReport` / `RunReport`).

Key design: pure derivation lives under `tests/src/ibc/`; the bin is orchestration + CLI contracts (`--format json`, preflight, atomic `--out`).

---

## TODO

- Unify and implement shims for replacing cosmrs/cosmos-sdk-proto libraries
- Wire blossom client traits for hashmerchant types (replaces manual URL format fetch keys)
- Spawn anvil + wire full hashmerchant production workflow
- Ensure relayer functionality: chain → argus → hash-merchant → nostr relay for dao-calendar events
- P1: rename package `terp-scripts` → `terp-scripts`, bin `ibc` → `terp-ibc` (see agentic automation design)
