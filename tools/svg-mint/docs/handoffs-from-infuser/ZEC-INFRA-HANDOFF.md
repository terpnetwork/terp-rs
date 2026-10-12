# Zcash infra specialist: wire LWD into DREGG (groot-wan → groot2-wan)

For the teammate already downloading the **Valar archive** and standing up **Zakura + lightwalletd** locally. This is how that stack attaches to the **live DREGG runtime on groot2-wan**. Not FE. Not dao-dao-ui.

**Done when:** `GET {DREGG}/v1/pay-targets?recipient=terp1…` returns a non-null `zec_uri` that starts `zcash:`, has `memo=`, and contains **no** `terp1`. Until then the static HTML teammate **hides** the ZEC button.

---

## Hosts (do not mix)

| Host | LAN | Job |
|---|---|---|
| **groot-wan** | `192.168.10.102` | Cargo/docker **lab**. You prepare archives, compose, cookie harness, `zcash-devtool` source. **aarch64** — do not copy ELFs to groot2. |
| **groot2-wan** | `192.168.10.103` | **Live DREGG house.** `svg-mint-fed` on `:8421` (`https://dregg-svg.permissionless.money`). Private **archive Zakura + LWD** at `/lab/zakura-mainnet`. **x86_64** — build linux/amd64 here. |
| **Akash sentry** | `provider.h6i-dedicated.eu-se-1.digitalfrontier.so:30761` (`77.76.13.210–212`) | **Porch only.** Pruned Zakura P2P, dseq **28404698**, Hub `zakuracore/zakura:1.2.0`. **No public lightwalletd.** (dead: `28333244` europlots; do not close Palmito/Headscale) |

`groot-wan` is where you **prepare**. `groot2-wan` is where you **spawn and leave it running**. Pointing public DREGG at a groot-wan LWD is wrong.

Local-first write-up (why public LWD was walked back): `crates/zakura/deploy/akash/zcash-zakura-lwd/PRODUCT-DESIGN-HANDOFF-LOCAL-FIRST.md`. Lab compose: `LAB-GROOT2.md`.

```text
Zcash P2P
    │
    ▼
Akash pruned sentry  :8233  (gossip only)
    │  preferred peer
    ▼
groot2  /lab/zakura-mainnet
    Zakura ARCHIVE  127.0.0.1:8232  cookie JSON-RPC   ← ops only
    lightwalletd    127.0.0.1:9067  CompactTxStreamer ← DREGG + wallets
    svg-mint-fed    127.0.0.1:8421  HTTP checkout
```

---

## Ports (wrong port = silent fail)

| Port | What | DREGG uses it? |
|---|---|---|
| **9067** | lightwalletd CompactTxStreamer gRPC | **Yes** — `ZCASH_DEVTOOL_SERVER` |
| 8232 | Zakura JSON-RPC (cookie) | **No** |
| 8233 | P2P | No |
| 8234 | Zakura P2P extra — **not** cookie | No |
| **18234** | Cookie HTTP for the LWD sidecar | LWD only, not DREGG |
| 8421 | `svg-mint-fed` | DREGG HTTP |

`zcash-devtool -s` is a **compact-tx-streamer** client. Never `zcash-rpc.terp.network`, never `:8232`, never `ZAKURA_RPC`.

---

## What DREGG actually does with ZEC

`svg-mint-fed` (groot2 `:8421`):

1. **Issue ZIP-321** in `GET /v1/pay-targets`:
   - Prefer `zcash-devtool wallet derive-address --diversifier-index N` (Linux ELF).
   - Else fall back to env `DREGG_ZEC_UA` (shared UA — only if the streamer is up; do **not** set this to fake a button).
   - `zec_uri` = `zcash:{ua}?amount=0.0015&memo=` Pedersen bytes. Memo is **not** `terp1`.
2. **Watch deposits** (`watch_zec_once`):
   - `watch_zec_devtool`: `zcash-devtool wallet -s $ZCASH_DEVTOOL_SERVER sync` then `list-tx` / `list-unspent` / `list-received`.
   - Optional `watch_zec_zcashd` if `ZCASHD_RPC` is a zcashd-compat wallet RPC (Zakura JSON-RPC has **no** wallet).

