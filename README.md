# terp-rs

CosmWasm contracts, orch suites, and the Rust SDK for Terp Network. This page
is an index. Each link below is the document that owns that topic.

## Start here

| Question | Where the answer lives |
|---|---|
| How a contract is compiled | [docs/COMPILING.md](docs/COMPILING.md), then [docs/wasm-artifacts.md](docs/wasm-artifacts.md) |
| How clients are generated | [tools/gen-tools/README.md](tools/gen-tools/README.md). Output is `api/`, which is gitignored |
| How to query a deployed contract from a browser | [web/](web/). Serve instructions are in that page |
| Which code ids and addresses cw-orch recorded | [public/state.json](public/state.json) |
| How the orch suites are run | [tests/suite/README.md](tests/suite/README.md) |
| How IBC derivation and `terp-scripts` are run | [tests/README.md](tests/README.md) |
| What CI runs | [docs/ci.md](docs/ci.md). Local recipes: `just --list` |
| Where chain protos and the SDK Rust types come from | [proto/README.md](proto/README.md) |
| Private bridge specs and the public pages | [docs/private-bridge/README.md](docs/private-bridge/README.md) |

Contracts live under `contracts/`. A contract's own schema and readme sit next
to its source. Optimized wasm is written to `artifacts/` by the compile docs
above; do not treat a host `cargo build --target wasm32-unknown-unknown` as
that artifact.

The SDK crate is `crates/sdk`. Its generated Rust is produced by the proto
package, not edited by hand.
