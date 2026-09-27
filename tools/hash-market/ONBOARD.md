# HashMerchant onboard (product side)

Onboard Authz is **embedded in hash-market-server** (`--features server` includes `onboard`).
There is **no** standalone `onboard-runtime` process and **no** `terp-onboard-authz` crate.

One-shot CLI (same crate, not a daemon):

```sh
cargo run -p hash-market --features onboard --bin prepare-authz -- --help
```

Public host: `https://hash.terp.network`  
Image pin: `registry.terp.network/hash-market:20260817-onboard`  
(`@sha256:cf4b394ce47982d29bb95d2f2371863c1aeb8eb9e5ac2d4f89fcc0056af9a4a5`)

## What this process is allowed to know

| Role | Address | Where it lives |
|---|---|---|
| DAO / granter (TEST wallet) | `terp1jehyjvvvdjlhpsshna4jh5863954vjxydzf0ca` | Akash env `ONBOARD_DAO` (or `DAO_ADDR`) |
| Runtime (feegrant issuer) | `terp14ekg32juuydrd07v5e6m0d7mefc8c6n5rrmxuy` | Akash env `ONBOARD_RUNTIME` (or `RUNTIME_KEY_ADDR`) + SFTP mnemonic |
| Runtime mnemonic | file only | groot `~/.terp/hashmerchant-runtime.mnemonic` → `/etc/hash-market/` `chmod 600` |

Never put `TEST_MNEMONIC` or `MAIN_MNEMONIC` in this container.

## How `/onboard/status` becomes `ready: true`

Env (wins) then toml. Both addresses must be live `terp1…` (length ≥ 39).  
`DAO_ADDR` / `RUNTIME_KEY_ADDR` as *values* → `ready: false` and grants 400.

```
GET /onboard/status
{
  "ready": true,
  "dao": "terp1jehyjvvvdjlhpsshna4jh5863954vjxydzf0ca",
  "runtime_key": "terp14ekg32juuydrd07v5e6m0d7mefc8c6n5rrmxuy",
  "broadcast": false,
  ...
}
```

`GET /health` is HashMerchant liveness only (not onboard).

## Broadcast

This image has **no terpd**. `ONBOARD_BROADCAST=1` is ignored unless `terpd version` works. Leave `broadcast=0`. Grants queue (`pending_*`).

## 120u-1 vs morocco-1

- 120u-1 authz already on chain (fund `10653B3C…`, grant `78C147C1…`).
- Mainnet: same runtime, grant from `terp1fxccvvhhy43tvet2ah7jqwq4cwl9k3dxmww052` after `terpd-mainnet` `catching_up=false`. Do not tell anyone mint/auth is live until `/onboard/status` prints the real `runtime_key` **and** that grant exists.

## o-line (their side — do not invent a new list)

1. `HASHMERCHANT_IMAGE=registry.terp.network/hash-market@sha256:cf4b394c…` so `oline hashmerchant render` image: line is this digest, not the Mac stub `6aafc8cf`.
2. Render → deploy → **upload** (`oline hashmerchant upload` + SFTP mnemonic) **before** DNS/`--force-danger`.
3. Remanifest wipes `/etc/hash-market` (not the beta3 volume). Expect sshd again.
4. CNAME only after `/health` and `/onboard/status` are JSON with live `terp1` roles.
5. Energize existing preflight: `crates/o-line/src/suite/preflight.rs`, `docs/examples/hashmerchant.md`, `just test hashmerchant`, `oline testnet-deploy --preflight-only`.

Operator copies: `~/.oline/hashmerchant/config.toml`, `~/.oline/hashmerchant-ssh-key`, rendered yml.
