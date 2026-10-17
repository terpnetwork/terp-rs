# cw-oracle-registry

Permissionless **price-bound** store for Osmosis TWAP (and later VE mids). Bounds only. Never mints.

## Two ingresses, one record

| Source | Who writes | Cost |
|---|---|---|
| `Relayer` | First sidecar / packet processor to `Submit` at `(market_id, height)` | Optional bounty from contract balance |
| `Polytone` | Note ACK `{ "callback": CallbackMessage }`. Query payload is Osmosis `arithmetic_twap` JSON. | Packet gas |
| `VeQuorum` | `sudo { hash_merchant }` when `algo = price_median_v1` | Free to readers (validators already voted) |

Bind the note with `SetNote` (or instantiate `note`). Only that address may `Callback`. `initiator_msg` is JSON `{ market_id, source_chain, pool_id, height }`. First `(market_id, height)` still wins.

Consumers always `Latest` / `History` / `Twap`. They do not talk to Osmosis RPC.

## Fractal

Anyone instantiates `cw-oracle-aggregator` pointed at this registry and TWAPs / medians in their own contract. A second registry instance is also valid if a community wants its own markets.

## HashMerchant sidecar

Sidecars listen for work (new Osmosis height, unpaid bounty) and race `Submit`. That is the IBC-v2-relayer incentive shape without putting polytone inside Axum.

Vote-extension Connect path stays available via sudo. It is not required to use the registry.
