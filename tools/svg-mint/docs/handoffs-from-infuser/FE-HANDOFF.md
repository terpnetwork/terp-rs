# FE handoff: permissionless SVG mint (Pedersen + DREGG)

**Current product cut (deploy now):** **ZEC + native TERP only.** Comment out ETH / Base / BTC in the static HTML. Specialist packet: **`FE-ZEC-NATIVE-ONLY.md`**. Infra for those three rails is parked (no live house watchers). Do not send users there.

**Status 2026-08-22 — wire this in.** Chain and checkout contract are ready for FE. DREGG public origin is still ops (not a blocker for mint-msg + state.json).

| Piece | Live |
|---|---|
| Chain | `morocco-1` |
| Minter instance | `terp188jpppxtfpf0vvt5gypdj52hk2xuswwcn38auczzxv20fcxz3kss9fhlru` **code 72** (no migrate; do not call `migrate`) |
| Minter `svg_code_id` | **83** (`update_code_id` tx `0B247C4007F7756BF262A0C0951A5A3748DA8FFE1111F2DEB0B0A231B4106422`) |
| Existing hopalong collection | `terp1vp7smemrv846hpsyqrzlnqapmu3sv7hyygf4rrnw3ls26ezk864qupr5zt` still **code 71** |
| New collections | minter instantiates **code 83** |
| cw-orch `state.json` | groot + groot2 `morocco-1.code_ids.cw721_svg = 83`; addresses unchanged |
| Static HTML SSOT | `terp.network/public/state.json` (`cw-svg-minter` **72**, `cw721_svg` **83**) and `lib/mint-svg.js` `CHAINS.morocco-1`. Not dao-dao-ui. |
| HashMerchant | Runtime **authz** → temp HM → one-time customer feegrant (register **gas** + mint **gas**). Runtime is never `fee.granter`. `POST /onboard/v1/mint-grant` is **404** (HM does not buy the NFT). See `HASHMERCHANT-AUTHZ.md`. |
| Native mint | 100 TERP or 100 THIOL (`100000000` uterp\|uthiol) |
| Rails | **`GET {DREGG}/v1/prices`** — do not hardcode. ETH/Base `0.0005`, BTC `0.000015`, ZEC `0.0015`; first 1000 rails **BOGO** |
| Privacy | pay-targets `memo_hex` starts `6431`; **no `terp1` on ETH/Base/BTC/ZEC** |
| Art | hopalong template + entropy slots `A,B,CX,CY,DX,DY` — not 1×1 placeholder |
| Mint execute | **on the collection**, never the minter. Wasm **rejects** top-level `{mint:…}` (`IncorrectEntrypoint`). Wrap `SvgExecuteMsgExt::Mint` in `update_extension`. Lab e2e (2026-08-22) proved: `{"update_extension":{"msg":{"mint":{"amnt":1,"proof_hashes":[],"alloc":0,"ext":null}}}}` (fields are `amnt`/`alloc`, not `amount`/`allocation`). Do not `migrate` minter code 72. |

`{DREGG}` = **`https://dregg-svg.permissionless.money`**. Hide ZEC if `zec_uri` is null. **This deploy: hide ETH/Base/BTC always** (`FE-ZEC-NATIVE-ONLY.md`).

---

For the **static HTML** teammate (`permissionless.money`, `terp.network` — `soon/mint.html`, `pages/svg.html`, `lib/mint-svg.js`). Not dao-dao-ui. This is the **checkout contract** with the DREGG runtime. Not HashMerchant mint. Not a new TypeScript SDK — reuse Keplr / local-wallet / passkey authenticator / ZIP-321 wallet deep links you already have. Gap review: `FE-RUNTIME-REVIEW.md` → `/workflow dregg-fe-runtime-review`.

**Privacy rule:** the user’s Terp mint address (`terp1…`) is sent **only** to DREGG over HTTPS. Ethereum, Base, Bitcoin, and Zcash payments carry a **blinded Pedersen commitment** (`memo_hex`), never `terp1`. Each checkout gets **fresh** ETH/BTC/ZEC receive addresses. `base_addr` is the **same 0x** as `eth_addr` (BIP44 coin 60; Base does not use a distinct HD index).

