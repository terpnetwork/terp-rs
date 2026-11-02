#!/bin/sh
# k19 lab: threaded wasm build of the raw-key prover (browser-mt). Output /tmp/hs-mt-pkg.
set -e
cd "$(dirname "$0")/../circuits/halo2"
export CARGO_TARGET_DIR=/tmp/hs-mt-target
export RUSTFLAGS='-C target-feature=+atomics,+bulk-memory,+mutable-globals -C link-arg=--shared-memory -C link-arg=--import-memory -C link-arg=--max-memory=4294967296 -C link-arg=--export=__wasm_init_tls -C link-arg=--export=__tls_size -C link-arg=--export=__tls_align -C link-arg=--export=__tls_base -C link-arg=--export=__heap_base -C link-arg=--export=__data_end --cfg getrandom_backend="wasm_js"'
rm -rf /tmp/hs-mt-pkg
wasm-pack build . --release --target web --out-dir /tmp/hs-mt-pkg --out-name zk_headstash -- --no-default-features --features browser-mt -Z build-std=panic_abort,std
wasm-tools print /tmp/hs-mt-pkg/zk_headstash_bg.wasm | grep -m2 'memory'
