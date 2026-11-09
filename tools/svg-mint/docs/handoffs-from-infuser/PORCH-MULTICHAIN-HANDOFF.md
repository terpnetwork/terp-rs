# Handoff — Multi-chain self-hosted porch + house (infra specialist)

**To:** Zcash / node / lab-infra teammate (development implementer — workflows, house daemons, e2e gates)  
**From:** Container / Akash deploy lane (porch SDL, lease lifecycle, groot2 compose wiring for rented face)  
**Audience job:** Constructively **critique** the plan until it is sound, then **implement the workflow + house query plane** the way you know will stay honest.  
**Not FE. Not dao-dao-ui.** Same product surface as `ZEC-INFRA-HANDOFF.md` / `PREFLIGHT-MULTICHAIN.md`.

**Approved topology (locked with operator):**

| Plane | Where | Job |
|---|---|---|
| **Porch** | **One** Akash dseq, multi-container | BTC + ETH + Base **P2P only** (optional fold of ZEC pruned sentry later). **No** public balance/index RPC. |
| **House** | groot2 `/lab` first (spill cold data to `/permafrost` only if needed) | Query nodes + indexers + `svg-mint-fed` on **`127.0.0.1`**. This is where privacy is real. |
| **Lab prove** | groot-wan | `svg_mint_local_prod` / `svg_mint_base_local` / cookie harness — aarch64; do not copy ELFs to groot2 |

```text
Public P2P only (no address/balance queries)
        │
        ▼
┌───────────────────────────────────────┐
│  ONE Akash lease — PORCH              │
│  btc-p2p | eth-p2p | base-p2p         │
│  (+ optional zakura pruned sentry)    │
└──────────────────▲────────────────────┘
                   │ preferred peers / tip
┌──────────────────┴────────────────────┐
│  groot2 HOUSE                         │
│  bitcoind(+wallet)  → BTC_RPC :8332   │
│  reth|geth          → ETH_RPC :8545   │
│  base EL            → BASE_RPC        │
│  zakura+LWD         → :9067           │
│  svg-mint-fed       → watches localhost only
└───────────────────────────────────────┘
```

**Marketing claim we are allowed to make only after skeptic green:**  
“Deposit and balance watches for checkout hit operator-run nodes on our hardware; Akash only carries P2P porch gossip.”  
Not “trustless.” Not “no infra.”

---

## Role split (do not step on each other)

| Lane | Owns | Does not own |
|---|---|---|
| **You (infra / workflow)** | House compose on groot2; RPC allowlist + preflight/e2e gates; **author + land** `.grok/workflows/porch-multichain-selfhosted.rhai`; watcher-driven entitle proofs; critique of disk/API/Base-op-node reality | Akash bid/select theater without a reviewable SDL; FE buttons |
| **Container / Akash lane** | `deploy-porch.yaml` (one dseq, multi-svc); lease deploy/close/update; P2P endpoint env (`~/.oline/dregg-porch-multichain-<dseq>.env`); porch image pins; no public unauth JSON-RPC on the lease | Inventing house indexer design; claiming ZEC/ETH ready before tip |

ZEC house (`/lab/zakura-mainnet` + LWD) is already on the local-first path — see `ZEC-INFRA-HANDOFF.md`. This pack **generalizes** that pattern to BTC/ETH/Base.

---

## What already exists (ground truth)

| Piece | Path / fact |
|---|---|
| Live DREGG | groot2 `:8421` → `https://dregg-svg.permissionless.money` |
| Fed watchers | `crates/dregg/svg-mint/src/bin/svg-mint-fed.rs` — `ETH_RPC`, `BASE_RPC`, `BTC_RPC` (`listsinceblock`), `ZCASH_DEVTOOL_SERVER` |
| Preflight (weak today) | `scripts/preflight-multichain.sh` — any URL with right chainId passes |
| Lab e2e | `svg_mint_local_prod`, `svg_mint_base_local` (ict-rs) — Docker anvil/bitcoind; often signed `/v1/deposit` bypasses watcher |
| ZEC porch | Akash dseq **28333244** europlots P2P `86.32.72.140:31417` — **no** public LWD |
| ZEC house | `/lab/zakura-mainnet` archive + LWD; LWD compact cache still catching up as of handoff |
| Workflow authoring | `~/.grok/bundled/skills/create-workflow/SKILL.md` → save `.grok/workflows/<name>.rhai` |
| Related packs | `dregg-multichain-preflight`, `dregg-base-rail`, `dregg-groot2-runtimes`, `svg-mint-settle` |
| Full plan | Session plan / operator-approved multichain porch plan (topology above) |

