# Step 1: plain upload (cw-orch-kms, no gov)

    cargo run --release --bin mainnet_deploy -- --phase hs-upload --signer kms   # Zigner signs one session grant

| code | artifact | sha256 | bytes |
|---|---|---|---|
| cw-headstash | artifacts/mainnet/cw_headstash.wasm | dd40ded55eb436f296fa8eac0d1794f69c5c7369f5a137b1b52dc2cb18e22c52 | 857272 |
| cw-headstash-frontend | artifacts/mainnet/cw_headstash_frontend.wasm | e72eee9b90012ce42fed2c0abd0b1fcc31b868168fa79d30d2f9d6fdd616cac2 | 357081 |

hs-upload refuses to store a file whose sha256 differs from the pin. kms stores raw wasm with instantiate permission Everybody.
After it lands, re-render with `--code-id <cw-headstash code id>` so prop 1 is final.

# Step 3: x/gov circuit only

Unchanged: phase-b/11-circuit-proposal.inner.json and phase-b/12-circuit-exec.unsigned.json (2709cc7b).
