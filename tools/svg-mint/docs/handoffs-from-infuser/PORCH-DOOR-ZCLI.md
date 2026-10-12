# PWeb SVG mint — porch / door + zcli + ztreamer

Locked design for **two Zakura nodes + one CompactTxStreamer + one Ironwood wallet**.
Prepare on groot-wan; execute on groot2 house + Akash porch. Do not JSON-RPC the Akash lease.

Status 2026-09-10: house `svg-mint-fed` Pay is closed (`zec_tip` `private_unreachable`). Public porch sentry lease was CLOSED; house has 0 peers. This document is the rebuild contract.

## Roles

| Name | Where | Public? | Job |
|---|---|---|---|
| **Porch** | Akash Zakura `zakuracore/zakura:1.3.0` pruned | **P2P `:8233` only** | Sync the public Zcash mesh. Never JSON-RPC, never LWD, never advertise house. |
| **Door / house** | groot2 `zakura-lab` | **loopback only** (`127.0.0.1:8233` P2P, `:8232` cookie RPC, ztreamer `:9067`) | Archive (or durable pruned) + indexer + wallet + DREGG. One outbound peer: the porch. |
| **Indexer** | `crates/ztreamer` **v0.1.0** | loopback CompactTxStreamer | Replaces ECC `lightwalletd`, zcli **zidecar**, and `zcash-devtool` as the scan source. |
| **Wallet** | `crates/zcli` (`rotkonetworks/zcli`, WAN clone `9386a35`) | none | Replaces `ZCASH_DEVTOOL_BIN` for UA / ZIP-321 / trial decrypt. Ironwood-aware. |

FE still talks **only** to DREGG TLS (`dregg-svg.permissionless.money`). Never ztreamer, never `:8232`.

## Does zcli have Ironwood support?

**Yes, in the wallet/witness path — not just a README claim.**

- `bin/zcli/src/witness.rs`: `Pool::Ironwood` vs `Pool::Orchard`, `get_ironwood_tree_state`, compact `ironwood_actions`. Same Merkle types as Orchard but **separate trees**; mixing them is `UnknownIronwoodAnchor`.
- `crates/zcash-wasm`: `regtest_ironwood_e2e.rs` (t→z / z→t v6 vs zebrad NU6.3), `frost_ironwood_send_v6.rs`, turnstile orchard→ironwood (separate delayed-activation chain).
- Top-level README still says “orchard shielded pool” for the CLI blurb. **PWeb mint must use Ironwood UAs** after NU6.3 (mainnet 3,428,143). Do not derive Orchard-only unified addresses for Pay.

Gap to close in wiring: `zcli sync` default server is **zidecar** (their native indexer). We point it at **ztreamer** CompactTxStreamer (`127.0.0.1:9067` house / `:19067` lab) so Ironwood tree-state RPCs come from our proto (`ironwoodTree`, `ShieldedProtocol.ironwood = 2`). zidecar is not the house indexer.

## Indexer: ztreamer instead of native zidecar / LWD / zcash-devtool

ztreamer v0.1.0 already:

- Parses **v6** and keeps orchard vs ironwood actions separate (`parser.rs`, `codec.rs`).
- Serves tonic `CompactTxStreamer`, `GetLightdInfo.vendor = Ztreamer`.
- **Refuses** bind `:8232` and **refuses** non-loopback binds.

Must-fix before it powers mint (today the bin starts an **ephemeral** `NodeClient` — lab only):

1. Attach to **house Zakura state** (same cache dir as `zakura-lab`), not `start_ephemeral`.
2. Bind house `127.0.0.1:9067` (keep lab `19067`).
3. Systemd + logging (README still lists daemon/observability as TODO).
4. DREGG env: `ZCASH_DEVTOOL_SERVER=127.0.0.1:9067` until the flag is renamed `ZTREAMER_GRPC`; drop `ZCASH_DEVTOOL_BIN` once `zcli` can `sync`/`balance` against that gRPC.
5. Fail-closed if `GetLightdInfo.vendor != Ztreamer` or if compact blocks lack `ironwood_actions` after Ironwood height.

