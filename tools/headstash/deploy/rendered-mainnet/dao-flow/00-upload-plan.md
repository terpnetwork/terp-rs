# Step 1: plain upload (cw-orch-kms, no gov)

    cargo run --release --bin mainnet_deploy -- hs-upload --signer kms   # Zigner signs one session grant

| code | artifact | sha256 | bytes |
|---|---|---|---|
| cw-headstash | artifacts/mainnet/cw_headstash.wasm | dd40ded55eb436f296fa8eac0d1794f69c5c7369f5a137b1b52dc2cb18e22c52 | 857272 |
| cw-headstash-frontend | artifacts/mainnet/cw_headstash_frontend.wasm | e72eee9b90012ce42fed2c0abd0b1fcc31b868168fa79d30d2f9d6fdd616cac2 | 357081 |

hs-upload refuses to store a file whose sha256 differs from the pin. kms stores raw wasm with instantiate permission Everybody.
After it lands, re-render with `--code-id <cw-headstash code id>` so prop 1 is final.

# Step 3: x/gov circuit only

Unchanged: phase-b/11-circuit-proposal.inner.json and phase-b/12-circuit-exec.unsigned.json (b591de37; metadata = 02 313fb903).

# Who signs what (DAO steps)

- Submitter: Foundation cw4 member `terp16xmf57fr64n9ajux0up3n6ypytct5zggj5d05r` (weight 1000 of 3000).
- Tx target: pre-propose A `terp1armcr7lme0ac3l59dx6ne3et3pe9mfyv57d8pc9m5tyxtzm64dpq9nnwpp`; the proposal lands in **proposal module A** `terp1p5582k3sw008jkmw0f48ucszn660nazqr9x3ut6h5aqj4frq8f0shaey4g` of DAO `terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w`.
  Votes (50-vote.template.json) and execute (51-execute.template.json) go to module A, not module B.
- Prop 1 ends with MsgChangeAdmin to `terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5ffndw` for both denoms (after create, metadata, mint, fund).
- Every DAO action (prop 1 / prop 2 submit, vote, execute) goes through cw-orch-kms (`mainnet_deploy`, Zigner approval page, fees uthiol): MsgExec as the member, fee from the member's session feegrant. Never the terpd CLI. The *.unsigned.json files are reference sign docs only; *.propose.json is the body kms submits. Vote/execute: `mainnet_deploy vote --proposals <id> [--execute]`.
