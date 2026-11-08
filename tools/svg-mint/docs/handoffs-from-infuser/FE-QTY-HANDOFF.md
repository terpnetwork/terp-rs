# FE qty — pay-targets `?qty=N` (1…20)

**To:** static HTML (already shipping Pieces). **From:** DREGG runtime.  
**Not dao-dao-ui.** Page still does **not** POST `/v1/deposit`.

## Contract

`GET {DREGG}/v1/pay-targets?recipient={terp1}&qty=N`

| Field | Rule |
|---|---|
| `qty` | Echo clamp 1…20 (missing → 1) |
| `zec_zat` | unit zat × N (`150000` × N) |
| ZIP-321 `amount=` | that zat as ZEC decimal; **`memo=`** ZIP-321 base64url of **ASCII hex** Pedersen (`6431…`, never raw bytes, never `terp1`) |
| `native_amount` | `100000000` × N |
| `eth_wei` / `base_wei` / `btc_sats` | unit × N (parked rails) |
| Watch | `floor(paid / unit)` — 3× zat entitles 3 units (BOGO still applies to NFT count) |

Do not invent a UA. Hide ZEC if `zec_uri` is null. Native qty works without ZEC.

Until this landed, the page rewrote only `amount=` on a 1-unit URI. Runtime now issues the scaled URI so QR and watch agree.

## Identify

ZEC watch parses paid zat from `list-received` / ZIP-321 `amount=` (never a dummy 10000 zat). Memo still binds the checkout (`commit_from_memo_param`). Same UA + unique memo per cart is OK.
