# Wasm artifacts

Contract wasm that this chain stores must pass `cosmwasm_vm::internals::compile_module` on Wasmer 7.4.2 singlepass, after the stack-height limiter and gas metering. A module that compiles under raw singlepass can still fail that path.

`crates/cw-infuser/artifacts/cw721_svg.wasm` (built 2026-09-21 with nightly rustc 1.98, `61d7280f3`, 2026-06-06) fails there with `Assembler failed finalization with: ImpossibleRelocation(Dynamic(DynamicLabel(0)))`. The same checker accepts `cw_svg_minter.wasm`, `cw721_base.wasm`, and `cw_infuser.wasm`. Do not publish wasm from that nightly.

## Compiler

Guest rustc is **1.91.0** with the `wasm32-unknown-unknown` target. That matches `crates/cosmwasm` `rust-version`.

Rust 1.86 cannot compile current `cosmwasm-std`: `u32::is_multiple_of` stabilized in 1.87. The optimizer image's contract compiler is rustup 1.91.0 in `tools/optimizer/Dockerfile`. Bob, the image's build helper, still compiles on the `rust:1.86.0-alpine` stage.

`cd tools/optimizer && make build-arm64` builds `terpnetwork/optimizer-arm64:0.17.0` on this Mac. `make build-amd64` builds `terpnetwork/optimizer:0.17.0`. The image keeps bulk-memory and disables guest SIMD. A host check, without that image, is:

```sh
cd crates/terp-rs
unset RUSTFLAGS
cargo +1.91.0 check --target wasm32-unknown-unknown --lib -p <package>
```

`unset RUSTFLAGS` lets `crates/terp-rs/.cargo/config.toml` apply. Those flags are for local checks only: `--allow-undefined` (wasmvm supplies `db_*`, `ed25519_*`, and the other host imports) and `-C target-feature=-simd128`.

Published artifacts use the optimizer flags instead:

```text
RUSTFLAGS="-C link-arg=-s -C target-feature=-simd128"
```

`-C link-arg=-s` strips symbols. `simd128` stays off because the chain gatekeeper rejects guest SIMD opcode `0xFD`.

Build scripts: `scripts/build/build-crosslink-wasm.sh`, `build-ics08-wasm-terp.sh`, `build-noncircuit-auth-wasm.sh`. They refuse `FORCE_HOST_WASM=1`. Mount the parent of `terp-rs` so `../cosmwasm` path dependencies resolve.

`tools/cw-optimizoor` is the no-Docker build of the same guest. It pins rustc 1.91.0, turns guest SIMD off, and runs binaryen without lowering bulk-memory. It only picks workspace members under a `contracts/` directory. The Docker image is the reproducible path.

## What the VM does to the module

`compile_module` (`crates/cosmwasm/packages/vm/src/wasm_backend/compile.rs`) deserializes with parity-wasm, injects the stack-height limiter, meters gas, then compiles with Wasmer 7.4.2 singlepass.

- Limiter crate: `nam-wasm-instrument` fork `0.5.3-terp.2` (git rev `248ac2796903fe39977f31cb9bdef54c77ccf7a0`), features `sign_ext`, `bulk`, `simd`.
- `MAX_WASM_STACK_HEIGHT` is 4096.
- `memory.copy` is allowed. Its metered base cost is 4_500_000 gas. Do not strip bulk-memory for this wasmvm. `wasm-opt` on these artifacts keeps it (`--enable-bulk-memory`). The stock `cosmwasm/optimizer:0.17.0` wasm-opt is version 116 and cannot `--llvm-memory-copy-fill-lowering`.
- The accept test is `packages/check` (`cosmwasm_vm::internals::compile_module`), not raw singlepass.

Wasmer 7.4.2 on `zk-cosmwasm` is host-only (`cfg(not(target_arch = "wasm32"))`). `wasmer-types` 7.4.2 is edition 2024, `rust-version` 1.95, so the guest compile must not build it. `cw-orch` is the same: host target only.

## Resolver constraints

Cargo keeps one version of a crate per semver-compatible range. A lock refresh resolves optional features of workspace members, so a contract-only `cargo check` still has to select those versions.

`wasmer-types` 7.4.2 requires `sha2 ^0.11.0`. `bip32` `=0.6.0-pre.1` requires `sha2 =0.11.0-pre.4`. Those ranges do not overlap, and both are 0.11, so resolution fails. `zcash_script` before 0.6 pins that same bip32 pre-release. `crypto-common =0.2.0-rc.1` fails the same way against `digest` 0.11's `crypto-common ^0.2`.

`crates/librustzcash` now depends on `bip32` 0.6 and `zcash_script` 0.6. The unused `block-buffer` / `crypto-common` exact pins are gone from `zcash_primitives`. That is the same pin move as [zakura-core/common#538](https://github.com/zakura-core/common/pull/538). The PR also moves `secp256k1` to 0.33 and drops context parameters on the transparent API. This tree still pins `secp256k1` 0.29. Versions resolve together (0.29 and 0.33 are different 0.x minors). A host build of the transparent signer still needs that API migration. The wasm32 contract check does not compile it.

`sha2` 0.10 (workspace pin `0.10.8` for contracts) and `sha2` 0.11 (Wasmer and bip32 0.6) coexist. They are different minors.

Do not edit the `cw-infuser` lock to chase this. That workspace is separate.

## Workspace packages that typecheck for wasm32

`cargo +1.91.0 check --target wasm32-unknown-unknown --lib` finished for:

`cw-private-dex`, `cw-ics08-wasm-crosslink`, `cw-ics08-wasm-dregg`, `cw-ics08-wasm-zap1`, `cw-ics08-wasm-terp`, `polytone-any-proxy`, `cw-zk-verify-demo`, `terp-dregg`, `terp-zkjwt`, `terp-vsck`, `terp-passkey`, `terp-recovery`, `terp-ed25519`, `terp-eth`, `terp-frost`, `terp-iavl-lc` (library, linked into the ics08 guest), `terp-zktls`, `terp-bls`, `terp-dao`.

`cargo +1.91.0 build --release --target wasm32-unknown-unknown --lib` wrote cdylibs under `target/wasm32-unknown-unknown/release/`. They are not copied to `artifacts/` and have not been passed through `compile_module`.

## Not in that check

- `contracts/revenue/auction` and `contracts/revenue/residual-registry` use `workspace = true` but are not members. `cw721` and `cw721-base` are not in this tree. Auction also asks for `cosmwasm-storage` and `cw2981-royalties`.
- `contracts/smart-accounts/terp-zkbn254` has no `[lib]` and no dependencies.
- Isolated workspaces (their own `[workspace]`, not the terp-rs lock) also typecheck with `cargo +1.91.0 check --target wasm32-unknown-unknown --lib`: `loyalty-verifier`, `trusted-builder-sbt`, `marketplace-mirror`, `cw-cashu-registry`. `marketplace-mirror` and `cw-cashu-registry` resolve crates.io `cosmwasm-std` 2.3.4, not this fork. `trusted-builder-sbt` uses the path `cosmwasm-std`. `loyalty-verifier` uses stable `sha2` 0.11, not `0.11.0-pre.4`.