**Now shipped (slice 1 + contrast):** `porch-multichain-selfhosted` (policy observe), `porch-house-btc-prove` (ultra-light local e2e). Preflight/spawn **do** reject Alchemy/Infura when `SVG_MINT_REQUIRE_SELF_HOSTED_RPC=1`. Remaining: house BTC on groot2, pending checkout TTL, porch digest bid, ETH/Base house, ZEC LWD tip.

---

## Ask of you — critique first (elevate the plan)

Please push back in writing (PR comment, reply packet, or short MD under `svg-mint/`) on these **specific** risks before locking craft:

1. **BTC wallet vs ephemeral HD `bc1`**  
   Fed uses `listsinceblock` against bitcoind wallet RPC. How do we watch **many short-lived** pay-target addresses without a bloated wallet or missing txs? Is Core wallet enough, or do we need Electrs/Fulcrum + a fed change? Critique the plan’s “bitcoind+wallet first” assumption.

2. **ETH disk on one Akash dseq + groot2 `/lab`**  
   `/lab` is ~469 G with ~259 G Zakura archive. Porch ETH PV ~800 Gi–1.2 Ti may fail bids. Is “one lease” still the right packaging, or should ETH porch be a **second dseq** under the same workflow/docs while BTC+Base share the first?

3. **Base tip stack**  
   Is a single Base EL container enough for `eth_getLogs` / HD watch at tip, or must the porch/house include **op-node** (two containers)? What image pins do you trust?

4. **House peer model**  
   Same as Zakura (`addnode` / preferred peers to porch IP:port). Any chain where porch-P2P-only is a footgun (e.g. house cannot sync without public seeds)?

5. **E2E honesty**  
   Which entitle paths must be **watcher-only** (no signed `POST /v1/deposit`) for the privacy claim? Plan asks ETH+BTC siblings of `svg_mint_base_local` — confirm or rewrite.

6. **Allowlist**  
   Propose the real allowlist (loopback, `192.168.10.0/24`, Headscale/tailnet prefixes). What else must pass for lab Docker DNS (`http://anvil:8545`)?

7. **Rollout order**  
   Locked: policy + lab e2e → house BTC (prune/watch-only) → pending TTL + import/drop → pin porch digests and bid P2P porch → ETH/Base house → ZEC when LWD ≈ tip. Porch bid is **not** the query plane.

Paste critiques as **concrete deltas** (what to change in SDL / compose / fed / workflow), not vibes.

---

## Ask of you — implement (after critique merges)

### 1. Land the workflow (you author)

**Name:** `porch-multichain-selfhosted`  
**Path:** `.grok/workflows/porch-multichain-selfhosted.rhai`

Phases (fail-closed skeptic at end):

1. **Inventory** — porch/house matrix from this file + `ZEC-INFRA-HANDOFF.md` + live env  
2. **Curate-porch** — expect SDL + `~/.oline/dregg-porch-multichain-*.env` (container lane supplies); no public JSON-RPC  
3. **Wire-house** — groot2 compose; peers → porch; fed env localhost-only  
4. **E2E** — groot-wan lab examples + groot2 `preflight-multichain.sh`  
5. **Skeptic** — quoted evidence per selected rail; reject public RPC hosts; ZEC is `:9067` not `:8232`

Args: `mode=observe|plan|craft`, `rails`, `house_host=groot2-wan`, `lab_host=groot-wan`, `allow_zec_skip`.

Follow create-workflow: pure-literal `meta`, `validate_only` smoke, then save. Document in `.grok/workflows/README.md` next to Pack DREGG multichain preflight.

Compose with existing packs by **pointing at their SSOTs** (workflows cannot nest). Do **not** revive public Akash LWD as the target (`zcash-zakura-lightwalletd` is historical).

### 2. Policy gates (you)

- Shared allowlist used by `preflight-multichain.sh` when `SVG_MINT_REQUIRE_SELF_HOSTED_RPC=1`  
- Spawn checklist / `spawn-groot2-runtimes.sh`: require rail RPCs when flag on  
- Fix `crates/zakura/deploy/akash/zcash-zakura-lwd/dregg-env.example` stale public LWD → `127.0.0.1:9067`  
- Lab e2e: assert RPC URLs are loopback/Docker-local; add watcher-driven ETH/BTC entitle paths as you specify in critique

### 3. House daemons (you design, we can help ship containers)

```text
/lab/dregg-house/          # compose + configs
/lab/dregg-house/{btc,eth,base}/
```

Prefer `/lab`; stage large syncs on `/permafrost` only for headroom (same lesson as Zakura archive untar).

| Env | Target |
|---|---|
| `BTC_RPC` | `http://127.0.0.1:8332` (wallet-capable) |
| `ETH_RPC` | `http://127.0.0.1:8545` |
| `BASE_RPC` | `http://127.0.0.1:<base>` chainId `0x2105` |
| `ZCASH_DEVTOOL_SERVER` | `127.0.0.1:9067` |