`DREGG` = `https://dregg-svg.permissionless.money`. HashMerchant = `https://hash.terp.network` (also `https://onboard.permissionless.money`). Do not hard-code loopback. **This deploy:** `FE-ZEC-NATIVE-ONLY.md`.

---

## 1. Identity (before any payment)

The page must have a `terp1…` mint dest. Two ways only.

### A. Keplr

Enable Terp (`morocco-1` production, `120u-1` lab). `getKey` / `getOfflineSigner` → bech32 is the mint dest.

### B. Local key + passkey authenticator

1. Generate a Terp key in the page (existing local-wallet primitive).
2. Register a smart-account authenticator authorized by WebAuthn. **Reuse the existing passkey → authenticator flow.** Do not add `fe/*.ts` wrappers.
3. **Feegrant is only for that registration.** HashMerchant **onboard** `POST /onboard/v1/*-grant` (not a mint grant). After registration, the user pays ETH/BTC/ZEC themselves.

Do not start checkout until `terp1` exists.

---

## 2. Talk to DREGG (TLS only)

Base: `{DREGG}` e.g. `https://dregg.example` (ops). All JSON.

### `GET {DREGG}/health`

When checkout is open: HTTP **200** `{ "ok": true, "service": "dregg-svg", "checkout_ready": true, ... }`. When closed/unfunded: HTTP **200** `{ "ok": false, "checkout_ready": false, "pay": false, "reasons": [...], ... }` — liveness stays 200 so the sidecar can come up before LCD funds; never `ok: true` while closed. Reasons include `minter_underfunded`, `minter_balance_unknown`, `sold_out`, `collection_unbound`, **`zec_tip_drift`**. Also: `minter_funded`, `remaining_supply`, `minted`, `max_supply`, `mint_batch_size`, `mint_timeout_secs`, **`zec_tip`**.

`zec_tip` is the house-vs-public Zcash height circuit (max **2** blocks). Hide Pay / ZEC QR / native execute when `zec_tip.ok === false` (same as `checkout_ready === false` — the runtime already folds it in). Typical closed body:

```json
"zec_tip": {
  "ok": false,
  "configured": true,
  "private_height": null,
  "public_height": 3120000,
  "drift": null,
  "max_drift": 2,
  "reason": "private_unreachable"
}
```

`reason` is `drift` | `private_unreachable` | `public_unreachable` | `private_unconfigured`. No refunds: the user has not paid yet. HashMerchant passkey remains **gas only**.

**Pay only if `checkout_ready === true` and `GET /v1/pay-targets` is HTTP 200.** Do **not** treat `/v1/prices` `ok` as a Pay signal.

Do **not** send users to a payment address when `checkout_ready` is false — the runtime cannot mint.

### `GET {DREGG}/v1/prices`

**Amounts only** (labels, wei/sats/zat, native). **Do not hardcode** ETH/BTC/ZEC/Base prices in the page. `ok: true` means the catalog payload is valid, **not** that Pay is allowed. The body also includes live `checkout_ready` from `checkout_now`; still require pay-targets **200** before showing Pay.

```json
{
  "ok": true,
  "checkout_ready": true,
  "source": "dregg",
  "eth": { "wei": "500000000000000", "ether": "0.0005", "chain_id": 1 },
  "base": { "wei": "500000000000000", "ether": "0.0005", "chain_id": 8453 },
  "btc": { "sats": 1500, "btc": "0.000015" },
  "zec": { "zat": 150000, "zec": "0.0015" },
  "native": { "amount": 100000000, "denom": "uthiol", "display": "100" },
  "bogo_cap": 1000
}
```

Checkout still uses `pay-targets` for **addresses + memo**. Those fields (`eth_wei`, `btc_sats`, `zec_zat`, `base_wei`) must match `/v1/prices`. If they disagree, treat as a server bug.

### `GET {DREGG}/v1/pay-targets?recipient={terp1}`

Call **once per checkout**, after rail choice (or once then let the user pick a rail from the payload). **Do not cache** the response across checkouts. **Do not put `terp1` in analytics, Sentry breadcrumbs, or query logs** if you can help it; prefer POST later if we add it.

Example response:

```json
{
  "ok": true,
  "eth_addr": "0x…",
  "eth_vault": "0x… or null",
  "eth_wei": "500000000000000",
  "base_addr": "0x… (same as eth_addr)",
  "base_vault": "0x… or null",
  "base_wei": "500000000000000",
  "base_chain_id": 8453,
  "btc_addr": "bc1… or bcrt1…",
  "btc_sats": 1500,
  "zec_ua": "u1…",
  "zec_diversifier": "hex 11 bytes",
  "zec_zat": 150000,
  "native_amount": 100000000,
  "native_denom": "uthiol",
  "bogo": true,
  "bogo_remaining": 1000,
  "zec_uri": "zcash:u1…?amount=0.0015&memo=…",
  "zip321": true,
  "memo_commit": "64 hex chars (Ristretto C)",
  "memo_hex": "6431…  (ASCII d1 + 32-byte C, hex)",
  "hd_index": 3,
  "or": true,
  "pending_ttl_secs": 1200,
  "memo": "put memo_hex in the payment — never terp1"
}
```

**QA this payload in the page before sending a chain tx:**

| Check | Fail if |
|---|---|
| `ok === true` | any other |
| `zip321 === true` | missing (stale DREGG) |
| `memo_hex` starts with `6431` (hex of `d1`) | empty or looks like `terp1` |
| `zec_uri` if present | does not start with `zcash:`, or contains `terp1` |
| `eth_addr` / `btc_addr` | empty |
| Same as last checkout | reused `eth_addr` / `btc_addr` / `memo_commit` — call pay-targets again |
| Unpaid past `pending_ttl_secs` (default 1200) | dest is abandoned; **select deposit again** (new pay-targets). Status stays `phase: null` |

The JSON **must not** include `recipient`. If it does, treat as a server bug and do not pay.

### `GET {DREGG}/v1/status?recipient={terp1}`

Poll every 4–8s after the user broadcasts.

```json
{ "ok": true, "checkout_ready": true, "recipient": "terp1…", "n_deposit": "…", "phase": "Entitled", "qty": 2, "mint_batches": [2] }
```

`phase`: `null` → `Deposited` → `Entitled` → `Settled` / `Refunded`.

`qty` is NFTs owed for that payment (`floor(paid/unit)` × BOGO). `mint_batches` is how the runtime splits the mint txs. If entitled and no mint is observed within `mint_timeout_secs` (default 20 min), DREGG refunds.

- `null` / missing: still waiting for the rail (keep polling, show “waiting for payment”).
- `Entitled` / `Settled`: show minted / minting; stop aggressive poll.
- Do **not** display `n_deposit` to the user.

Stop after ~20 min with “we have not seen the payment yet; keep this tab, the memo is one-time.”

---

## 3. Rails (user pays once; OR)

**This deploy:** two controls — **ZEC** and **native**. Comment out ETH / Base / BTC (`FE-ZEC-NATIVE-ONLY.md`). Historical four-button spec below is parked; do not ship it.

UI (parked): four buttons (ETH | Base (EVM) | BTC | ZEC). Selecting one **does not** call pay-targets again unless the previous payload was consumed or is stale. Hide **Base** if `base_addr` is missing.

### Ethereum

1. Use `eth_wei` as `value`.
2. If `eth_vault` is set: `lock(bytes32 nDeposit, uint64 deadline, string memo)`  
   - `memo` = **`memo_hex` string** (the hex, not `terp1`).  
   - `nDeposit` = random `bytes32` from the page (or `0x11…` only in lab).  
   - `deadline` = unix now + 7d (or vault policy).
3. If vault is null: send `eth_wei` to **`eth_addr`** with calldata / data = `0x` + `memo_hex` (same bytes).
4. Never put `terp1` in `data`.

Wallets: injected EVM (MetaMask / Rabby). Show amount in ETH from `eth_wei / 1e18`.

### Base (EVM)

Same as Ethereum: `lock(memo_hex)` on `base_vault` (SvgMintRailVault ABI), or send `base_wei` to `base_addr` with data = `0x` + `memo_hex`. Wallet chain id **`base_chain_id` (8453 lab/prod)**. Never `terp1` in calldata. Hide the button if `base_addr` is missing.

### Bitcoin

1. Pay **`btc_sats`** to **this checkout’s** `btc_addr` only.
2. Add an **OP_RETURN** output whose payload is UTF-8 or raw bytes of **`memo_hex`** (the `6431…` hex string is fine).
3. QR: BIP21 `bitcoin:{btc_addr}?amount={btc}` **plus** an on-screen “include this OP_RETURN” copy of `memo_hex`. Many mobile wallets cannot attach OP_RETURN — show a desktop/Sparrow path or “advanced” note. Do not fall back to embedding `terp1`.