If `ZCASH_DEVTOOL_BIN` is missing or not an ELF, `zec_uri` is **null**. FE hides ZEC. That is correct.

ETH/Base/BTC on groot-wan e2e are already proven. **ZEC e2e is not live yet.** Do not claim native ZEC until pay-targets + one watched deposit.

---

## Track A — finish the house on groot2 (this is “get it working”)

You may download/extract on groot-wan, but the **canonical data root is groot2**. If the tarball already lives on groot-wan, rsync it; do not start a second 252 GiB extract on the wrong disk and then wonder which node DREGG talks to.

### A1. Archive bootstrap

```bash
ssh groot2-wan
cd /lab/zakura-mainnet
cat state/.snapshot-bootstrap.json   # wait until extract finished
tail -f logs/bootstrap-archive.log
```

- Snapshot: Valar **archive** (`https://zakura.valargroup.dev/mainnet/snapshots.json`, ~252 GiB). Pruned **cannot** feed stock LWD.
- Image: `zakura:vct-treestate-lwd` (historical treestate). Hub `1.2.0` **alone** loops at Sapling 419200 (`trees below the last checkpoint were never written`).
- Peer only the sentry: `initial_mainnet_peers = ["86.32.72.140:31417"]`. **No** `external_addr`.
- Env pointer: `~/.oline/zakura-lab-groot2.env` on groot2 (also `~/.oline/zcash-zakura-sentry-pruned-28333244.env`).

**Valar archive ≠ LWD-ready** without the treestate image. `--sync-from-height` on LWD only drops cache; it does not skip Sapling.

### A2. Spawn Zakura + LWD

```bash
ssh groot2-wan
cd /lab/zakura-mainnet
docker compose up -d
docker compose logs -f
```

Expect:

- Cookie fetch in LWD logs (`Fetched cookie`). Cookie is on **18234**, not 8234.
- Then compact ingest from Sapling (`Adding block to cache 419572…`) — hours.
- Host: RPC `127.0.0.1:8232`, LWD `127.0.0.1:9067`.

Keep `lwd-cache` across restarts.

**Ready for DREGG** only when LWD cache is **near tip**, not when the container started.

### A3. Linux/amd64 `zcash-devtool` on groot2

