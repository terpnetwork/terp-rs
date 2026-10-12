# FE specialist — ship mint with **ZEC + native TERP only**

**To:** static HTML teammate (`permissionless.money`, `terp.network`). **Not dao-dao-ui.**  
**From:** DREGG / infra.  
**Why now:** ETH, Base, and Bitcoin house/porch are **parked**. Live payments we will actually see are **Zcash (ZIP-321)** and **native TERP/THIOL**. Comment those three rails out of the UI so we can deploy checkout and mint without sending users to dead watchers.

Do **not** delete DREGG JSON fields. The runtime still returns `eth_addr` / `btc_addr` / `base_*`. **Do not render them. Do not deep-link wallets to them. Do not show Pay on those rails.**

Full contract (health, pay-targets, mint wrap): `FE-HANDOFF.md`. Amounts: `FE-PRICES-HANDOFF.md`. Files you own: `FE-RUNTIME-REVIEW.md`. ZEC + Zallet after ztreamer: **`FE-ZTREAMER-ZALLET.md`** → `/workflow dregg-fe-zec-zallet`.

**You build the static.** Oline edge mirrors it to Phase G — **usb2 success ≠ live site.** See `HANDOFF-OLINE-STATIC-MINT.md`.

---

## Live origins (wire these — verified 2026-08-26)

| Use | Origin | Proven |
|---|---|---|
| **DREGG** checkout | `https://dregg-svg.permissionless.money` | `GET /health` → `ok: true`, `checkout_ready: true`, `pay: true`, `minter_funded: true` |
| Prices | `GET {DREGG}/v1/prices` | `native.display` / `zec.zec` — **not** a Pay signal |
| Pay-targets | `GET {DREGG}/v1/pay-targets?recipient={terp1}` | HTTP 200, `memo_hex` starts `6431`. `zec_uri` may be **null** (hide ZEC) |
| Status poll | `GET {DREGG}/v1/status?recipient={terp1}` | every 4–8s after pay |
| **HashMerchant** passkey gas | `https://hash.terp.network` (same origin: `https://onboard.permissionless.money`) | `GET /onboard/status` → `ready: true`, `broadcast: true`, `fee_granter` ≠ `runtime` |
| Passkey grant | `POST {HM}/onboard/v1/passkey-grant` | gas only |
| Mint grant | `POST {HM}/onboard/v1/mint-grant` | must **404** — HashMerchant does not buy the NFT |

Do not point checkout at HashMerchant. Do not use `http://127.0.0.1:8421` or `:19090` in the shipped HTML.

Chain: **`morocco-1`**. Minter `terp188jpppxtfpf0vvt5gypdj52hk2xuswwcn38auczzxv20fcxz3kss9fhlru` (code **72**). Collection `terp1vp7smemrv846hpsyqrzlnqapmu3sv7hyygf4rrnw3ls26ezk864qupr5zt` (svg code **71** live; new collections **83**). `state.json` SSOT: `cw-svg-minter` 72, `cw721_svg` 83.

---

## Product cut (this deploy)

| Show | Hide / comment out |
|---|---|
| **ZEC** — ZIP-321 `zec_uri` (QR + `zcash:` link) | **ETH** (MetaMask / vault `lock`) |
| **Native** — 100 TERP or 100 THIOL, execute on the **collection** | **Base** (chain 8453) |
| | **Bitcoin** (BIP21 / OP_RETURN) |

If `zec_uri` is **null**, hide ZEC (honest). Still ship **native** so mint is not blocked on LWD.

`{DREGG}` = `https://dregg-svg.permissionless.money`

---

## Files to touch (comment out, do not invent a new SDK)

Prefer **`websites/`** copies if those are what you deploy.

| Surface | Paths |
|---|---|
| Mint / Pay UI | `soon/mint.html`, `pages/svg.html` (both sites) |
| Rail buttons / amounts | whatever maps four rails today — leave **two**: ZEC, native |
| Helper | `lib/mint-svg.js` (`mintSvgNft`, `CHAINS.morocco-1`) |
| Prices labels | only `prices.zec` and `prices.native` |

