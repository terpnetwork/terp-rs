# rust-deps: Rust dependency scraper

`gen-deps.mjs` runs `cargo metadata` on one or more cargo roots and merges their
full dependency trees (normal + build dependencies, direct and transitive, all
target platforms; dev-dependencies left out) into one JSON graph. Depth is the
shortest path from a workspace member (members are depth 0). Node ≥ 18 and
cargo are the only requirements.

```sh
just gen deps                       # from the terp-rs root → target/gen-deps/rust-crates.json
just gen deps path/to/out.json      # custom output
node tools/gen-tools/rust-deps/gen-deps.mjs --out out.json \
  --root wasmvm=/src/wasmvm/libwasmvm --only terp-rs,wasmvm --offline
```

Options: `--out FILE` (or a bare first argument), `--config FILE` (default
`roots.json` here), `--root id=PATH` (override or add a root), `--only a,b`,
`--offline` (pass to cargo), `--cache-dir DIR` and `--no-clone`. `CARGO`
overrides the cargo binary.

## Roots

`roots.json` lists the default roots: the terp-rs workspace, CosmWasm, wasmvm
(`libwasmvm`), Wasmer and parity-wasm. Each path takes `${VAR}` /
`${VAR:-default}` from the environment (`TERP_RS_DIR`, `COSMWASM_DIR`,
`WASMVM_DIR`, `WASMER_DIR`, `PARITY_WASM_DIR`); relative paths resolve against
`roots.json`. A root whose path is unset or missing is shallow-cloned from its
`git` URL into `$XDG_CACHE_HOME/terp-gen-deps/<id>` (or `~/.cache/...`) unless
`--no-clone` is given.

## Output

`{ generated_at, generator, edge_format, roots[], counts{crates,edges,shared}, crates[], edges[] }`.
A crate is `{ n, v, s: path|git|registry, d, r: { rootId: depth } }`, keyed by
name@version#source. An edge is `[from, to, rootBitmask, buildOnly]`.

The Terp Network site (`websites/terp.network`) consumes this file as
`public/graphs/rust-crates.json` for `public/graphs/rust-crates.html`; its
`scripts/gen-rust-crates.mjs` calls this script with that output path.
