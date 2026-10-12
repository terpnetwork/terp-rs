# Trailmark — permissionless mint flows (handoff)

For the teammate writing Trailblaze trails. This is the **product contract** between the mint page, DREGG, onboard, house Zakura/ztreamer, and the admin desk. Not a session log.

**Website SoT:** groot-wan `websites/permissionless.money/`  
**Runtime SoT:** groot2 `svg-mint-fed` `:8421` + `onboard-runtime` `:19090` (loopback; nginx TLS in front)  
**Lab e2e:** groot-wan `ict-rs` `svg_mint_local_prod` + `dregg/svg-mint/scripts/lab-e2e-zec.sh` (`pweb_walk`)

Local sandbox (not prod): http://127.0.0.1:8001/mint · http://127.0.0.1:8001/ops

---

## Actors (do not mix keys)

| Actor | What it is | Must not be |
|---|---|---|
| Holder | Keplr `terp1…` or passkey-bound device | Admin |
| Deposit dest | ZIP-321 UA / native pay-target from DREGG HD | Admin zid |
| HashMerchant | `fee_granter` for **one** msg: `/terp.smartaccount.v1beta1.MsgAddAuthenticator` + 1 THIOL runway | Minter, deposit UA |
| DREGG runtime | Entitles and mints after payment | Feegrant customer |
| Admin | zafu/zid **or** `DREGG_ADMIN_TOKEN` | Deposit wallet, HM runtime key |

First zid login on an empty house writes `~/.terp/dregg-admin-zid.pk`. Later logins must match. Token backup: `~/.terp/dregg-admin.token` (600).

---

## Critical user flow (mint desk)

```
You (wallet gate)
  → Desk (prices + checkout_ready)
  → Send (QR / ZIP-321 or native Keplr)
  → Seen
       Mempool   = unconfirmed ZEC (confirmations=0)
       Deposited = in a block
  → NFT mint (DREGG entitle / settle)
  → Device gas (passkey only): POST /onboard/v1/passkey-grant {address, c}
       c = pay-targets memo_hex (Pedersen)
       unpaid → 402
  → Yours + success/error modal
```

**Wallet gate:** Watch / ZIP-321 does not load until `holderReady()` (Keplr terp1 or passkey). Toast and stay on unlock otherwise.

**CORS:** Production mint uses same-origin `/chain-rpc` and `/chain-api`. Do not send the page to Infura/Alchemy. Admin `Authorization` is allowed on DREGG CORS.

**Rails live:** ZEC + native THIOL. ETH/BTC/Penumbra/UM parked.

---

## Runtime ↔ admin communication

| Call | Auth | Purpose |
|---|---|---|
| `GET /health` | none | `checkout_ready`, `pay`, `zec_tip`, minted |
| `GET /v1/prices` | none | Catalog; Pay still requires checkout |
| `GET /v1/pay-targets?recipient=` | none | Dest + `memo_hex` (`c`) |
| `GET /v1/status?recipient=` | none | `phase`: null / `Mempool` / `Deposited` / `Entitled` / `Settled` |
| `GET /v1/query/paid?recipient=&c=` | none (house) | Onboard fail-closed paid check |
| `GET /v1/admin/challenge` | none | zid nonce (TTL ~2 min) |
| `POST /v1/admin/session` | zid sig | `{challenge, publicKey, signature}` → `{token, pubkey}` |
| `GET /v1/admin/stats` | Bearer token **or** zid session | pending, mempool, minted, recipients — **not** on the public mint story |

Public FE must not render admin stats. `/ops` is unlinked.

---

## What is already true (use as Trailmark oracles)

1. `/health` `pay`/`checkout_ready` can be true while `minted=0` — that only means the desk is open, not that a payment landed.
2. Unpaid passkey-grant **402** without `c` (live onboard).
3. Admin stats **401** without token; **200** with Bearer.
4. Admin challenge JSON `ok` + `challenge` (live).
5. Compact indexer is **ztreamer** on `127.0.0.1:9067`, not ECC LWD, not `:8232`.
6. House Zakura is **one** container `zakura-lab` 1.4.0, loopback RPC.
7. Lab e2e `pweb_walk` asserts: unpaid blocked, paid grant, admin stats, `paid_c`, zcli wallet, ztreamer vendor.

---

## Soundness gaps worth trails (not all green)

| Gap | Why it bites testers | What to assert |
|---|---|---|
| Payment “did nothing” | Watch used **minconf 1**; ZIP-321 memo often never bound (`pending>0`, `recipients=0`) | After send: `phase=Mempool` then `Deposited`; if still null, capture txid + `memo_hex` |
| `zec_tip` public RPC | `public_unreachable` used to close Pay; now may be empty `reasons` while public height is null | Don’t treat `pay:true` as “ZEC tip honest” |
| Grant vs dest | `c` is the **pay-target** commitment for that `terp1`. Granting a different address 402s | Trail: same address on pay-targets, status, grant |
| Keplr vs passkey | Keplr **skips** feegrant/SA | Walk “Device gas” skipped with reason, not error |
| Admin origin | `/ops` fetches DREGG origin with Bearer; CF/nginx must pass `Authorization` | Trail: 401 then zid session then stats |
| Compact vs zidecar | zcli `view address` needs `ZCLI_SKIP_SYNC=1`; sync is zidecar, indexer is CompactTxStreamer | Don’t require zcli sync against `:9067` |
| Prod vs lab | `:8001` is this laptop; prod is S3 from WAN `dist/` | Trails against lab URL until publish |

---

## Suggested Trailmark scenarios (delegate)

1. **Unlock gate** — no QR until Keplr or passkey.  
2. **Native Pay** — Keplr mint; walk Payment → Mint → Yours; no grant.  
3. **ZEC Pay** — ZIP-321 QR; mempool copy; then block; then mint.  
4. **Passkey after paid** — 402 without `c`; grant after `paid:true`.  
5. **Ops desk** — unlinked `/ops`; zid sign-in; stats shape; token is not the UA.  
6. **Failure modals** — mint ok / gas fail vs mint fail.  
7. **Lab e2e** — `lab-e2e-zec.sh` `pweb_walk` keys (Docker host), also `--target cworch` (`cworch_spawnSvgMint` / `cworch-preflight`).

Reuse these as primitives on other products: **wallet gate → pay dest + commitment → mempool/block → entitle → optional capability grant → admin read plane with a different key**.
