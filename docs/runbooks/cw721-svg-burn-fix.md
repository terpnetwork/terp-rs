# cw721-svg burn fix runbook

Status 2026-11-11: code 111 stored (sha256 f275bba3…6122), permissionless-fractal migrated to 111 (next_token_id 18).
svg-minter update_code_id 111 is still pending (svg_code_id 98). The record below is the staging README used for the run.

# permissionless-fractal burn fix (morocco-1) — 2026-11-11

Nothing here has been broadcast. All three steps are dry-run verified.

## Mainnet facts (read-only LCD 192.168.10.101:1317, 2026-11-11 ~04:20 UTC)
- collection `terp15anc99l7lvk2v2pzjusnmylr3lw3xnpxg89xv8uh0fj6pwrumrksqwzs6x` label `permissionless-fractal`, code 98, wasm admin + creator-owner `terp1fxccvvhhy43tvet2ah7jqwq4cwl9k3dxmww052`, cw2 `cw721_svg 0.1.0`
- num_tokens 17; ids 0..17 minus 12; highest id 17 (owner terp1slpz5656…); 12 not found -> next mint retries 17 (jam)
- total 10000, tiers 100/500/1000 THIOL (until 1000/5000/10000), whitelist terp1k5vz89…
- svg-minter `terp1wg5z9rcw…` code 99: `code_id {}` = 98 (Item `svg_code_id`); cw-ownable owner terp1fx…; wasm admin terp14w2qva6d… ; change via owner-only `update_code_id {code_id}` (no migrate needed)
- 31 code-98 collections scanned: only the fractal has a gap/jam.

## Artifact
`cw721_svg.wasm` 1,207,987 B, sha256 `f275bba344e104daa43b613479a1ecd55551d1f7a64af0381b84bda3884f6122` (code 98 on chain: 1,310,006 B; chain MaxWasmSize 1,638,400 B)
terp optimizer build: image `terpnetwork/optimizer-arm64:0.17.0-rust1.97` (rustc 1.97.1, wasm-opt 116), canonical mounts from `just core optimize`, volumes terp-rs_cache/registry_cache; bob's exact per-package cargo args + `wasm-opt -Os --enable-bulk-memory --enable-sign-ext --disable-simd`. Pre-opt 1,456,162 B. Image cosmwasm-check: pass; arm64/x86_64 singlepass scan PASS (largest fn 68,960). Earlier host build kept as `cw721_svg.host-build.wasm` (1,199,881 B, sha 7571f81e…1604) — not for upload.
Host cosmwasm-check (caps incl. cosmwasm_3_0) on the optimizer artifact: pass.

## Broadcast, in order (from ~/abstract/terp-core/crates/terp-rs on groot-wan)
```sh
S=~/grok-bot/staging/fractal-burn-fix-20261111
cargo build -p dao-scripts --bin cw721_svg_burn_fix
# (a) store -> prints "stored code id N"
TERP_SCRIPT_BROADCAST=1 target/debug/cw721_svg_burn_fix store --wasm $S/cw721_svg.wasm --out $S --chain-id morocco-1 --broadcast
# (b) migrate fractal -> N, next_token_id 18 (refuses unless code N checksum == wasm sha256, admin == terp1fx, token 18 absent)
TERP_SCRIPT_BROADCAST=1 target/debug/cw721_svg_burn_fix migrate --wasm $S/cw721_svg.wasm --code-id N --next-token-id 18 --out $S --chain-id morocco-1 --broadcast
# (c) svg-minter update_code_id N (refuses unless owner == terp1fx and checksum matches)
TERP_SCRIPT_BROADCAST=1 target/debug/cw721_svg_burn_fix update-code-id --wasm $S/cw721_svg.wasm --code-id N --out $S --chain-id morocco-1 --broadcast
```
Each step opens its own cw-orch-kms session (approval page on a loopback port 18700-18999, printed with the ssh -L line), scoped to: (a) MsgStoreCode for this checksum only, (b) MsgMigrateContract on the fractal only, (c) MsgExecuteContract `update_code_id` on svg-minter only. Grants revoke on close. Drop `--broadcast` for a dry-run.

## Verify after (read-only)
- `raw/svg_next_token_id` on the fractal = 18; contract code_id = N
- svg-minter `{"code_id":{}}` = N
- next mint emits token_id 18