Build **on groot2** (`cargo build --release --all-features` from [zcash/zcash-devtool](https://github.com/zcash/zcash-devtool)). Confirm ELF:

```bash
file "$ZCASH_DEVTOOL_BIN"   # must be ELF 64-bit LSB, x86-64
head -c 4 "$ZCASH_DEVTOOL_BIN" | xxd   # 7f 45 4c 46
```

Do **not** scp groot-wan’s aarch64 binary.

Init a **UFVK view-only** wallet dir (mode 700). DREGG only needs to **see** ZIP-321 deposits, not spend.

### A4. Env for live `svg-mint-fed` (groot2)

Same process as `SPAWN-CHECKLIST.md` A3. Add:

```bash
# CompactTxStreamer on THIS host — not Akash, not 8232
export ZCASH_DEVTOOL_BIN=/path/on/groot2/zcash-devtool   # linux/amd64 ELF
export ZCASH_DEVTOOL_SERVER=127.0.0.1:9067
export ZCASH_DEVTOOL_NETWORK=main
export ZCASH_DEVTOOL_WALLET=/path/on/groot2/ufvk-wallet  # 0700

# Optional shared UA only after derive-address works
# export DREGG_ZEC_UA=u1...
```

Restart fed so it inherits env (`scripts/spawn-groot2-tunnel.sh` does **not** currently export these — set them in the unit/shell that launches the binary).

```bash
curl -sS http://127.0.0.1:8421/health
# HTTP 200 always. Pay only if checkout_ready === true.

curl -sS "http://127.0.0.1:8421/v1/pay-targets?recipient=terp1…" | jq '{zec_ua,zec_uri,zip321,memo_hex}'
```

**Pass**

- `zec_uri` starts `zcash:`
- has `amount=` and `memo=`
- no substring `terp1`
- `memo_hex` starts `6431`
- second pay-targets call gets a **fresh** dest (or a new diversifier), not a cached UA from a dead streamer

**Fail** — leave ZEC hidden. Do not set `DREGG_ZEC_UA` to paper over a down LWD.

Public `{DREGG}` is the Cloudflare tunnel to groot2 `:8421`. After local curl is green, repeat against `https://dregg-svg.permissionless.money`.

---

## Track B — lab soundness on groot-wan (do not skip)

Regtest cookie path (no Valar, no mainnet):

```bash
ssh groot-wan
cd ~/abstract/terp-core/crates/zakura/deploy/akash/zcash-zakura-lwd/local-cookie-harness
bash run.sh
# LWD on host 127.0.0.1:19067  (compose maps 19067:9067)
```

Then a **lab** `zcash-devtool` (aarch64 ELF is fine **on this host only**):

```bash
export ZCASH_DEVTOOL_BIN=/path/on/groot-wan/zcash-devtool
export ZCASH_DEVTOOL_SERVER=127.0.0.1:19067
export ZCASH_DEVTOOL_NETWORK=regtest
export ZCASH_DEVTOOL_ACTIVATION_HEIGHTS=/path/to/activation.toml
```

ict-rs e2e (`svg_mint_local_prod`) already skips ZEC honestly (`zec_skipped` + reason) when this is unset. Do **not** mark ZEC sold-out when the streamer is down.

---

## Do not

- Point DREGG at Akash `zcash-lwd.terp.network` — public LWD was a scaffold; current product is **local LWD on groot2**.
- Orange-cloud a gRPC hostname (breaks HTTP/2). Grey/DNS-only if you ever expose 9067.
- Use pruned Valar for LWD.
- Copy groot-wan ELFs onto groot2.
- Set `DREGG_ZEC_UA` without a live CompactTxStreamer.
- Treat `GET /health` HTTP 200 as “ZEC ready”.
- Open the FE ZEC button until pay-targets `zec_uri` is non-null on the **public** origin.

---

## Related files

| Doc | Why |
|---|---|
| `crates/zakura/deploy/akash/zcash-zakura-lwd/LAB-GROOT2.md` | groot2 compose after snapshot |
| `…/PRODUCT-DESIGN-HANDOFF-LOCAL-FIRST.md` | sentry vs house; VCT footgun |
| `…/dregg-env.example` | env names (Akash 9067 example — **override** to `127.0.0.1:9067` for live DREGG) |
| `…/local-cookie-harness/` | groot-wan regtest cookie proof |
| `crates/dregg/svg-mint/SPAWN-CHECKLIST.md` | groot2 fed spawn |
| `crates/dregg/svg-mint/FE-HANDOFF.md` | ZIP-321 contract (hide ZEC if null) |
| `crates/ict-rs/ict-rs/src/chain/zcash_devtool.rs` | lab wallet CLI wrapper |

---

## Ask of this teammate

1. Finish **archive extract on groot2** `/lab/zakura-mainnet` (rsync from groot-wan if that is where the tarball already is).
2. `docker compose up -d` with **treestate** Zakura; wait until LWD cache ≈ tip.
3. Install **amd64** `zcash-devtool` + UFVK dir on groot2.
4. Export `ZCASH_DEVTOOL_*` into the **running** `svg-mint-fed` and restart it.
5. Paste a redacted pay-targets JSON (`zec_uri` / `memo_hex` only) proving ZIP-321 is live.

Until step 5, ZEC stays hidden. ETH/Base/BTC and native mint do not wait on this.

**Multichain generalization (BTC/ETH/Base porch + house):** `PORCH-MULTICHAIN-HANDOFF.md` — same local-first shape; infra teammate critiques + lands the workflow; Akash lane ships one multi-container P2P porch.
