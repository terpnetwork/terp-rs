# Slice 2 — House BTC prove (after observe-green)

**Status:** **Parked.** Product payments are **ZEC only**. BTC lab e2e was proven; live house BTC and Akash BTC porch (dseq 28337960) are **closed**. Do not bid ETH/Base. ZEC porch+house: `ZEC-INFRA-HANDOFF.md`.

## What observe already certified

| Certified | Not certified |
|---|---|
| Alchemy/Infura denied | Live `ETH_RPC`/`BASE_RPC`/`BTC_RPC` on fed |
| Loopback allowed | House bitcoind running |
| Porch SDL P2P-only draft | Any Akash porch dseq |
| ZEC streamer port rule (`9067`) | Watchers seeing HD payments on operator nodes |

## Locked interim critique (container + ops defaults — infra may override)

**Weight target (operator, locked):** ultra-light watchers — “did this HD dest get paid?” only. Akash porch is **P2P gossip**, never a DREGG JSON-RPC oracle. House is `127.0.0.1`. Public SaaS is forbidden under `SVG_MINT_REQUIRE_SELF_HOSTED_RPC=1`.

Fill/override in the reply packet; until then workflows use these:

| Topic | Default for Slice 2 |
|---|---|
| **BTC HD / `listsinceblock`** | Lab: ict-rs bitcoind wallet pays + watches. Live house: **watch-only `importaddress`/`importmulti`** of each `btc_addr` when pay-targets issues it (fed hook or small importer). No Electrs required for first entitle. Drop/abandon imports when pending checkout TTL expires (craft item — see below). |
| **ETH packaging** | Prefer **one logical porch**; if 800 Gi bids fail, ETH on **second dseq**, same workflow docs. Do not fall back to Infura. House: snap + short lookback (fed already ~48 blocks for HD); avoid vault `eth_getLogs` from `earliest` on live (bloat). |
| **Base op-node** | Assume **EL + op-node** on house later; porch Base P2P-only EL is draft until infra confirms tip without sequencer. |
| **E2E gate** | Lab: `svg_mint_local_prod` with `bitcoin` feature must keep Docker-local `BTC_RPC`. Add/keep a path that reaches `Entitled` via **watcher** when possible; signed deposit alone is not the privacy proof. |
| **Allowlist** | Keep `rpc_allowlist` as shipped; extend `DREGG_RPC_ALLOWLIST` only for mesh peers. |
| **Who DREGG queries** | **House `127.0.0.1` only.** Never Akash porch RPC (porch has none public). Never Infura. |

### Checkout timeboxes (current fed behavior)

| Phase | Behavior |
|---|---|
| Watcher threads | Poll every **12s** forever (process lifetime) |
| What they scan | Open **`pending`** HD dests + entitled slots — not the whole chain |
| After **entitle**, no mint | `DREGG_MINT_TIMEOUT_SECS` default **20 minutes** → `SweepStaleEntitlements` → refund path |
| Before deposit | Fresh dests per `pay-targets`; unpaid dests expire after `DREGG_PENDING_TTL_SECS` (default **20 minutes**, `0` disables). Watchers drop them; bitcoind watch-only is relabeled `dregg-expired` (Core cannot delete imports) |

So UX is checkout-like for mint timeout after payment is seen; unpaid carts expire after `pending_ttl_secs`. FE must **select deposit again** (fresh pay-targets) if the user waits past TTL.

## Recommended sequence (locked)

1. Policy + lab e2e `live:true` — this workflow, `mode=lab-e2e`
2. House BTC ultra-light on `/lab/dregg-house` (prune + watch-only HD) — `mode=house-up`, human gate; no live fed flip until tip
3. Pending TTL + import/drop hook — **shipped** (`DREGG_PENDING_TTL_SECS`, `importaddress` on issue, expire + `setlabel dregg-expired`)
4. Pin porch digests → bid one Akash multi-container **P2P** porch (optional peer for house). DREGG still never queries it
5. ETH/Base house, same ultra-light pattern (short lookback; no vault `eth_getLogs` from `earliest`)
6. ZEC when LWD ≈ tip (LWD stays on the house, never Akash)

