# Headstash v0.0.1 — Reproducible Verification Guide (morocco-1)

Everything below can be checked with public sources and the public LCD `https://api.terp.network`.

## 1. cw_headstash wasm (code 105)

### Expected
| Artifact | sha256 | size |
|---|---|---|
| `cw_headstash.wasm` (code 105) | `dd40ded55eb436f296fa8eac0d1794f69c5c7369f5a137b1b52dc2cb18e22c52` | 857,272 B |
| `cw_headstash_frontend.wasm` (code 106) | `e72eee9b90012ce42fed2c0abd0b1fcc31b868168fa79d30d2f9d6fdd616cac2` | — |
| `Cargo.lock` used for the build (resolved) | `df105de86b0f6dbc8e3952374ed769b884d4f3c30ac14e5cb05038429f69bc4f` | — |

### Sources
| Repo | URL | Branch | Commit |
|---|---|---|---|
| terp-rs (contract source) | https://github.com/permissionlessweb/terp-rs | wip/headstash-consolidate (pending publish) | `4de39b1931806f9354403510c55a09fd8743394c` |
| cosmwasm | https://github.com/permissionlessweb/cosmwasm | security/advisory-bumps | `adace180be362e03c5aabed4b705c37c7f633d94` |
| cw-minus | https://github.com/permissionlessweb/cw-minus | main | `e9aaeafc9a05c98ba9cb7c970557c316b81a6573` |
| cw-plus-plus | https://github.com/permissionlessweb/cw-plus-plus | feat/mvp | `efa52358b2978abedecc7c58c27f817af0aa5a37` |
| cw-storage-plus | https://github.com/permissionlessweb/cw-storage-plus | v3.1.0-zk.0 | `cb69d46c10c91d391a13d9ee55a80684c1fbd95a` |
| zakura-common | https://github.com/permissionlessweb/common | feat/plonk-artifact-codec | `5df51585a0df2faaceb15cc1e406ecd060265ffc` |

Image: `terpnetwork/optimizer-arm64:0.17.0-rust1.91@sha256:b4d2087efbaf36126e0b992b4313b5c322502bc3bb538582050d324bfd6f5d6f` (arm64 host).

### Mount path (required, baked into the wasm)
rustc embeds absolute source paths in the wasm. The published hash was built with the sources mounted at

```
/Users/returniflost/abstract/terp-core/crates/terp-rs-consolidate     # terp-rs
/Users/returniflost/abstract/terp-core/crates/<sibling>               # each sibling above
```
Any other mount path yields a different sha256. The build script bind-mounts host `$CRATES` read-only, mounts each `SNAP/<sibling>` over `$CRATES/<sibling>`, and mounts the terp-rs source at `$CRATES/terp-rs-consolidate`. So create `$CRATES=/Users/returniflost/abstract/terp-core/crates` on the build host (it can hold just the extra siblings listed below) and export `CRATES` to that path.

The terp-rs workspace manifest also references other sibling repos (cw-orchestrator, cosmos-rust, polytone, …); they must be present at `$CRATES/<name>` for cargo to load the workspace, but they are not in cw-headstash's dependency graph, so their exact commit does not affect the hash. Verified isolated build needed: cw-orchestrator cw-multi-test-fork polytone abstract alloy cosmos-rust dao-contracts cw-asset cw-packages cw-plus cw-nfts flock zcash hermes ibc-proto-rs ics23 ict-rs osmosis-rust zeratul tendermint-rs zebra-crosslink librustzcash c-kzg-4844 blst polytone-evm. Mount them into the container at `$CRATES/<name>` (read-only).

### Build
```sh
mkdir -p snap && cd snap
git clone https://github.com/permissionlessweb/cosmwasm        && git -C cosmwasm checkout adace180be362e03c5aabed4b705c37c7f633d94
git clone https://github.com/permissionlessweb/cw-minus        && git -C cw-minus checkout e9aaeafc9a05c98ba9cb7c970557c316b81a6573
git clone https://github.com/permissionlessweb/cw-plus-plus    && git -C cw-plus-plus checkout efa52358b2978abedecc7c58c27f817af0aa5a37
git clone https://github.com/permissionlessweb/cw-storage-plus && git -C cw-storage-plus checkout cb69d46c10c91d391a13d9ee55a80684c1fbd95a
git clone https://github.com/permissionlessweb/common zakura-common && git -C zakura-common checkout 5df51585a0df2faaceb15cc1e406ecd060265ffc
for r in cosmwasm cw-minus cw-plus-plus cw-storage-plus zakura-common; do echo "$r $(git -C $r rev-parse HEAD)"; done > MANIFEST.txt
cd ..
git clone https://github.com/permissionlessweb/terp-rs src && git -C src checkout 4de39b1931806f9354403510c55a09fd8743394c
# place the resolved lock (sha256 df105de8…bc4f) at src/Cargo.lock
docker pull terpnetwork/optimizer-arm64:0.17.0-rust1.91@sha256:b4d2087efbaf36126e0b992b4313b5c322502bc3bb538582050d324bfd6f5d6f
SNAP=$PWD/snap CRATES=/Users/returniflost/abstract/terp-core/crates \
  tools/headstash/deploy/build-cw-headstash.sh $PWD/src $PWD/out
sha256sum out/cw_headstash.wasm   # dd40ded55eb436f296fa8eac0d1794f69c5c7369f5a137b1b52dc2cb18e22c52
```
Inside the container: `cargo build -j 4 --ignore-rust-version --release --lib --target wasm32-unknown-unknown -p cw-headstash --features zk-api`, then `wasm-opt -Os --one-caller-inline-max-function-size=1000 --enable-bulk-memory --enable-sign-ext --disable-simd`.

