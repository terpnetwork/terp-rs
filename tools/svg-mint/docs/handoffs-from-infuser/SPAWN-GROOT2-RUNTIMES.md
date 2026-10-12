# Spawn DREGG + HashMerchant on groot2 (nginx + Cloudflare)

Ops task for the runtime teammate. Static-HTML and ZEC-archive teammates do **not** run this.

**Host:** `ssh groot2-wan` (`192.168.10.103`, x86_64). Never cargo on the laptop. Never copy groot-wan aarch64 ELFs.

**Public origins FE uses**

| Origin | Loopback | Job |
|---|---|---|
| `https://dregg-svg.permissionless.money` | `127.0.0.1:8421` `svg-mint-fed` | Checkout, prices, pay-targets |
| `https://hash.terp.network` (also `onboard.permissionless.money`) | `127.0.0.1:19090` `onboard-runtime` | Passkey **feegrant only** |

Ingress: **Cloudflare tunnel → nginx `:80` (Host) → loopback**. Do not point cloudflared at the binary.

---

## Secrets (exist or stop)

Files on groot2, mode `600`, never git/nginx/chat/logs:

| File | Contents |
|---|---|
| `~/.terp/dregg-svg.env` | `DREGG_HD_SEED` (hex, persist), `DREGG_MINTER_LCD`, `DREGG_MINTER_ADDR` (minter `terp188jppp…`), optional `DREGG_MINTER_GAS_RESERVE`, ZEC vars when LWD is tip-ready |
| `~/.terp/onboard.env` | `ONBOARD_DAO` MAIN `terp1fxccvvhhy43tvet2ah7jqwq4cwl9k3dxmww052`, `ONBOARD_RUNTIME` **disjoint** from DREGG mint runtime, `ONBOARD_CHAIN=morocco-1`, `ONBOARD_BROADCAST=1`, `ONBOARD_FROM` key name |
| `~/.terp/dregg-svg-runtime.mnemonic` | cw-orch mint signer (not HashMerchant) |
| terpd keyring for `ONBOARD_FROM` | HashMerchant feegrant broadcasts |

Do **not** set `DREGG_MINTER_FUNDED=1`. Do **not** invent a seed if `DREGG_HD_SEED` is missing (binary would rotate dests every restart).

---

## Build (on groot2)

```bash
ssh groot2-wan
export PATH="$HOME/.cargo/bin:/usr/local/bin:$PATH"
cd ~/abstract/terp-core/crates/dregg/svg-mint
cargo build --release --bin svg-mint-fed
file target/release/svg-mint-fed   # ELF 64-bit LSB x86-64

cd ~/abstract/terp-core/crates/terp-rs/tools/onboard-authz
cargo build --release --bin onboard-runtime
file target/release/onboard-runtime
```

---

## Spawn

```bash
set -a
. ~/.terp/dregg-svg.env
. ~/.terp/onboard.env
set +a
# Bind loopback (script defaults)
export SVG_MINT_FED_BIND=127.0.0.1:8421
export ONBOARD_BIND=127.0.0.1 ONBOARD_PORT=19090
bash ~/abstract/terp-core/crates/dregg/svg-mint/scripts/spawn-groot2-runtimes.sh
```

Cloudflare: existing named tunnel Hosts `dregg-svg.permissionless.money` and `hash.terp.network` / `onboard.permissionless.money` must target **nginx :80**, not `:8421`/`:19090`. Inspect `~/.cloudflared/config.yml` (or the token tunnel ingress). Grey-cloud is fine; do not invent a new tunnel id.

---

## Prove (must quote HTTP)

Local via Host header:

```bash
curl -sS -H 'Host: dregg-svg.permissionless.money' http://127.0.0.1/health
# HTTP 200, "service":"dregg-svg". ok/checkout_ready may be false until LCD funds — still 200.

curl -sS -H 'Host: hash.terp.network' http://127.0.0.1/onboard/status
# service hashmerchant-onboard (or onboard-authz), chain_id morocco-1

curl -sS -o /dev/null -w '%{http_code}\n' -H 'Host: hash.terp.network' http://127.0.0.1/onboard/v1/mint-grant
# 404
```

Public:

```bash
curl -sS https://dregg-svg.permissionless.money/health
curl -sS https://dregg-svg.permissionless.money/v1/prices
curl -sS https://hash.terp.network/onboard/status
curl -sS -o /dev/null -w '%{http_code}\n' https://hash.terp.network/onboard/v1/mint-grant
```

`live=true` only if **public** health has `service=dregg-svg`, public onboard status is reachable, mint-grant is **404**. Pay may stay closed (`checkout_ready:false`) until minter LCD is funded — that is honest, not a spawn fail.

ZEC `zec_uri` may be null until the archive teammate finishes LWD. Do not block spawn on ZEC.

---

## Keep-outs

- No migrate of minter code 72
- No mint-grant
- No `DREGG_MINTER_FUNDED=1`
- No terp1 on ETH/Base/BTC/ZEC
- No HashMerchant paying the NFT
- No cargo on laptop / groot-wan aarch64 copy

Workflow: `/workflow dregg-groot2-runtimes`
