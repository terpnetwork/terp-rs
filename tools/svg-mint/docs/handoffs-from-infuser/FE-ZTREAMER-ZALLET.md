# FE handoff — ZEC mint via ztreamer + Zallet (three paragraphs)

**To:** static HTML teammate (`permissionless.money`, `terp.network` — `soon/mint.html`, `pages/svg.html`, `lib/mint-svg.js`). **Not dao-dao-ui.**  
**From:** DREGG / ZEC lab.  
**Ticket (send this):** `FE-SPECIALIST-REVIEW.md`  
**Workflow (FE specialist only):** observe then craft — `/workflow dregg-fe-zec-zallet {"mode":"observe"}`  
Infra/DREGG does **not** edit mint HTML. `mode=observe` is the only mode for this desk.  
**Contract still applies:** `FE-ZEC-NATIVE-ONLY.md` · `FE-HANDOFF.md` · `FE-PRICES-HANDOFF.md`  
**`{DREGG}`** = `https://dregg-svg.permissionless.money` (lab loopback on groot-wan is `http://127.0.0.1:8421` only for you, never ship it).

---

We replaced stock `lightwalletd` as the CompactTxStreamer that light wallets (Zallet / Zashi / `zcash-devtool`) sync from. The indexer is **ztreamer** (Zakura under the hood, `GetLightdInfo.vendor = Ztreamer`). Lab cargo on groot-wan (`dregg-zec-ztreamer`) is **sound**: `live=true`, `compact_vendor=Ztreamer`, `lwd_absent=true`, `compact_grpc=127.0.0.1:19067`, no LWD container. Your page does **not** talk to ztreamer or Zakura. You still talk **only** to DREGG over TLS: health, `/v1/prices` (labels), `/v1/pay-targets?recipient={terp1}` (ZIP-321), `/v1/status` poll. When DREGG can derive a UA, `zec_uri` is a `zcash:` payment request with amount + Pedersen memo — that URI is what Zallet opens. Hide the ZEC control when `zec_uri` is null; do not invent a UA.

Tune the already-shipped **ZEC + native** cut until it is stable: `LIVE_RAILS = ['zec','native']` (ETH/Base/BTC stay commented — no live house watchers for those). Pay only if `checkout_ready === true` **and** pay-targets HTTP 200; `/v1/prices` `ok` is not Pay. Use `zec_uri` **as-is** for QR + `zcash:` deep link (Zallet/Zashi). Never put `terp1` in the URI, memo, QR extras, or analytics. Fresh pay-targets every checkout; poll status every 4–8s after pay; native mint still wraps `update_extension.msg.mint` `{amnt,alloc}` on the **collection**. Do not POST `/v1/deposit`. Do not use HashMerchant mint-grant (must 404). Do not point checkout at `:8232` or a public LWD.

While you integrate, we rebuild **groot-wan** so the lab indexer + DREGG runtime sit in front of the same CompactTxStreamer Zallet uses: ztreamer on `127.0.0.1:19067`, `svg-mint-fed` with `ZCASH_DEVTOOL_SERVER` pointed at that port (not Go LWD). When `GET {DREGG}/v1/pay-targets` returns a non-null `zec_uri` (`zcash:…`, `memo=`, no `terp1`), turn the ZEC button on. Until then native TERP/THIOL still mints. Do not wait on the multi-day LWD rebuild; do not publish loopback in the static HTML; Phase G publish is still `HANDOFF-OLINE-STATIC-MINT.md` after the page is honest.

---

## Files / workflow (ops, not extra prose)

| | |
|---|---|
| FE loop | `/workflow dregg-fe-zec-zallet` |
| Lab indexer + fed | `/workflow dregg-groot-wan-ztreamer` · `scripts/spawn-groot-wan-ztreamer.sh` |
| Lab streamer | `127.0.0.1:19067` vendor **Ztreamer** |
| Zallet | CompactTxStreamer host `127.0.0.1:19067` (no TLS on loopback) |
| Hide ZEC | `zec_uri == null` |
