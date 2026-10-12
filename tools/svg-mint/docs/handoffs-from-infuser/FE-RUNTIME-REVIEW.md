# Static HTML specialist: DREGG runtime contract + gap review

For the teammate already shipping **static HTML/JS** on `permissionless.money` and `terp.network`. **Not** dao-dao-ui, **not** Next.js, **not** SDA modules. Reuse Keplr / existing `lib/*.js` / ZIP-321. Do not add a TypeScript SDK.

Spawn this **before** wiring checkout against the stable DREGG lab runtime (groot-wan e2e `live:true` 2026-08-22).

---

## Spawn the review

From a grok session whose workspace can see the site trees (repo root `terp-core/` is enough):

```
/workflow dregg-fe-runtime-review
```

Scripts (same file, two catalogs):

- `terp-core/.grok/workflows/dregg-fe-runtime-review.rhai` (what `/workflow` resolves)
- `crates/.grok/workflows/dregg-fe-runtime-review.rhai` (DREGG pack copy)

Read-only. Greps the HTML/JS below against this contract; skeptics must quote code. `sound: true` only if **zero confirmed blockers**. An empty scan is **not** a ship stamp — it usually means the session could not open the site trees.

Do **not** run `fe-module-dev-loop` for this work. That workflow is dao-dao-ui.

---

## Files you own (in scope)

Prefer the **websites/** copies if those are what you deploy. Repo-root `permissionless.money/` and `terp.network/` can lag (e.g. root `lib/` may lack `mint-svg.js`).

| Surface | Paths |
|---|---|
| Native mint page | `websites/permissionless.money/soon/mint.html` · `permissionless.money/soon/mint.html` · `terp.network/soon/mint.html` |
| Collection creator (hopalong / not rail checkout) | `permissionless.money/soon/svg-mint.html` · `websites/permissionless.money/soon/svg-mint.html` |
| terp.network SVG page | `terp.network/pages/svg.html` · `websites/terp.network/pages/svg.html` |
| Mint execute helper | `lib/mint-svg.js` under both site trees (`mintSvgNft`, `requireLiveMintGrant`, `CHAINS`) |
| HashMerchant onboard | `lib/onboard-client.js` (`requestMintGrant`) |
| Minter TS client (create collection only) | `terp.network/lib/cw-svg-minter.js` |
| Chain SSOT on the static site | `terp.network/public/state.json` (`cw-svg-minter` code **72**, `cw721_svg` code **83**) |
| Runtime contract | `FE-ZEC-NATIVE-ONLY.md` (this deploy) · `FE-HANDOFF.md` · `FE-PRICES-HANDOFF.md` · this file |

**Out of scope:** `websites/dao-dao-ui/**`, `packages/stateful/**`, TSX modules, `terp-state.json` inside dao-dao-ui. Drop those findings.

---

## Runtime that is already proven (do not re-litigate)

Lab e2e on groot-wan: `live: true`, `mint_engine: cw-orch`, `queried_minted: 1`, mint tx `3114320C…`. **No host `terpd`.** Federation is HTTP; mint is Keplr/cw-orch **gRPC execute on the collection**.

| Fact | Implication for the page |
|---|---|
| `GET /health` HTTP **200** even when closed | Never use HTTP status as Pay. Use `ok` **and** `checkout_ready` **and** `pay`. |
| `GET /v1/prices` `ok: true` is catalog only | Prices are labels/amounts. **Not** Pay. Do not hardcode 0.0005 / 1500 sats / 100 TERP. |
| `GET /v1/pay-targets?recipient=terp1` **503** when not `checkout_ready` | Pay button exists only after **200** pay-targets. |
| Underpay is ignored (no slot) | UI **must** send exact `eth_wei` / `base_wei` / `btc_sats` / ZIP-321 amount. |
| `POST /v1/deposit` and `/v1/entitle` are **not** public | Browser never calls them. |
| Mint wasm | Collection execute: `update_extension.msg.mint` with `amnt`, `alloc`, `proof_hashes`, `ext`. Top-level `{mint:{amount,allocation}}` **fails** (`IncorrectEntrypoint`). |
| Minter `terp188jppp…` code **72** | **No migrate.** New collections via minter instantiate code **83**. |
| HashMerchant mint-grant | **404**. Passkey **feegrant only**. Delete `requireLiveMintGrant` / `requestMintGrant` from checkout. |
| `{DREGG}` | `https://dregg-svg.permissionless.money` |

Proven collection mint JSON:

```json
{"update_extension":{"msg":{"mint":{"amnt":1,"proof_hashes":[],"alloc":0,"ext":null}}}}
```

---

## Pay gate (blocker if violated)

Show Pay **iff all** of:

1. User has a `terp1` (Keplr or local+passkey).
2. `GET {DREGG}/health` → `checkout_ready === true` (and `ok === true`).
3. `GET {DREGG}/v1/pay-targets?recipient={terp1}` → **HTTP 200** and `ok === true`.
4. Amounts on pay-targets **equal** `/v1/prices` for that rail.

Disable Pay on: health `ok:false`, `checkout_ready:false`, pay-targets 503 / `reserved_sold_out`, prices fetch used as a substitute for those gates.

Rail checkout is a **new** surface on these sites (today native mint does not talk to DREGG). Until it exists, native mint still must not claim ETH/BTC/ZEC Pay.

---

## Privacy (blocker if violated)

- `terp1` only to DREGG over HTTPS (`recipient=` query).
- Never in ETH/Base calldata, BTC OP_RETURN, ZIP-321 memo, QR extra, Sentry breadcrumbs, analytics.
- `memo_hex` starts `6431`. Fresh dests every checkout (no cache of `eth_addr`/`btc_addr`/`zec_uri`/`memo_commit`).
- `base_addr` is the same 0x as `eth_addr` (hide Base if missing). Hide ZEC if `zec_uri` is null.

---

## What the review will hunt (start here)

These shapes already live in the static trees — quote them, do not invent dao-dao paths:

1. **`lib/mint-svg.js` `mintSvgNft`** — top-level `{mint:{amount, allocation, proof_hashes}}`. Wasm wants `update_extension.msg.mint` with **`amnt` / `alloc` / `ext`**.
2. **`lib/mint-svg.js` `requireLiveMintGrant`** + **`soon/mint.html`** calling it — HashMerchant mint-grant is 404. Passkey feegrant only.
3. **`terp.network/pages/svg.html` `buildMintExecuteMsg`** — comment still says `ExecuteMsg::Mint { amount, allocation, proof_hashes }`. Same wrap + field names as (1).
4. **Hardcoded morocco-1 amounts / no `GET {DREGG}/v1/prices`.** Native 100 TERP must come from prices `native.amount` + `native.denom`.
5. **No `checkout_ready` / pay-targets gate** if a page offers ETH/Base/BTC/ZEC.
6. **`CHAINS['120u-1']` or lab `terp1wug8sewp6…` used as production.**
7. **`cw-svg-minter.js` used to mint NFTs** — that client is create-collection / update_code_id only. Mint is on the **collection**.
8. Browser `POST /v1/deposit` or `/v1/entitle`.
9. User-editable underpay on a rail.

Full checkout narrative: `FE-HANDOFF.md`. Amounts: `FE-PRICES-HANDOFF.md`.
