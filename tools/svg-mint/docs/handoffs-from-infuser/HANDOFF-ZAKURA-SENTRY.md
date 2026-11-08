# Handoff — Akash Zakura porch (sentry) for house door

**To:** infra / oline specialist (Akash + oline deploy tooling).  
**From:** pweb SVG mint (DREGG / ztreamer / zcli).  
**Do not:** JSON-RPC the lease, expose LWD, close Palmito/Headscale/Phase G, bind house `0.0.0.0:8233`.

House logic is already implemented. **You only respawn the public porch and stitch one peer.** Pay stays fail-closed until that stitch (`zec_tip` `private_unreachable` today).

## Goal

Two nodes:

| | Porch (you) | Door (already coded) |
|---|---|---|
| Host | Akash `zakuracore/zakura:1.3.0` pruned | groot2 `zakura-lab` |
| Public | **TCP 8233 only** | nothing |
| PEX | yes, public Zcash mesh | **no** — peerset 1, sentry-only |
| RPC / LWD | none | loopback `:8232` cookie + **ztreamer `:9067`** |

House must not learn extra peers. Sentry must not advertise house.

## SDL

Use `~/.oline/zakura-mainnet-sentry-pruned.yml` (already on groot-wan).

- Image **1.3.0**, pruned Valar ~11GiB.
- Expose **8233**. No 8232, no 9067, no CompactTxStreamer.
- `peerset_initial_target_size` ~32, DNS seeds on.
- **Omit** `ZAKURA_NETWORK__CACHE_DIR:"false"` (string path bug).
- Two-step oline: `oline deploy --sdl …` then `oline deploy --select DSEQ PROVIDER`. Prefer h6i.

Last porch **28414060** was CLOSED; BME REST 500 blocked a later `--sdl`. Retry when `api.akashnet.net` `/akash/bme/v1/status` is healthy.

## After `--select`

1. Note `PROVIDER_HOST:MAPPED_8233`.
2. On **groot2** house toml (`/lab/zakura-mainnet` / compose):
   - `initial_mainnet_peers = ["HOST:PORT"]` **only**
   - `listen_addr = "127.0.0.1:8233"`
   - `peerset_initial_target_size = 1`
   - `crawl_new_peer_interval = "24h"`
   - `cache_dir = false` (toml bool, not env string)
   - wipe on-disk `mainnet.peers`
   - do not publish WAN 8233
3. Recreate `zakura-lab`. Recopy cookie → `~/.terp/zakura.rpc.cookie` mode **600**.
4. Prove:
   - `getpeerinfo` length **1**, addr is porch
   - `getblockcount` vs public tip ≤ 2
   - `ztreamer probe --bind 127.0.0.1:9067` → `vendor=Ztreamer`
   - DREGG `/health` `zec_tip.ok` and `checkout_ready` (other gates aside)

Snippet: `dregg/svg-mint/deploy/house-zakura.toml.snippet`.

## What mint already expects

- `ZTREAMER_GRPC=127.0.0.1:9067` (alias of old `ZCASH_DEVTOOL_SERVER`)
- `ZTREAMER_BIN` runs `ztreamer probe` fail-closed unless vendor is **Ztreamer**
- `ZCLI_ENDPOINT` loopback only (wrapper `scripts/zcli-house.sh` refuses rotko/zec.rocks)
- systemd unit: `crates/ztreamer/deploy/ztreamer.service` (`--cache-dir` onto house RocksDB)
- Design: `dregg/svg-mint/PORCH-DOOR-ZCLI.md`

zcli still encodes **Orchard** unified addresses in `address.rs`; Ironwood is in witness/tx builders. CompactTxStreamer (ztreamer) already separates `ironwood_actions`. UA Ironwood receiver is a follow-up on the mint desk, not this deploy.

## Out of scope for this ticket

- Replacing HashMerchant onboard
- Public ztreamer on Akash
- Opening house PEX “to fix sync”
