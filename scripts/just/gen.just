# scripts/just/gen.just
# Master generation and orchestration pipeline.
#
# Contract clients have one entry: `just gen api`.
# gen-tools reads tools/gen-tools/gen-tools.yaml and walks workspace members.
# A new contract needs a workspace membership and src/bin/schema.rs
# (or examples/schema.rs). There is no second list to edit.
# Module recipes run from scripts/just/.
set working-directory := '../..'

# Master generation pipeline (run after changes)
gen:
    just docs gen-docs
    just docs gen-tools
    just docs gen-just
    just sft sft-generate
    just skills sync-agents
    @echo "✅ Full generation pipeline complete (docs, tools, SFT, skills)"

# Full workspace sync (format, lib tests, generate).
sync:
    just core fmt
    just core test-rs
    just gen gen
    @echo "✅ Workspace fully synced and LLM-ready"

alias g := gen
alias s := sync

# Schema JSON plus TS, Python, Zod, Go, OpenAPI, proto, and Rust clients.
# Writes under api/<lang>/<contract>/.
api:
    cargo run -p gen-tools --offline -- --config tools/gen-tools/gen-tools.yaml --project terp-rs

# Rebuild clients from schema JSON already on disk.
api-clients:
    cargo run -p gen-tools --offline -- --config tools/gen-tools/gen-tools.yaml --project terp-rs --skip-schema

# Schema + TypeScript client for one nftfi gen-tools project (from nftfi/pipeline c028f38).
# Projects: marketplace, reserve-auction, raffle, nft-loan.
# Clients land in api/ts/<cargo-package>/{types,client,message-composer}/.
# One cargo at a time. The contract directory must already be a workspace member.
nftfi-schema project:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}"
    export CARGO_BUILD_JOBS=4
    export CARGO_TARGET_DIR="${PWD}/target"
    cd tools/gen-tools
    cargo run --release -- --config gen-tools.yaml --project "{{project}}" --steps schema,ts-codegen

# Sequential. Do not fan these out; the machine runs one cargo at a time.
nftfi-schema-all:
    just gen nftfi-schema marketplace
    just gen nftfi-schema reserve-auction
    just gen nftfi-schema raffle
    just gen nftfi-schema nft-loan

# Quick zk-wasm development cycle
zk-cycle module="terp-core":
    @echo "⚡ Running zk-wasm dev cycle for {{module}}..."
    @just test-{{module}} || echo "Module test skipped"
    @just build-{{module}} || echo "Module build skipped"
    @just gen --module={{module}}
    @echo "✅ zk-cycle complete for {{module}}"

# Rust dependency graph JSON: terp-rs workspace + CosmWasm, wasmvm, Wasmer,
# parity-wasm (roots in tools/gen-tools/rust-deps/roots.json; see its README).
# `just gen deps [out] [--root id=PATH --offline ...]`
deps out="target/gen-deps/rust-crates.json" *args="":
    node tools/gen-tools/rust-deps/gen-deps.mjs --out {{out}} {{args}}