zcli **zidecar** stays unused on house. Do not run ECC `lightwalletd` containers.

## Porch / door (Akash) — gossip lock

Cosmos analogue: porch `pex=true` with the public mesh; door `pex=false` + private persistent peer.

**Porch SDL** (`~/.oline/zakura-mainnet-sentry-pruned.yml`):

- Image `zakuracore/zakura:1.3.0`, pruned Valar snapshot.
- Expose **TCP 8233 only**. No LWD, no 8232, no 9067.
- `peerset_initial_target_size` ~32, DNS seeds on, `cache_dir` default (public addr-book OK).
- **Do not** set `ZAKURA_NETWORK__CACHE_DIR:"false"` as a string (that becomes a path).
- Never put the house `external_addr` or WAN IP in porch config.
- Two-step oline: `--sdl` then `--select DSEQ PROVIDER`. Prefer h6i. Do not close Palmito/Headscale/Phase G for this.

**Door / house** (`/lab/zakura-mainnet` on groot2):

- `initial_mainnet_peers` = **only** `porch_host:mapped_8233`.
- `peerset_initial_target_size = 1`, `crawl_new_peer_interval` 24h, `cache_dir = false`, wipe `mainnet.peers` on recreate.
- Listen `127.0.0.1:8233` (not `0.0.0.0`). No WAN publish of 8233.
- Cookie RPC `127.0.0.1:8232` → `~/.terp/zakura.rpc.cookie` mode 600 for DREGG `zec_tip`.
- After porch `--select`, rewrite `initial_mainnet_peers`, recreate zakura-lab, recopy cookie, prove **1 outbound peer** and `zec_tip.ok`.

If porch dies: Pay fail-closes (`zec_tip` drift / private_unreachable). **No refunds from that circuit.** FE hides Pay. Do not open house to the public mesh to “fix” sync.

## Two-node + indexer maintenance workflow

1. **Porch** — oline deploy pruned sentry; wait logs for P2P listen; write `host:port` into house toml.
2. **Door** — compose up zakura-lab; `getpeerinfo` length 1 and addr is porch; `getblockcount` vs public tip ≤ 2.
3. **ztreamer** — systemd `ztreamer.service` after zakura; `GetLightdInfo` vendor Ztreamer; height tracks house.
4. **zcli** — watch-only / merchant: `zcli -w` (or SSH-key seed on an air-gap later). `sync` against ztreamer. Confirm Ironwood notes, not Orchard-only.
5. **DREGG** — `ZCASH_DEVTOOL_SERVER=127.0.0.1:9067`; `zec_tip` uses house cookie vs public getblockcount; Pay only if `checkout_ready`.
6. **Onboard** — unchanged HashMerchant passkey gas; not this ZEC wallet.

Lab on groot-wan: ztreamer `:19067` + ephemeral node is allowed. Live mint is **groot2 house only**.

## Env (house)

```
ZTREAMER_BIND=127.0.0.1:9067
ZTREAMER_INDEX=/var/lib/ztreamer
ZAKURA_COOKIE=~/.terp/zakura.rpc.cookie
ZCASH_PRIVATE_RPC=http://127.0.0.1:8232
ZCASH_PUBLIC_RPC=<self-hosted getblockcount, not Infura>
ZCASH_TIP_MAX_DRIFT=2
ZCASH_DEVTOOL_SERVER=127.0.0.1:9067
# ZCASH_DEVTOOL_BIN retired once zcli is the mint wallet
```

## Not in this cut

- ETH/Base/BTC house watchers
- Public CompactTxStreamer on Akash
- zidecar on the porch
- Gossiping house into sentry addr-book
- Closing oline static/minio leases to “fix” ZEC
