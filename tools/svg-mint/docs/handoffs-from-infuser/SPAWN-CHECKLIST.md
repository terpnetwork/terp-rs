# Spawn checklist: DREGG + HashMerchant onboard + morocco-1 collection mint

Do these in order. Mint is **cw-orch Daemon**, not `terpd`. HashMerchant is **passkey feegrant only**, never SVG mint.

Live morocco-1 (do not use lab `terp1wug8sewp6…`):

| | code | address |
|---|---|---|
| cw-svg-minter | 72 (instance) | `terp188jpppxtfpf0vvt5gypdj52hk2xuswwcn38auczzxv20fcxz3kss9fhlru` |
| cw721_svg wasm | **83** (minter `update_code_id`) | new collections |
| cw721_svg live collection | 71 | `terp1vp7smemrv846hpsyqrzlnqapmu3sv7hyygf4rrnw3ls26ezk864qupr5zt` |

---

## A. DREGG runtime (groot2 + Cloudflare)

- [ ] **A1.** Build **linux/amd64** `svg-mint-fed` on **groot2** (host is x86_64). Do **not** copy groot’s aarch64 ELF.
- [ ] **A2.** Two mnemonics, two files (never the same key, never git/nginx/chat):
  ```sh
  umask 077
  mkdir -p ~/.terp
  # DREGG mint runtime (groot2 + groot-wan) — NOT the admin
  nano ~/.terp/dregg-svg-runtime.mnemonic
  chmod 600 ~/.terp/dregg-svg-runtime.mnemonic
  # cw-svg-minter admin terp1fxcc… (groot-wan only) — collection/minter admin, not a migrate signer
  nano ~/.terp/svg-minter-admin.mnemonic
  chmod 600 ~/.terp/svg-minter-admin.mnemonic
  ```
  Addresses are derived. Do not export bech32.
- [ ] **A3.** Env on groot2:
  - `SVG_MINT_FED_BIND=0.0.0.0:8421`
  - `SVG_MINT_FED_STORE=$HOME/.terp/svg-mint-fed-cell.json`
  - `DREGG_HD_SEED` (hex, persist)
  - prices: `GET /v1/prices` SSOT — ETH/Base 0.0005 / BTC 0.000015 / ZEC 0.0015 / 100 TERP\|THIOL; first 1000 rails BOGO
  - ZEC when ready: `ZCASH_DEVTOOL_BIN` (linux/amd64 ELF on **groot2**), `ZCASH_DEVTOOL_SERVER=127.0.0.1:9067`, `ZCASH_DEVTOOL_NETWORK=main`, UFVK dir. Full infra: `ZEC-INFRA-HANDOFF.md`
  - Self-hosted BTC/ETH/Base: `BTC_RPC` / `ETH_RPC` / `BASE_RPC` must be house/lab (`127.0.0.1` or `DREGG_RPC_ALLOWLIST`). On groot2 spawn, `SVG_MINT_REQUIRE_SELF_HOSTED_RPC=1` is required. Porch P2P is Akash only — `PORCH-MULTICHAIN-HANDOFF.md`
- [x] **A4.** `svg-mint-fed` on groot2 `:8421` — `GET /health` `{ok:true,service:dregg-svg}`
- [x] **A5.** `GET /v1/pay-targets?recipient=terp1…` → `zip321`, `memo_hex` starts `6431`, fresh HD addrs, `bogo`
- [x] **A6.** `{DREGG}` = `https://dregg-svg.permissionless.money` (tunnel `c72adebb…` → `:8421`)

---

## B. HashMerchant local (authz proxy only)

- [ ] **B1.** Spawn HashMerchant with **onboard** nest: `/onboard/status`, `/onboard/v1/*-grant`
- [ ] **B2.** `ONBOARD_RUNTIME` (long-lived authz granter) **≠** `ONBOARD_HASHMERCHANT` (temp fee.granter). Runtime must not appear as customer `fee.granter`. See `HASHMERCHANT-AUTHZ.md`.
- [ ] **B3.** Grants **only** passkey authenticator registration txs. No mint grant, no rail payment.
- [ ] **B4.** FE: Keplr **or** local key + existing passkey flow + onboard feegrant, then DREGG checkout (`FE-HANDOFF.md`)

---

## C. Fresh svg wasm + minter `update_code_id` (no migrate)

Live **code 72 has no `migrate` export**. Point the existing minter at a **new** `cw721_svg` code id:

```sh
cargo run --example svg_mint_update_svg_code -p ict-rs-cw-orch
SVG_CODE_UPLOAD=1 UPDATE_BROADCAST=1 cargo run --example svg_mint_update_svg_code -p ict-rs-cw-orch
```

Optional `MINTER_CODE_UPLOAD=1` stores a new minter wasm (does **not** migrate the live instance). Signer is `~/.terp/svg-minter-admin.mnemonic`. gRPC `http://127.0.0.1:9090`.

- [ ] **C1.** Dry-run `svg_mint_update_svg_code` (admin derived `terp1fxcc…`).
- [ ] **C2.** Upload + `update_code_id` on minter `terp188jppp…`.
- [ ] **C3.** Fund DREGG runtime; hopalong mint on collections created after the update (or existing collection if still on old svg code).

---

## D. Go-live gates (do not open public deposits until)

- [ ] Tunnel `{DREGG}/health` and pay-targets from the public hostname
- [ ] HashMerchant onboard grant works for one passkey registration
- [ ] DREGG runtime can `Daemon.execute` hopalong mint on live `cw721_svg` (not migrate code 72)
- [ ] One lab hopalong mint via Daemon (`mint_engine=cw-orch`) already proven locally
- [ ] ZEC button hidden in FE until `zec_uri` is non-null (UFVK + compact streamer)

FE contract: `FE-HANDOFF.md`. Live groot2 spawn (nginx + Cloudflare): `SPAWN-GROOT2-RUNTIMES.md` and `/workflow dregg-groot2-runtimes`. Direct-to-binary helper (do not use for public FE): `scripts/spawn-groot2-tunnel.sh`.