### Compare with chain
```sh
curl -s https://api.terp.network/cosmwasm/wasm/v1/code/105 | jq -r .code_info.data_hash   # DD40DED55EB436F296FA8EAC0D1794F69C5C7369F5A137B1B52DC2CB18E22C52
curl -s https://api.terp.network/cosmwasm/wasm/v1/code/106 | jq -r .code_info.data_hash   # E72EEE9B90012CE42FED2C0ABD0B1FCC31B868168FA79D30D2F9D6FDD616CAC2
curl -s https://api.terp.network/cosmwasm/wasm/v1/code/105 | jq -r .data | base64 -d | sha256sum   # byte-for-byte
curl -s https://api.terp.network/cosmos/tx/v1beta1/txs/A751CD56386D97FC301868F09881431665E5525EE4F6FAC9C293625DA4AE68D3   # store code 105
curl -s https://api.terp.network/cosmos/tx/v1beta1/txs/F3D620DCF60FB0802DB15DB091A3703E104E10661C57B0F1800A276D973CA99D   # store code 106
```
Both codes: creator `terp16kl3dvrphct0md27y36kams3lj9g55lw24nm4x`, instantiate permission Everybody.

### Frontend (code 106)
Build recipe pending: `build-cw-headstash.sh` builds only cw-headstash. Until a recipe is published, code 106 can be checked only against the published sha256 above.

## 2. Circuit (governance proposal phase-b/11)

Proposal: `tools/headstash/deploy/rendered-mainnet/phase-b/11-circuit-proposal.inner.json`, sha256 `a58616f734531002dfbd0b9ac2b9d07bbdb7d27c8f7b6ca905ef5938e03286e9`. Message `MsgStoreFullCircuit`, sender = gov module, auth `{circuit_type 0, curve_type 0, k 11}`.

Regenerate (K=11, deterministic, minutes on a laptop; generator `tools/headstash/examples/bench_note_proofs.rs`, package `zk-headstash`):
```sh
cargo run --release --locked -j 4 -p zk-headstash --example bench_note_proofs ./circ 0
# blob 135023 params 131140 vk_body 3883 footer 80
sha256sum circ/params.bin circ/vk_body.bin
cat circ/params.bin circ/vk_body.bin | sha256sum
```
| | sha256 |
|---|---|
| params.bin (131,140 B) | `1eab6f93a080ce41b908d935c04bd2e3ed1ac23f277c15d11c499d56d28fa0f7` |
| vk_body.bin (3,883 B) | `30cff614a3c554fe03e39595f41b2130fa4653ddc949b8542986094d30fff9cc` |
| params ‖ vk_body | `34d11437d957554f4008377dece5fac0f047e0e9cc012d15b3d09448a6cc5398` |

After the proposal executes, query the stored circuit (zk_id 1) on the zk module and check that its params/vk hashes equal the table.

## 3. DAO deployment (prop 14) and instantiate2 addresses

- DAO core: `terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w`
- Prop 14 execution tx `2CB3132F0B24212196EC78513E8B83925A139974CCA43FCDD31EEF21109ADAA9`, height 24291979

| Instance | Address | Label | Salt (base64) |
|---|---|---|---|
| terpy | `terp16ww6y3e7y63v98d9jzg0cft2u0ztx5kda0zvr0e90kwawh4hxweqhtvv7q` | headstash-2026-1-terpy | `c2WPXjvl1zZDgplxgztVXuqkf3/myk7sNVWSBv+JA8k=` |
| thioly | `terp1yc4qtcwzv6g55vqrj95xq8anruwyy2ly4narcqs4a2hzgnu6rlrsa4s9r7` | headstash-2026-1-thioly | `6T5ZUhUD3s6Q+FiWB6/E9/TeFwKecA7X3Zu18+ezScE=` |

Both: code 105, admin = creator = DAO core. Frontend (gov flow): `terp1llutyfy858ehucaleypljfgut9srz34kd5c06p27hzacv0gfyjwqg0e5n9`.

`addr = sha256(sha256("module") ‖ "wasm\0" ‖ lp(checksum) ‖ lp(creator) ‖ lp(salt) ‖ lp(""))`, lp = 8-byte big-endian length prefix.
```sh
python3 tools/headstash/instantiate2_check.py \
  dd40ded55eb436f296fa8eac0d1794f69c5c7369f5a137b1b52dc2cb18e22c52 \
  terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w \
  c2WPXjvl1zZDgplxgztVXuqkf3/myk7sNVWSBv+JA8k=      # -> terp16ww6…v7q
python3 tools/headstash/instantiate2_check.py \
  dd40ded55eb436f296fa8eac0d1794f69c5c7369f5a137b1b52dc2cb18e22c52 \
  terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w \
  6T5ZUhUD3s6Q+FiWB6/E9/TeFwKecA7X3Zu18+ezScE=      # -> terp1yc4q…9r7
curl -s https://api.terp.network/cosmwasm/wasm/v1/contract/terp16ww6y3e7y63v98d9jzg0cft2u0ztx5kda0zvr0e90kwawh4hxweqhtvv7q | jq .contract_info
```

## Known gaps
- terp-rs `4de39b1` must be published (it is not yet on a public branch).
- The resolved `Cargo.lock` (df105de8…) must be published alongside.
- The workspace manifest needs the full set of terp-core sibling repos present at `$CRATES` to load.
- Frontend (code 106) recipe pending.