### Zcash (ZIP-321)

1. Prefer **`zec_uri` as-is** (`window.location` / `zcash:` deep link / QR).
2. The URI already has `amount` and `memo` = ZIP-321 base64url of **ASCII hex** Pedersen (`d1`||C, starts `6431` after the wallet decodes). Zallet/Zold show hex, not raw bytes. **Do not append `terp1`.**
3. If you rebuild the URI: `zcash:{zec_ua}?amount={decimal}&memo={from server}`. Use the server `zec_uri`; do not invent memo encoding.
4. If `zec_uri` is null: hide the ZEC button (wallet/streamer not up). Do not fake a UA.

---

## 4. Suggested screen flow

1. **Mint dest** — Keplr connected or local+passkey done. Show truncated `terp1` only in this Terp-origin UI.
2. **Pay** — ETH | Base (EVM) | BTC | ZEC. Fetch pay-targets. Show **only** the chosen rail’s address + amount + `memo_hex` (truncated, copy button).
3. **Privacy line** (always visible on Pay):  
   “Your Terp address never goes on Ethereum, Base, Bitcoin, or Zcash. This payment uses a one-time commitment.”
4. **Broadcast** — wallet / QR / ZIP-321.
5. **Waiting** — poll status. No txid required from the user.
6. **Done** — NFT on Terp. Paid mint **collection** `terp1vp7sm…` (code 71). New drops from minter `terp188jppp…` use svg **code 83**. Query `code_id` on the minter if you create a collection in-page.

Do not write ETH/Base/BTC/ZEC txids into NFT metadata.

---

## 5. HashMerchant boundary

| Allowed | Forbidden |
|---|---|
| Onboard feegrant for **passkey authenticator add** | `requestMintGrant` / mint feegrant |
| Showing that we cover **gas to attach passkey only** | Implying HashMerchant pays the NFT |
| Keplr / local wallet | New authz SDK |

HashMerchant runtime `terp14ekg…` is **not** DREGG. Do not point checkout at HashMerchant.

---

## 6. Copy

- “Connect Keplr, or create a Terp key here and lock it to your passkey. Then pay with ETH, Base, Bitcoin, or Zcash.”
- “Your Terp address stays off those chains. The payment only carries a one-time commitment.”
- “Each checkout uses a new receive address.”
- Passkey: “We cover gas only to attach your passkey to this account, not to buy the NFT.”

---

## 7. Do not

- Put `terp1` in ETH/Base calldata, BTC OP_RETURN, ZIP-321 memo, QR extra, or tweet intents.
- Reuse `btc_addr` / `eth_addr` / `base_addr` / `zec_uri` from a previous `pay-targets` call.
- Build Pedersen in the browser (DREGG already committed `C`; you only relay `memo_hex` / `zec_uri`).
- Call `/v1/deposit` or `/v1/entitle` from the browser (federation QC; not a public mint API).
- Refund UI.
- Lab `120u-1` minter `terp1wug8sewp6…` as production.

---

## 8. FE QA checklist

- [ ] Health `ok: false` / `checkout_ready` false → Pay disabled. Pay only if `checkout_ready` **and** pay-targets HTTP 200. `/v1/prices` `ok` is amounts, not Pay.
- [ ] `reasons` includes `zec_tip_drift` or `zec_tip.ok === false` → hide ZEC **and** native Pay (house node off tip / unreachable). Do not invent refunds.
- [ ] Pay-targets twice → different `eth_addr`, `btc_addr`, `memo_commit`.
- [ ] `zec_uri` has no substring `terp1`.
- [ ] ETH/Base tx input / BTC OP_RETURN / ZEC memo searchable: no `terp1`.
- [ ] Status poll reaches `Entitled` or `Settled` after a real payment (or stays `null` honestly).
- [ ] Passkey path never hits a mint grant.
- [ ] Mobile + desktop for ZIP-321 QR.

DREGG server work (Pedersen, HD, ZEC view-wallet) is owned by the runtime, not the page. The page’s job is: **TLS recipient in, commitment memo out, poll status.**