Akash “all three chain porches” is step 4, not step 2.

## Workflow

```text
/workflow porch-house-btc-prove {"mode":"lab-e2e"}
# Contrast: scripts/ultralight-contrast.sh (repo vs lock)
# Act: ssh groot-wan, PATH includes /usr/local/bin, scripts/ultralight-local-e2e.sh
# sound only if cargo JSON live:true AND btc_recorded:true (not a source quote)

/workflow porch-house-btc-prove {"mode":"plan"}
/workflow porch-house-btc-prove {"mode":"house-up"}   # human gate before docker on groot2
/workflow porch-house-btc-prove {"mode":"porch-review"} # pin digests, do not bid
```

**Runner:** `scripts/ultralight-local-e2e.sh` runs contrast then `lab-e2e-btc.sh` (Linux `ZAKURAD_BIN` + Linux fed ELF). Without those ELFs, cargo exits 2.

**SSOT compose:** `deploy/house/docker-compose.yml` → `/lab/dregg-house/`  
**SSOT porch draft:** `deploy/akash/porch-multichain/deploy-porch.yaml`  
**Policy:** `SVG_MINT_REQUIRE_SELF_HOSTED_RPC=1` stays on for groot2

## Done when (BTC rail)

1. `cargo run --example svg_mint_local_prod -p ict-rs-cw-orch --features docker,zakura,dregg-node,ethereum,bitcoin` on **groot-wan** quotes Docker-local `BTC_RPC` and a BTC entitle path (deposit or watcher).  
2. Allowlist still denies Alchemy.  
3. Optional: house `bitcoind` on groot2 `getblockchaininfo` via `127.0.0.1:8332` — tip sync may still be in progress; **do not** flip live fed `BTC_RPC` until near tip + wallet import strategy is live.  
4. Porch lease still optional; P2P peer is a later wire (`PORCH_BTC_P2P`).

### Lab e2e — landed (groot-wan)

Workflow `porch-house-btc-prove` `mode=lab-e2e` ran `scripts/ultralight-local-e2e.sh` (contrast then cargo). Cargo JSON (not a `.rs` quote):

| Field | Value |
|---|---|
| `live` | `true` |
| `mint_engine` | `cw-orch` |
| `queried_minted` | `1` (`266D4FE7…`) |
| BTC | `btc_recorded: true`, tx `8e996467…`, HD `bcrt1q7ypvlhpsjuv06y0g5zdyym95fu48cyj54t8qgc` |
| `BTC_RPC` | `http://127.0.0.1:18453` (lab Docker bitcoind — not Infura, not Akash) |
| ETH / Base | txs recorded |
| ZEC | `zec_skipped: true` — zakura lab not NU6.3 |
| Siblings | zakura, dregg-fed, anvil, base-anvil, bitcoind |
| Contrast | `ok:true`, `pending_ttl_shipped:false` (expected) |

First dashboard result was `sound:false` because the skeptic grepped git for `btc_tx` instead of `act.quoted`. Cargo had already succeeded. Workflow now writes `lab-e2e-quoted.json` scratch and the host owns the live/btc_recorded gate.

**This does not replace house-up on groot2.** Next product step remains `/lab/dregg-house` bitcoind + watch-only HD import, then tip before flipping live fed `BTC_RPC`.

## Do not

- Flip live fed RPCs while house is empty (spawn will demand URLs — use lab-only or leave watchers unset with flag understanding missing=ok in allowlist check for unset vars; spawn currently **requires** URLs — use staged env only after house tip).  
- Bid Akash porch until images are digest-pinned and BTC house path is green in lab.  
- Claim “100% self-hosted multichain” after BTC alone.