### 4. Done when (proof)

| Gate | Evidence |
|---|---|
| Preflight | `SVG_MINT_REQUIRE_SELF_HOSTED_RPC=1` fails on Infura/Alchemy URL; passes on localhost house |
| Watcher | At least one entitle per selected rail **without** relying on public RPC in fed environ |
| Workflow | `/workflow porch-multichain-selfhosted {"mode":"observe"}` then `craft` skeptic green with quotes |
| Claim | Only then update product copy to “self-hosted query plane” |

Until skeptic green, FE/docs must not claim 100% self-hosted watches.

---

## What the container / Akash lane will deliver for you

1. **`crates/dregg/svg-mint/deploy/akash/porch-multichain/deploy-porch.yaml`** — one dseq, `btc` + `eth` + `base` (+ optional `zakura`), **P2P public, RPC not world-open**  
2. Lease env: `~/.oline/dregg-porch-multichain-<dseq>.env` with `*_P2P` / `*_P2P_IP`  
3. Help wire house `addnode`/preferred peers to those endpoints  
4. Keep ZEC sentry healthy (or fold into multi-chain dseq when you say go)

If ETH disk makes one dseq unbiddable, we will say so with bid evidence and propose ETH-on-second-dseq under the **same** workflow — not silent fallback to public RPC.

---

## Do not

- Put CompactTxStreamer or anonymous ETH/BTC JSON-RPC on the Akash porch  
- Point live DREGG at Infura/Alchemy/publicnode to “unblock”  
- Set `DREGG_ZEC_UA` / fake pay-targets to green FE  
- Treat container-started as tip-ready (ZEC LWD lesson)  
- Copy groot-wan aarch64 ELFs to groot2  
- Nest workflows or embed mnemonics/dseqs as secrets in `.rhai`  
- Block ETH/Base/BTC mint rails that already work while ZEC LWD catches up — stage flags per rail

---

## Related files

| Doc | Why |
|---|---|
| `ZEC-INFRA-HANDOFF.md` | ZEC porch/house precedent |
| `PREFLIGHT-MULTICHAIN.md` | Current (weak) soundness check |
| `SPAWN-CHECKLIST.md` / `SPAWN-GROOT2-RUNTIMES.md` | Live fed spawn |
| `crates/zakura/deploy/akash/zcash-zakura-lwd/PRODUCT-DESIGN-HANDOFF-LOCAL-FIRST.md` | Porch vs house narrative |
| `LAB-GROOT2.md` | Zakura house compose |
| `.grok/workflows/README.md` | Pack index |
| `~/.grok/bundled/skills/create-workflow/SKILL.md` | How to land the Rhai workflow |

---

## Reply packet (please fill)

```text
CRITIQUE:
- BTC watch model: …
- ETH packaging (1 vs 2 dseq): …
- Base op-node: yes/no + images: …
- E2E watcher gates: …
- Allowlist CIDRs/hosts: …
- Reorder slices: …

IMPLEMENT OWN:
- [x] porch-multichain-selfhosted.rhai (validate_only + README pack) — slice 1
- [x] SVG_MINT_REQUIRE_SELF_HOSTED_RPC preflight/spawn + fed refuse + rust tests
- [x] deploy/house compose (BTC first; copy to /lab/dregg-house)
- [x] porch-house-btc-prove + `ultralight-local-e2e.sh` (contrast + groot-wan cargo; not a source quote)
- [x] pending checkout TTL + bitcoind import (`importaddress`) / drop (`setlabel dregg-expired`; Core cannot delete)
- [ ] watcher e2e deltas (ETH/BTC siblings of svg_mint_base_local)
- [x] draft deploy-porch.yaml for container lane review (not bid)

NEED FROM CONTAINER LANE:
- [ ] deploy-porch.yaml draft for review before bid
- [ ] …
```

Hand this packet back; we iterate once on critique, then you craft the workflow while we land the Akash porch SDL.

---

## Slice status (ops)

| Slice | State |
|---|---|
| **1 — Policy** | Landed + observe-green (`rpc_allowlist`, preflight/spawn/fed, porch SDL draft, house compose draft, `porch-multichain-selfhosted`) |
| **2a — Lab BTC e2e** | `/workflow porch-house-btc-prove {"mode":"lab-e2e"}` → `scripts/ultralight-local-e2e.sh` (contrast JSON then cargo). Must cargo-run, not quote `.rs`. |
| **2b–2d — BTC/ETH/Base porch+house** | **Parked.** 28337960 closed. Product payments are **ZEC only**. |
| **ZEC** | Porch dseq **28333244** P2P; house Zakura+LWD on groot2 `:9067`. `ZEC-INFRA-HANDOFF.md` |
