# ZEC watch without LWD — Zakura custom P2P + spendability PIR (research brief)

**Audience:** DREGG / Zakura teammates replacing `lightwalletd` as the house CompactTxStreamer for checkout deposit detection.  
**Inputs:** [Twitter thread](https://x.com/distractedm1nd/status/2091898291696914534), [distractedm1nd/p2p-spendability-pir](https://github.com/distractedm1nd/p2p-spendability-pir), our live LWD stall (~2.4 blk/s rebuild after v0.5.3).  
**Not FE.**

**Lab compact streamer (gRPC, now):** `ZEC-ZTREAMER.md` — `crates/ztreamer` `[[bin]] ztreamer`, vendor `Ztreamer`, lab `:19067`. CustomService compactstreamer over P2P is **remaining=true** (not the lab-e2e gate). PIR still does not discover deposits.

---

## What the thread actually ships

Ryan (@distractedm1nd) on Zakura **custom P2P services in main**:

1. **Spendability PIR over P2P** — private, sub-second checks that known Orchard/Ironwood notes are still spendable **before** the wallet finishes sync ([p2p-spendability-pir](https://github.com/distractedm1nd/p2p-spendability-pir)).
2. **Sphinx / Nym onion tx submission** ([zonion](https://github.com/distractedm1nd/zonion)) — private broadcast experiment; orthogonal to deposit watch.
3. **Header-only light clients** (zkfart) — path toward “wallet *is* a network node,” still early.

Zakura maintainer reaction: supportive; this is the intended extension point (`CustomService` + request/response streams).

---

## Critical product mismatch (read this first)

| Need for DREGG `watch_zec_*` | Spendability PIR provides? |
|---|---|
| **Discover** a new payment to our ZIP-321 UA / diversifier + memo | **No** — docs explicitly: PIR does **not** discover notes, does **not** create history/balances |
| Confirm note exists / amount / memo for entitle | **No** — operates only on notes **already** in the wallet DB from prior scanning |
| Instant spendability of idle wallet notes | **Yes** — nullifier + witness PIR |
| Avoid leaking “which notes we care about” to a server | **Yes** — SimplePIR / YPIR over HTTP or Zakura P2P |

So: **PIR alone cannot replace LWD for DREGG deposit monitoring.** It replaces *“wait for full sync before spend”*, not *“see a new inbound ZIP-321 payment.”*

Today DREGG does:

```text
zcash-devtool wallet -s $ZCASH_DEVTOOL_SERVER sync
→ list-tx / list-unspent / list-received
```

against CompactTxStreamer (`:9067`). That **is** note discovery. LWD’s multi-day compact rebuild is the friction we feel.

---

## What we *can* leverage from the repo

### A. Zakura custom P2P service pattern (highest leverage for DREGG)

From `p2p_parity.rs` / `P2pPirService`:

- Service ID: `zakura.spendability-pir.v1`
- Capability bit + stream kind/version
- Two-node loopback: **provider** `spawn_zakura_endpoint_with_services(..., CustomService { provides, seeks })` + **client** `P2pPirNode::spawn(identity, bootstrap_peers=[node_id@ip:port])`
- Request/response framing (`HealthReq`, query/metadata messages) with parity vs HTTP

**DREGG-shaped reuse:** define a **new** custom service (name TBD, e.g. `zakura.compact-watch.v1` or `zakura.ufvk-receive.v1`) on the **archive house Zakura** that answers:

- compact-block ranges, **or**
- “notes received for UFVK/diversifier since height H” (viewing-key scan server-side — trust/privacy tradeoff), **or**
- memo/commitment presence proofs for our Pedersen `memo_hex`

Client side (lab + live fed) becomes a thin Zakura P2P peer that **seeks** that service — no CompactTxStreamer, no multi-GiB LWD cache on the watcher host.

This matches the thread’s endgame: *“light wallets could just be a node in the network.”* DREGG’s watcher *is* that light node.

### B. Existing two-node e2e template (lab)

`crates/server/tests/p2p_parity.rs` (`http_and_zakura_clients_match`, ignored for full YPIR):

1. Spin **server** Zakura endpoint with `CustomService` providing PIR.
2. Spin **client** `P2pPirNode` dialing `node_id@127.0.0.1:port`.
3. Assert HTTP client ≡ P2P client for health/metadata/query.

**Our ict-rs / `svg_mint_local_prod` ZEC path should morph to:**

| Role | Process | Job |
|---|---|---|
| Node A (provider) | `zakurad` archive/regtest + custom receive/compact service (and optionally PIR) | Holds chain + serves P2P queries |
| Node B (watcher) | lightweight Zakura peer **or** DREGG-side P2P client | Issues ZIP-321 dest, waits for receive via P2P, entitles |
| Proof | Pay on A’s network → B sees note/memo **without** LWD | Replace `ZCASH_DEVTOOL_SERVER=…:9067` |

Today lab already has cookie-harness LWD on `:19067`; the replacement gate is: **same entitle proof with LWD absent.**

### C. PIR itself (secondary for mint desk)

Useful **after** we detect a deposit (or for a future “spend/refund” path):

- Nullifier PIR: “has this note already been spent?” (~65 ms, ~56 MB DB)
- Witness PIR: fetch Merkle path so a note is spendable before ShardTree catch-up

**Still needs an ingest source of compact/nullifier/commitment data.** Current server CLI still takes `--lwd-url` for building PIR DBs. So PIR **does not** remove LWD from the *provider* until ingest can read Zakura state/RPC/treestate directly (Ironwood activation height in their docs: **mainnet 3 428 143** — same wall we hit with LWD v0.4.18).

### D. What not to over-claim

| Feature | For DREGG mint watch? |
|---|---|
| Nullifier/Witness PIR | Not deposit discovery |
| zonion onion submit | Tx broadcast privacy only |
| Header-only light client | Future; not a drop-in for UFVK receive watch yet |
| Keep running house LWD to tip | Still valid backup; just slow (~2.4 blk/s → ~2 weeks) |

---

## Recommended architecture (house)

```text
Akash pruned Zakura          — P2P porch only (unchanged)
        │
        ▼
groot2 archive Zakura        — treestate image, tip-synced ✅
        │  CustomService: receive/compact watch (NEW)
        │  Optional: spendability-pir CustomService (later)
        ▼
DREGG svg-mint-fed           — Zakura P2P client (seek service)
        OR thin sidecar that feeds the same watch_zec_* surface
```

**ZIP-321 issuance** can stay `zcash-devtool derive-address` offline (UFVK dir) — does not need LWD.  
**Watch** moves from CompactTxStreamer sync → P2P receive/compact query.

Privacy: prefer PIR-shaped or commitment-only queries over shipping raw UFVK to a random peer; for **our** house, the provider *is* us (127.0.0.1 / LAN), so a trusted UFVK scan service on archive Zakura is acceptable for v1, with PIR as the public-facing upgrade.

---

## Workflow / e2e changes (concrete)

| Today | Next |
|---|---|
| `ZCASH_DEVTOOL_SERVER=127.0.0.1:9067` | `ZAKURA_P2P_WATCH=node_id@127.0.0.1:port` (+ service id) |
| Lab: cookie-harness + LWD | Lab: **two Zakura processes**, provider registers custom service; watcher dials it |
| `svg_mint_local_prod` `zec_skipped` if no LWD | New feature flag: prove ZEC entitle **with LWD stopped** |
| Preflight: port 9067 / not 8232 | Preflight: P2P dial + `HealthReq` (or compact-watch equivalent) |
| Workflow `porch-multichain-selfhosted` ZEC house = LWD tip | Add card: **ZEC-P2P-watch** — inventory Zakura `CustomService`, spike receive protocol, two-node ict-rs gate |

Suggested new workflow name: **`dregg-zec-p2p-watch`**  
Phases: Survey (this brief + Zakura `CustomService` APIs) → Spike protocol → Lab two-node e2e → Wire fed → Skeptic (LWD process must be down; entitle still happens).

---

## Suggested spike order (honest)

1. **Spike custom compact/receive service on Zakura** (clone `P2pPirService` pattern; stub methods; prove two-node `HealthReq`-style round-trip in ict-rs).  
2. **Minimal receive path for lab:** provider scans with UFVK (or serves compact blocks from height H); watcher polls until memo/`list-received` equivalent matches pay-targets.  
3. **Swap `watch_zec_devtool`** behind a trait: `CompactStreamer | ZakuraP2pWatch`.  
4. **Optional:** colocated spendability-pir for refund/spend later; ingest from Zakura once LWD-free ingest exists.  
5. Keep LWD running in background as research/backup until (2)–(3) green — do not block FE native mint on it.

---

## Bottom line (PIR)

- **Thread/repo gold:** Zakura **custom P2P request/response services** + **two-node dial** pattern.  
- **Not a drop-in LWD killer for DREGG:** spendability PIR does not detect new ZIP-321 deposits.  
- **Path that actually kills LWD for mint watch:** a **CompactTxStreamer-shaped** surface on Zakura (see **ztreamer** below), DREGG/`zcash-devtool` as client, proven with **two local nodes** and stock LWD absent.

---

## ztreamer — the actual LWD replacement for DREGG

**ztreamer** (Ryan / distractedm1nd): optimized CompactBlock indexer for light wallets, **Zakura under the hood**. Stated goal: **CompactTxStreamer over Zakura custom P2P** — last RPC to kill for light clients. Indexing speedups are “nobrainers along the way.”

### Why this matches DREGG (unlike spendability PIR)

| | Stock LWD (our pain) | ztreamer | Spendability PIR |
|---|---|---|---|
| Supplies **CompactBlocks** / CompactTxStreamer | Yes (slow rebuild) | **Yes — purpose** | No |
| Speaks protocol `zcash-devtool -s` already uses | Yes (`:9067`) | **Target yes** (gRPC now → P2P custom service TODO) | Different API |
| Discovers new receives for UFVK watch | Via compact sync | Via compact sync | **No** |
| Needs multi-day Sapling→tip cache on watcher host | Yes (~2.4 blk/s) | Faster index; ideally **provider holds LMDB**, watcher pulls ranges | N/A |

So for `watch_zec_devtool` / ZIP-321 entitle: **ztreamer is the right dependency**, PIR is optional garnish.

### Features to lean on

1. **Embed `zakurad` + parse raw bytes from DB handle** — skips field-element de/re-compress that burns ~91% CPU in Zaino. Uses our existing archive/treestate house node instead of a second full decompress pipeline.
2. **Pipelined fetch → build → LMDB write** — higher throughput than LWD’s single-threaded cache fill.
3. **Seal/persist policy:** ranges of 1000, seal after 100, LMDB after 10; still handles deep reorgs. Contrast Zaino: only durable after **1000 confs**, recent tip in slow passthrough. **Tip-proximal deposits (mint checkout)** care about that recent window — ztreamer’s design is friendlier to “pay now, see soon.”
4. **Concurrent initial indexing** — attacks the cold-start we just reset with LWD v0.5.3.
5. **TODO that unlocks our topology:** `create compactstreamer custom service` — same `CustomService` / two-node pattern as `zakura.spendability-pir.v1`, but for `GetBlockRange` / CompactTxStreamer semantics over P2P.

### How we’d wire DREGG

```text
groot2 archive Zakura (treestate)     ← already tip-synced
        │
        ▼
ztreamer (embeds/reads Zakura DB)     ← CompactBlock LMDB index
        │  today: gRPC CompactTxStreamer (drop-in for :9067)
        │  next:  Zakura custom P2P CompactStreamer service
        ▼
zcash-devtool / svg-mint-fed watch    ← same CLI surface; change SERVER endpoint only
```

**Phase 1 (compat):** Point `ZCASH_DEVTOOL_SERVER` at ztreamer’s gRPC (still localhost). Prove entitle faster than LWD; keep API identical.  
**Phase 2 (P2P):** Custom service live; lab two-node e2e with **no** LWD and **no** gRPC to streamer — watcher dials `node_id@ip:port`.  
**Phase 3 (optional):** Colocate spendability-pir on the same Zakura for spend/refund; still not required for mint detect.

### E2E / workflow impact

| Gate | Change |
|---|---|
| Lab `svg_mint_local_prod` ZEC | Spawn Zakura + **ztreamer** (not stock LWD); pay → entitle |
| Two-node proof | Provider: Zakura+ztreamer(+custom service); Watcher: P2P client or gRPC to provider only |
| Preflight | Accept ztreamer gRPC or P2P health; stop requiring `electriccoinco/lightwalletd` |
| Workflow | **`dregg-zec-ztreamer`** (`mode=lab-e2e`): clone https://github.com/distractedm1nd/ztreamer, spawn CompactTxStreamer, `scripts/lab-e2e-zec.sh` + `ultralight-local-e2e.sh`. SSOT: `ZEC-ZTREAMER.md`. Skeptic: LWD down. |

### Benchmark question (their TODO)

> would this make our sync faster, or even replace lightwalletd?

For **our** desk: yes to both *if* CompactTxStreamer parity holds.

- **Faster:** DB-handle parse + pipeline + early LMDB seal vs LWD’s Go cache rebuild (~2.4 blk/s → multi-day).  
- **Replace:** Phase 1 as drop-in `:9067` peer; Phase 2 as P2P so light clients (and DREGG) need zero RPC.  
- **Caveat:** Still need **archive/treestate Zakura** for historical compact data (same as LWD). Porch pruned node alone is not enough. Benchmarks should compare wall-clock **Sapling→tip index** and **tip lag under load** vs LWD v0.5.3 / Zaino / Zinder on the same Zakura datadir.

### Stack ranking for us

1. **ztreamer** — primary path off stock LWD for DREGG watch.  
2. **Zakura CustomService (compactstreamer)** — how we kill the last RPC.  
3. **Spendability PIR** — nice for spend/refund UX later; not mint detect.  
4. **Stock LWD** — keep as backup until ztreamer e2e green; don’t wait weeks of LWD rebuild to ship native mint.

“Do not try at home with real funds” still applies to experimental PIR/spend paths; house ztreamer + our archive Zakura is the operationally sane mint path.