Comment with `// parked: ETH/Base/BTC — no live house watchers` so we can restore later.

---

## Gates (unchanged — still required)

Pay / ZEC QR / native execute **only if**:

1. `GET {DREGG}/health` → `checkout_ready === true` (HTTP is **always 200**; ignore status code as Pay).
2. `zec_tip.ok === true` (or `zec_tip.configured === false`). If `reasons` includes `zec_tip_drift` or `zec_tip.ok === false`, **hide all payment UI** (ZEC QR and native). House Zakura is off the public tip (max 2 blocks) or unreachable — DREGG would not see a ZEC paste. No refund path.
3. `GET {DREGG}/v1/pay-targets?recipient={terp1}` → HTTP **200** `ok: true`.
4. User has a `terp1` (Keplr **or** local key + passkey). HashMerchant covers **passkey gas only**, never the NFT.

`GET /v1/prices` `ok` is **labels only**, not Pay. Use `zec.zec` / `zec.zat` and `native.display` / `native.amount` / `native.denom`. Do not hardcode `0.0015` or `100`.

---

## Native TERP / THIOL

1. Amount from prices: `native.amount` (default `100000000` = 100 TERP\|THIOL).
2. Execute **on the collection**, never the minter. Wrap mint in `update_extension` (`FE-HANDOFF.md` § mint execute). Lab-proven:

```json
{"update_extension":{"msg":{"mint":{"amnt":1,"proof_hashes":[],"alloc":0,"ext":null}}}}
```

3. Collection in production: hopalong `terp1vp7sm…` (code 71). New drops from minter `terp188jppp…` use svg code **83**. `state.json` SSOT: `cw-svg-minter` **72**, `cw721_svg` **83**.
4. Do not `migrate` minter code 72. Do not `requestMintGrant`.

---

## ZEC (ZIP-321)

1. After pay-targets **200**, if `zec_uri` is a string starting `zcash:` with `memo=` and **no** `terp1` → show QR + open-wallet.
2. Use **`zec_uri` as-is**. Do not append `terp1`. Do not invent memo encoding.
3. `zec_uri === null` → hide ZEC. Do **not** set a fake UA. Native remains.
4. Poll `GET {DREGG}/v1/status?recipient={terp1}` every 4–8s. No txid from the user.
5. If still `phase: null` after `pending_ttl_secs` (default 1200): **select deposit again** (new pay-targets).

Privacy: `terp1` goes to DREGG over HTTPS only. ZEC memo is Pedersen `memo_hex` (`6431…`), never the bech32.

---

## Copy (replace four-rail strings)

- “Connect Keplr, or create a Terp key and lock it to your passkey. Pay with Zcash or native TERP.”
- “Your Terp address never goes on Zcash. The payment carries a one-time commitment.”
- Passkey: “We cover gas only to attach your passkey, not to buy the NFT.”
- Do **not** mention ETH, Base, or Bitcoin on this deploy.

---

## QA before you ship the static files

- [ ] No visible ETH / Base / BTC button, address, QR, or MetaMask prompt.
- [ ] Native mint still works with `checkout_ready` + collection `update_extension` wrap.
- [ ] ZEC shown only when `zec_uri` is non-null, starts `zcash:`, has no `terp1`.
- [ ] Pay disabled when `checkout_ready` is false.
- [ ] Two pay-targets calls → new `zec_uri` / `memo_commit` (do not cache).
- [ ] Passkey path never hits a mint grant.
- [ ] Mobile + desktop ZIP-321 QR.

---

## Out of scope (do not wait on these)

- dao-dao-ui
- Akash BTC/ETH/Base porch (closed / not bid)
- House bitcoind / geth
- Uncommenting ETH/Base/BTC without a new infra handoff

You ship `soon/mint.html` / `pages/svg.html` / `lib/mint-svg.js` on `permissionless.money` and `terp.network`. After deploy, ping us with the live URL — we only verify `{DREGG}` health and that ETH/Base/BTC are not on the page.
