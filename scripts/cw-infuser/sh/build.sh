#!/bin/sh
# compile 
cd cw-infuser && cargo wasm 
# move wasm 
mv ../target/wasm32-unknown-unknown/release/cw_infuser.wasm ../artifacts/
# run cw-orch
sha256sum ../artifacts/cw_infuser.wasm