# ZEC watch via ztreamer (CompactTxStreamer, not stock LWD)

**Audience:** DREGG / ict-rs teammates replacing `electriccoinco/lightwalletd` as the house/lab compact streamer.  
**Upstream:** https://github.com/distractedm1nd/ztreamer  
**Related:** `ZEC-P2P-PIR-HANDOFF.md` (PIR ≠ deposit discovery), `ZEC-INFRA-HANDOFF.md` (house ports), `PORCH-SLICE2-BTC-PROVE.md` (e2e style).  
**Workflow:** `/workflow dregg-zec-ztreamer {"mode":"lab-e2e"}`

Not FE. Not public Akash 9067.

---

## Product lock

| Surface | Role |
|---|---|
| Archive/regtest **Zakura** | Consensus + raw block bytes |
| **ztreamer** | CompactBlock index + CompactTxStreamer gRPC (vendor `Ztreamer`) |
| `zcash-devtool` / `watch_zec_devtool` | UFVK sync → ZIP-321 deposit discovery |
| Stock LWD | Backup only until lab-e2e green |
| Spendability PIR | Spend/refund later — does **not** see new notes |

Phase 1: gRPC drop-in for `ZCASH_DEVTOOL_SERVER=127.0.0.1:19067` (lab) / `:9067` (house).  
Phase 2: Zakura `CustomService` compactstreamer over P2P (upstream TODO; not the lab gate).

---

## Why this vs LWD / Zaino

- Parses **raw bytes from a Zakura DB handle** (embeds `zakurad` / `NodeClient`) — skips field-element de/re-compress (~91% CPU in Zaino).
- Pipelined fetch → parse → LMDB write; concurrent initial index.
- Seal after 100 / persist after 10 (ranges of 1000). Tip-proximal mint deposits are not stuck in Zaino-style 1000-conf passthrough.
- `GetLightdInfo.vendor` is **`Ztreamer`** — e2e proof that we are not talking to Go LWD.

Porch pruned Zakura is not enough for historical compact. House still needs archive/treestate.

---

## Lab e2e (groot-wan, same style as BTC)

Do **not** start `local-cookie-harness` lightwalletd.

```sh
# on groot-wan; PATH includes /usr/local/bin (docker)
export ZAKURAD_BIN=$HOME/abstract/terp-core/crates/zakura/target/zakura-regtest-e2e-linux/zakurad
export DREGG_NODE_BIN=$HOME/abstract/terp-core/crates/dregg/svg-mint/target/linux-gnu/svg-mint-fed
# optional: ZCASH_DEVTOOL_BIN linux ELF
bash crates/dregg/svg-mint/scripts/lab-e2e-zec.sh
```

`ultralight-local-e2e.sh` = contrast + BTC cargo + this ZEC script.

**Sound** when cargo JSON has:

| Key | Value |
|---|---|
| `live` | `true` |
| `compact_vendor` | contains `Ztreamer` |
| `lwd_absent` | `true` |
| `compact_grpc` | loopback host:port |
| `btc_recorded` | still `true` (do not drop BTC) |

`zec_skipped` is allowed if the streamer still proved. Ironwood escrow entitle **without** `compact_vendor=Ztreamer` is **not** enough.

Fail-closed: public LWD, `:8232`, quoting `.rs` as live, `docker ps` showing `lightwalletd`.

---

## Layout

```text
crates/zakura/                     ← path dep for ztreamer
crates/ztreamer/                   ← clone of distractedm1nd/ztreamer (Cargo.toml ../zakura/crates/...)
crates/ztreamer/src/bin/ztreamer.rs ← lab daemon (upstream has no [[bin]])
crates/ztreamer/src/node.rs        ← NodeClient over zakura_state
crates/ztreamer/src/custom_service.rs ← P2P CustomService stub (remaining=true)
svg_mint_local_prod                ← spawn streamer sibling; set ZCASH_DEVTOOL_SERVER
```

`ZakuraSource::new(&NodeClient)` needs an **in-process** zakurad. ict-rs `spawn_zakura_local` is Docker-only (RPC URL, no `NodeClient`). Lab spawns an embedded zakura_state + ztreamer process for the compact surface. JSON: `compact_surface=in-process-zakurad+ztreamer`.

---

## Lab spawn (groot-wan, Zallet)

```text
/workflow dregg-groot-wan-ztreamer
bash crates/dregg/svg-mint/scripts/spawn-groot-wan-ztreamer.sh
```

Binds ztreamer `127.0.0.1:19067` + `svg-mint-fed` `127.0.0.1:8421`. Stock LWD stays down. FE: `FE-ZTREAMER-ZALLET.md`.

---

## House (groot2) — later, human gate

`mode=house-up`: point `ZCASH_DEVTOOL_SERVER=127.0.0.1:9067` at ztreamer only after GetLightdInfo vendor is Ztreamer. Do not kill a catching-up LWD until that is true.

---

## Remaining (not lab gate)

- compactstreamer **CustomService** over Zakura P2P (last RPC) — `remaining=true` in `crates/ztreamer/src/custom_service.rs`. Do **not** block `lab-e2e-zec.sh`.
- daemon/logging/observability in upstream
- wall-clock Sapling→tip vs LWD 0.5.3 / Zaino / Zinder on the same datadir
