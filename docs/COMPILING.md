# Compiling contracts

Contracts in this workspace are built with the CosmWasm optimizer image or with `cargo cw-optimizoor`. Guest rustc is **1.91.0**. Both tools strip symbols, leave bulk-memory and sign-extension enabled, and compile with guest SIMD off (`-C target-feature=-simd128`).

Image names and how the image is built are in the repository guide [docs/COMPILING.md](../../../docs/COMPILING.md).

| Architecture | Image |
|---|---|
| linux/amd64 | `terpnetwork/optimizer:0.17.0` |
| linux/arm64 | `terpnetwork/optimizer-arm64:0.17.0` |

`terpnetwork/rust-optimizer` and `terpnetwork/workspace-optimizer` are the same image (`-arm64` on the arm64 tag).

## Docker optimizer

This workspace path-depends on crates that sit next to `terp-rs`. Mount that parent directory and point `PROJECT_DIR` at this workspace. On arm64, swap in `terpnetwork/optimizer-arm64:0.17.0`.

```sh
cd crates
docker run --rm \
  -v "$(pwd)":/workspace \
  -e PROJECT_DIR=/workspace/terp-rs \
  --mount type=volume,source=terp-rs_cache,target=/target \
  --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry \
  terpnetwork/optimizer:0.17.0
```

Optimized wasm is written to `crates/terp-rs/artifacts/`.

A contract with its own `[workspace]` (`contracts/hash-merchant/*`, `contracts/cashu/cw-cashu-registry`) is built by setting `PROJECT_DIR` to that directory:

```sh
cd crates
docker run --rm \
  -v "$(pwd)":/workspace \
  -e PROJECT_DIR=/workspace/terp-rs/contracts/cashu/cw-cashu-registry \
  --mount type=volume,source=cw-cashu-registry_cache,target=/target \
  --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry \
  terpnetwork/optimizer:0.17.0
```

## cw-optimizoor

`cw-optimizoor` is the same guest compile without Docker. It builds workspace members whose paths are under `contracts/`.

```sh
rustup toolchain install 1.91.0
rustup target add wasm32-unknown-unknown --toolchain 1.91.0
cargo +1.91.0 install --path tools/cw-optimizoor

cd crates/terp-rs
cargo cw-optimizoor
```

`cargo cw-optimizoor /path/to/Cargo.toml` selects another workspace manifest. A typecheck of one package, before either optimizer, is:

```sh
cd crates/terp-rs
unset RUSTFLAGS
cargo +1.91.0 check --target wasm32-unknown-unknown --lib -p <package>
```

`unset RUSTFLAGS` lets `crates/terp-rs/.cargo/config.toml` apply `--allow-undefined` and `-C target-feature=-simd128` for that host check. The optimizer and `cw-optimizoor` apply their own `RUSTFLAGS` when they produce artifacts.
