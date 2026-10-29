set -e
S=/tmp/hsfe-standalone
C=$HOME/abstract/terp-core/crates
mkdir -p /tmp/hsfe-opt-target /tmp/hsfe/opt-out
cp -cR /tmp/hs-p2-opt-target/. /tmp/hsfe-opt-target/ 2>/dev/null || true
/usr/local/bin/docker run --rm -m 6g -v $C:$C -v $S:$S -v $HOME/.cargo/registry:/usr/local/cargo/registry -v $HOME/.cargo/git:/usr/local/cargo/git -v /tmp/hsfe-opt-target:/target -v /tmp/hsfe/opt-out:/out \
  -w $S -e CARGO_TARGET_DIR=/target --entrypoint sh terpnetwork/optimizer-arm64:0.17.0-rust1.91 -c '
set -e
cargo build -j 4 --ignore-rust-version --release --lib --target wasm32-unknown-unknown -p cw-headstash --features zk-api 2>&1 ; echo BUILD_RC=$?
cargo build -j 4 --ignore-rust-version --release --lib --target wasm32-unknown-unknown -p cw-headstash-frontend 2>&1 ; echo BUILD_RC=$?
for n in cw_headstash cw_headstash_frontend; do
 wasm-opt -Os --one-caller-inline-max-function-size=2000 --enable-bulk-memory --enable-sign-ext --disable-simd /target/wasm32-unknown-unknown/release/$n.wasm -o /out/$n.wasm
 sha256sum /out/$n.wasm; wc -c /out/$n.wasm
 cosmwasm-check --available-capabilities cosmwasm_1_1,cosmwasm_1_2,cosmwasm_1_3,cosmwasm_1_4,cosmwasm_2_0,cosmwasm_2_1,cosmwasm_2_2,cosmwasm_3_0,iterator,staking,stargate,bulk_memory,hash_poseidon,hash_blake,redpallas,bn254,ibc2 --wasm-limits "{\"max_function_locals\":1024}" /out/$n.wasm 2>&1 | tail -2
done'
echo OPT_DONE
