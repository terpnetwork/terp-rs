# FE specialist — review, then tune, then we exercise

**To:** static HTML specialist (`permissionless.money`, `terp.network`). **Not dao-dao-ui.**  
**From:** DREGG / ZEC lab. We do **not** edit mint HTML.  
**Your loop:** `/workflow dregg-fe-zec-zallet {"mode":"observe"}` then `{"mode":"craft"}`  
**When you ping:** we exercise the **public** mint page against live DREGG. Do not wait for us to ship HTML.

Paste-ready:

> Review mint against DREGG only (not ztreamer/LWD). Contract: `FE-SPECIALIST-REVIEW.md` + `FE-ZTREAMER-ZALLET.md`. Helper already in-tree: `websites/permissionless.money/lib/dregg-svg-checkout.js` (`LIVE_RAILS = ['zec','native']`, hide null `zec_uri`, `zec_uri` as-is for Zallet). Wire `mint.html` / `svg.html` to that helper. ETH/Base/BTC stay parked. Never ship `127.0.0.1:8421`. When dist is honest, ping DREGG — we exercise `https://permissionless.money/mint` + `https://dregg-svg.permissionless.money`. Native still mints if `zec_uri` is null.

---

## What we already locked (do not redo)

| Layer | Status | Your job |
|---|---|---|
| CompactTxStreamer | **ztreamer** lab-sound (`vendor=Ztreamer`, LWD down) | **None.** Page never talks to `:19067` / `:9067` / `:8232` |
| DREGG HTTP | `https://dregg-svg.permissionless.money` | TLS only: health, prices, pay-targets, status |
| ZIP-321 | Server builds `zec_uri` | Render **as-is** (QR + `zcash:`). Zallet opens that URI |
| Rails | ZEC + native | Do not resurrect ETH/Base/BTC Pay |
| Helper | `lib/dregg-svg-checkout.js` | **Use it.** Do not fork a second client |

Lab loopback (`127.0.0.1:8421` fed, `:19067` ztreamer) is **ops**, not a browser origin.

---

## Review first (observe)

1. Read `FE-ZTREAMER-ZALLET.md` (three paragraphs), `FE-ZEC-NATIVE-ONLY.md`, `FE-HANDOFF.md` § Zcash + mint execute.
2. Grep `mint.html`, `svg.html`, `mint-svg.js`, `dregg-svg-checkout.js` on **both** sites (prefer `websites/`).
3. Confirm the page **imports** the helper (or equivalent). A comment in `mint-svg.js` is not enough if Pay still shows four rails.

`/workflow dregg-fe-zec-zallet {"mode":"observe"}` — gaps only, no edits.

---

## Then tune (craft — you)

Wire UI to the helper. Do not invent SDK / Pedersen / UA.

| Must | Must not |
|---|---|
| `LIVE_RAILS = ['zec','native']` | Render `eth_addr` / `btc_addr` / `base_*` |
| Hide ZEC if `hideZec(zec_uri)` | Fake a UA when `zec_uri` is null |
| `assignZecUri` for QR + `href` | Append `terp1` to memo / URI / QR |
| Pay only if `payAllowed(health, targets)` | Treat `/v1/prices` `ok` as Pay |
| Native: `update_extension.msg.mint` `{amnt,alloc}` on **collection** | `migrate` minter 72; `requestMintGrant`; POST `/v1/deposit` |
| Fresh pay-targets every checkout; poll status 4–8s | Cache dests; ship loopback DREGG |

If `zec_uri` is null on **public** pay-targets: hide ZEC, **native still works**. That is the honest product until house UFVK is up.

---

## Ping us to exercise (done-when)

Reply with:

1. Paths you changed (mint / svg / helper).
2. Confirm: ZEC hidden on null URI; native Pay still shown when `checkout_ready`.
3. Confirm: no `127.0.0.1:8421` in shipped HTML.
4. Dist built; Phase G is **oline** (`HANDOFF-OLINE-STATIC-MINT.md`) — usb2 ≠ live.

We then exercise:

- `GET https://dregg-svg.permissionless.money/health` + pay-targets
- Public `/mint`: only ZEC + native; Zallet-shaped `zcash:` when URI exists; Pay off when `checkout_ready` is false

Do not block native mint on ZEC. Do not ask infra to edit the page.
