# Handoff — publish mint static to oline Phase G (not usb2)

**To:** oline static-edge / MinIO mirror person  
**From:** HTML / DREGG  
**Do not** close leases, change CF, or rewrite Host maps for this publish.

## Why

`npm run build` / mirror to **groot2 `usb2`** updates local MinIO only. The browser hits **Phase G** `static` via oline nginx Host maps. Until you mirror into Phase G, `https://permissionless.money/mint` stays stale.

## Split

| HTML teammate | Oline edge |
|---|---|
| Build → `websites/permissionless.money/dist/` (or usb2 as staging) | `mc mirror` into Phase G `static/permissionless.money/dist/` |
| Does not close leases / CF / Host maps | Origin Host check, then public URL |

## Gold keys (do not invent)

```text
static/permissionless.money/dist/pages/mint.html
static/permissionless.money/dist/lib/…
```

Not `permissionless/pages/`. Not usb2 as the live origin.

## Alias

From oline elevated **`role=minio`** for the **live Phase G sites** lease (example: dseq **28305759**, NodePort `provider.akash-palmito.org:32755`). On groot2 that is often **`palm`**; on the laptop **`palmito-live`** if it points at the same endpoint. **Not** usb2. **Not** the dead `oline` europlots :32332 alias.

## Publish

```bash
# From a host that can see both aliases (usually groot2):
mc mirror --overwrite \
  usb2/static/permissionless.money/dist/ \
  palm/static/permissionless.money/dist/
# or from a fresh local build:
# mc mirror --overwrite websites/permissionless.money/dist/ palm/static/permissionless.money/dist/

mc anonymous set download palm/static
```

## Verify

1. `mc stat …/pages/mint.html` — size/ETag match source; body contains `LIVE_RAILS = ['zec', 'native']`.
2. Public: `curl -sSI https://permissionless.money/mint` — fresh `Last-Modified`, then body has parked lede + `LIVE_RAILS`.
3. Done when Pay UI shows **TERP/THIOL + Zcash only** (ETH/Base/BTC not offered).

## Landed (2026-08-26)

**Correct source:** `ssh groot-wan` (`groot.local`)  
`/Users/returniflost/abstract/terp-core/websites/{permissionless.money,terp.network}/dist/`

**Correct dest:** Phase G MinIO **`:32755`** (dseq **28305759**). On groot-wan the alias `palmito-live` was **dead `:30438`** — use `mc alias set phaseg-sites http://provider.akash-palmito.org:32755 …` (keys from `~/.oline/s3-phase-g-sites-restore-28305759.env`). **Not usb2.** **Not** HTML-only puts.

Full `mc mirror --overwrite` of both `dist/` trees → `phaseg-sites/static/…`. Public `/mint` + `/lib/{onboard-client,auth,mint-svg,dregg-svg-checkout,console-ingest}.js` share **Last-Modified: Wed, 26 Aug 2026 07:30:48 GMT**; onboard has `export async function attachPasskeyWithGrant`; mint has `LIVE_RAILS = ['zec','native']` + fractal Play/Random. No lease close.
