# Headstash frontend: DAO proposal + chunk upload (mainnet, morocco-1)

Generic cw-orch-kms flow (`scripts/dao-scripts/src/bin/mainnet_deploy.rs`), uthiol fees, no terpd CLI on mainnet.

## Render (reproducible, groot-wan)
```
python3 tools/headstash/e2e/render_frontend_prop.py --dao terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w \
  --code-id 106 --checksum E72EEE9B90012CE42FED2C0ABD0B1FCC31B868168FA79D30D2F9D6FDD616CAC2 \
  --target terpy=terp16ww6y3e7y63v98d9jzg0cft2u0ztx5kda0zvr0e90kwawh4hxweqhtvv7q \
  --target thioly=terp1yc4qtcwzv6g55vqrj95xq8anruwyy2ly4narcqs4a2hzgnu6rlrsa4s9r7 \
  --prover-dir ~/abstract/terp-core/websites/dao-dao-ui/apps/dapp/public/headstash-prover/membership \
  --rest https://api.terp.network --relay https://terp.network --chain-id morocco-1 --denom uthiol \
  --gas-price 0.05 --claim-gas 400000 --out tools/headstash/deploy/rendered-mainnet/dao-flow \
  --verify-lcd https://api.terp.network
```
Re-render is byte-identical (gzip -9, mtime 0, 128 KiB chunks).

| item | value |
|---|---|
| predicted frontend (instantiate2, salt `headstash-frontend:v1`) | terp1gnyl92dycyhwmw7sgmx0d6vt943vf6jtlulrmcs2j97s868wh2tqk7j7cu |
| 30-dao-prop-frontend.propose.json sha256 | 18ac49f2d52f449f05ad5d3def65bdef91f6fd1d1191d592c140a35d1cf93f5f |
| 30-dao-prop-frontend.chunks.json sha256 | 54e91c3b5b8466bd6ee210766254db8c8a5d81936761dab57c35b6eca68b252d |
| manifest pin | cac6977abbd0f9b3281a8458e7fcde0b2ef1de96e8700aca5c1e7ceb906557e7 |
| bundle sha256 | 17b5419d26ed0c28aced5f51a76b35401fdffbbb065a518416583c76761b892f (10 chunks, 678997 B) |
| loader-terpy.html | 53e4a82898a860f4e87fb2985ab58bea89b72e3636c8d8a5be0b06e3104b7a20 |
| loader-thioly.html | 1f87a58324d1f9aa8de5281b45c90c51c60495c0d3fd72c79c5c61b75f1aa69e |

Prover: existing pinned artifacts (zk_headstash.js 37a98d34…a964, zk_headstash_bg.wasm 51681559…c1e8). A fresh rebuild is NOT
byte-reproducible today (wasm-bindgen CLI 0.2.122 vs crate 0.2.129 after lock drift).

Proposal msgs: instantiate2 code 106 (admin+owner = DAO) -> PublishManifest{activate:true} -> set_frontend_contract on terpy and thioly.

## Run (GUI Terminal, groot-wan, repo root)
```
target/release/mainnet_deploy dao-propose --dao terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w \
  --pre-propose terp1armcr7lme0ac3l59dx6ne3et3pe9mfyv57d8pc9m5tyxtzm64dpq9nnwpp \
  --propose-file tools/headstash/deploy/rendered-mainnet/dao-flow/30-dao-prop-frontend.propose.json \
  --propose-sha 18ac49f2d52f449f05ad5d3def65bdef91f6fd1d1191d592c140a35d1cf93f5f \
  --allow-target terp16ww6y3e7y63v98d9jzg0cft2u0ztx5kda0zvr0e90kwawh4hxweqhtvv7q \
  --allow-target terp1yc4qtcwzv6g55vqrj95xq8anruwyy2ly4narcqs4a2hzgnu6rlrsa4s9r7 \
  --grpc http://192.168.10.101:9090 --rpc http://192.168.10.101:26657 --chain-id morocco-1 --broadcast
```
After the proposal passes and is executed (frontend exists at the predicted address):
```
target/release/mainnet_deploy chunk-upload \
  --chunks-file tools/headstash/deploy/rendered-mainnet/dao-flow/30-dao-prop-frontend.chunks.json \
  --chunks-sha 54e91c3b5b8466bd6ee210766254db8c8a5d81936761dab57c35b6eca68b252d \
  --frontend terp1gnyl92dycyhwmw7sgmx0d6vt943vf6jtlulrmcs2j97s868wh2tqk7j7cu \
  --grpc http://192.168.10.101:9090 --rpc http://192.168.10.101:26657 --chain-id morocco-1 --broadcast
```
chunk-upload checks every chunk sha against the on-chain manifest before any grant, skips stored chunks (idempotent).

## Serve (groot2 nginx, additive, NOT applied)
Tunnel already maps terp.network -> http://localhost:80; no cloudflared change. In terp.network.conf before `location /`:
```
location = /headstash { return 301 /headstash/; }
location ^~ /headstash/ { alias /var/www/headstash/; index loader-terpy.html; }
```
Backup, then `sudo nginx -t && sudo systemctl reload nginx`. Relay (`/headstash/feegrant` -> 127.0.0.1:18795) is not deployed yet; loader is off-chain and can be re-rendered.

## Local e2e (no mocks)
```
REST_CORS=1 WASM=artifacts/mainnet/cw_headstash.wasm RESET=1 HARNESS=dao KMS_DEPLOY_BIN=target/release/mainnet_deploy \
FE_WASM=artifacts/mainnet/cw_headstash_frontend.wasm FE_PROP=1 STOP_AFTER_FE=1 HSFE_PROVER_DIR=<prover dir> \
DAO_ROLES=1 TERPD_EXPECT=cosmos_sdk_version NO_LANE=1 tools/headstash/e2e/run-local.sh
```
2026-11-03: E2E_RESULT passed=50 failed=0.
