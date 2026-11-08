# Wasm artifacts

Contract wasm that this chain stores must pass `cosmwasm_vm::internals::compile_module` on Wasmer 7.4.2 singlepass, after the stack-height limiter and gas metering. A module that compiles under raw singlepass can still fail that path.

`crates/cw-infuser/artifacts/cw721_svg.wasm` (built 2026-09-21 with nightly rustc 1.98, `61d7280f3`, 2026-06-06) fails there with `Assembler failed finalization with: ImpossibleRelocation(Dynamic(DynamicLabel(0)))`. The same checker accepted `cw721_base.wasm` and `cw_infuser.wasm`. `cw_svg_minter.wasm` from the current tree also fails on arm64, see [Arch scan](#arch-scan). Do not publish wasm from that nightly.

## Compiler

Guest rustc is **1.97.1** with the `wasm32-unknown-unknown` target. That is the zakura-common toolchain pin. `crates/cosmwasm` `rust-version` is 1.91.0 and still builds on it.

Rust 1.86 cannot compile current `cosmwasm-std`: `u32::is_multiple_of` stabilized in 1.87. The optimizer image's contract compiler is rustup 1.97.1 in `tools/optimizer/Dockerfile`. Bob, the image's build helper, still compiles on the `rust:1.86.0-alpine` stage. That does not affect guest output. The image's `RUST_VERSION` env says 1.97.1. The base image's stale 1.86.0 value is overridden.

`cd tools/optimizer && make build-arm64` builds `terpnetwork/optimizer-arm64:0.17.0` on this Mac. `make build-amd64` builds `terpnetwork/optimizer:0.17.0`. The image keeps bulk-memory and disables guest SIMD. A host check, without that image, is:

```sh
cd crates/terp-rs
unset RUSTFLAGS
cargo +1.97.1 check --target wasm32-unknown-unknown --lib -p <package>
```

`unset RUSTFLAGS` lets `crates/terp-rs/.cargo/config.toml` apply. The image sets the same two flags: `--allow-undefined` (wasmvm supplies `db_*`, `ed25519_*`, and the other host imports) and `-C target-feature=-simd128`.

Published artifacts use the optimizer flags instead:

```text
RUSTFLAGS="-C link-arg=-s -C link-arg=--allow-undefined -C target-feature=-simd128 --cfg getrandom_backend=\"unsupported\""
```

`-C link-arg=-s` strips symbols. `simd128` stays off because the chain gatekeeper rejects guest SIMD opcode `0xFD`.

Build scripts: `scripts/build/build-crosslink-wasm.sh`, `build-ics08-wasm-terp.sh`, `build-noncircuit-auth-wasm.sh`. They refuse `FORCE_HOST_WASM=1`. Mount the parent of `terp-rs` so `../cosmwasm` path dependencies resolve.

## `just core optimize`

```sh
cd tools/optimizer && make build-arm64-local   # terpnetwork/optimizer-arm64:0.17.0-rust1.97, local only, no push target
cd crates/terp-rs && just core optimize
```

The recipe mounts `crates/` at `/Users/returniflost/abstract/terp-core/crates` and this checkout at `terp-rs-consolidate` under that prefix. It uses the `terp-rs_cache` target volume. Set `OPTIMIZE_TARGET_VOLUME=<name>` to build from a fresh volume. The amd64 tag is `terpnetwork/optimizer:0.17.0-rust1.97` (`make build-amd64-local`).

Bob builds only workspace members whose `[lib] crate-type` contains `cdylib`. It logs `Skipping ...: no cdylib lib target` for the others (`contracts/**/e2e`, packages, test helpers). The full workspace builds 52 contracts and skips 77 crates.

## Chain wasm runtime

- `go.mod`: `github.com/CosmWasm/wasmvm/v3 v3.0.7` is replaced by the in-tree fork `./x/wasm/wasmvm` (libwasmvm 4.1.0, host rust-version 1.96). `crates/cosmwasm/packages/vm` pins wasmer `=7.4.2` singlepass.
- Capabilities: `wasmkeeper.BuiltInCapabilities()` plus `cosmwasm_3_0`.
- Gatekeeper allows floats, bulk-memory and sign-ext. It rejects SIMD, threads, exceptions and reference-types instructions (the type subset that rustc ≥1.82 emits is accepted).
- Guest check: `wasm-tools validate --features=mvp,bulk-memory,sign-extension,mutable-global,saturating-float-to-int <file>`.

The host toolchain and the guest toolchain are deliberately different. `terp-rs` has no `rust-toolchain` file because host builds need ≥1.95 (wasmer-types 7.4.2) and libwasmvm needs 1.96. The guest compiler is 1.97.1, pinned by the optimizer image and by `cargo +1.97.1`.

`tools/cw-optimizoor` is the no-Docker build of the same guest. It pins rustc 1.97.1, turns guest SIMD off, and runs binaryen without lowering bulk-memory. It only picks workspace members under a `contracts/` directory. The Docker image is the reproducible path.

## What the VM does to the module

`compile_module` (`crates/cosmwasm/packages/vm/src/wasm_backend/compile.rs`) deserializes with parity-wasm, injects the stack-height limiter, meters gas, then compiles with Wasmer 7.4.2 singlepass.

- Limiter crate: `nam-wasm-instrument` fork `0.5.3-terp.2` (git rev `248ac2796903fe39977f31cb9bdef54c77ccf7a0`), features `sign_ext`, `bulk`, `simd`.
- `MAX_WASM_STACK_HEIGHT` is 4096.
- `memory.copy` is allowed. Its metered base cost is 4_500_000 gas. Do not strip bulk-memory for this wasmvm. `wasm-opt` on these artifacts keeps it (`--enable-bulk-memory`). The stock `terpnetwork/optimizer:0.17.0` wasm-opt is version 116 and cannot `--llvm-memory-copy-fill-lowering`.
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

## Arch scan

Every `just core optimize` run ends with a PASS/WARN/ERROR table for each artifact. The scan is `crates/terp-rs/scripts/wasm_fn_budget.py scan`. The optimizer image vendors a copy (`tools/optimizer/scripts/`, `make sync-scanner`) and ships a `cosmwasm-check` built from `crates/cosmwasm`. The scan runs after `checksums.txt` is written and is read-only, so artifact bytes do not change.

- **ERROR** means `cosmwasm-check` (the `compile_module` path: stack limiter, gas metering, singlepass) failed to compile the module. On a relocation failure the scan stubs the largest functions one at a time (`--bisect`, default 12) and names the function whose removal lets the module compile.
- **WARN** means a heuristic overrun (largest body > 69396 bytes, or `i64.div_u` tail > 28062 bytes) on a module that compiles, or a module that only compiles with relaxed static limits (for example "more than 100 locals").
- **x86_64** uses rel32 branches. It is PASS unless the opcode walk fails.

The heuristics are not predictive. Bodies near 69 KB fail on arm64, while `terp721_account_manifold` (tail 42787) and `terp_bounty_escrow` (tail 42989) compile. Only a real compile counts as ERROR.

```sh
WASM_SCAN=0 just core optimize                  # skip
WASM_SCAN_FAIL_ON=error just core optimize      # fail the build on ERROR (`warn`: on WARN too)
python3 scripts/wasm_fn_budget.py scan --check <cosmwasm-check> --functions artifacts/*.wasm   # host run
```

The recipe passes no env into the container by default. To pass the variables, add `-e WASM_SCAN_FAIL_ON` (or similar) to the `docker run`.

### cw_svg_minter on arm64

`ImpossibleRelocation(Dynamic(DynamicLabel(0)))`. The offender was `cw721_svg_fn_split` (≈69 KB, no `i64.div_u`), exported from `contracts/infuser/cw721-svg/src/lib.rs`. That export exists to give the msg decoders a second caller in `cw721_svg.wasm`. Under `features = ["library"]` the cw721-svg entry points are not exported, so in `cw_svg_minter.wasm` it was the decoders' only caller. `wasm-opt -Os` then inlined all three decoders into it. The export is now gated `#[cfg(all(target_arch = "wasm32", not(feature = "library")))]`.

`wasm-opt` inlines every single-caller function, whatever its size. Rust `#[inline(never)]` does not survive that. A second caller, or a `core::hint::black_box` fn pointer (`call_indirect`), does.

### Other findings (2026-10-26, arm64)

- `cw_infuser` (108 locals) and `cw_headstash` (120 locals) exceed the default static limit of 100 locals per function. `cw_headstash` also fails arm64 with ImpossibleRelocation once the limits are relaxed (func[337], body 98027, confirmed by stubbing).
- A fresh image build of `cw721_svg` (body 101638) is ERROR on arm64. The host build of the same tree (largest 74071) and the committed artifact compile. Not caused by the fn_split gate: the cdylib still exports `cw721_svg_fn_split`.
- `cw_headstash`, `cw_headstash_manifold`, `cw_ics08_wasm_crosslink` and `whitelist_mtree` require the `hash_blake` capability.
