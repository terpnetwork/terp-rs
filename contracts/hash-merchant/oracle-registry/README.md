# cw-oracle-registry

Permissionless **price-bound** store for Osmosis TWAP (and later VE mids). Bounds only. Never mints.

## One record, several writers

| Source | Who writes | Cost |
|---|---|---|
| `Relayer` | First sidecar / packet processor to `Submit` at `(market_id, height)` | Optional bounty from contract balance |
| `Polytone` | Note ACK `{ "callback": CallbackMessage }`. Query payload is Osmosis `arithmetic_twap` JSON. | Packet gas |
| `VeQuorum` | `sudo { hash_merchant }` when `algo = price_median_v1` | Free to readers (validators already voted) |
| `DexQuote` | IBC v2 ack on port `dex-quote`. Posted position `p/q`, not a swap fill. | Packet gas |

`RegisterIbcV2` (admin) stores this chain's client id. `RequestQuote` emits one `Ibc2Msg::SendPacket`. `RequestQuotes` puts up to 16 of those requests in the same packet under `{"quotes":[...]}`. The payload carries each `market_id`, the filing height, the pair, and an optional position id. `ibc2_packet_ack` files a mid for each success. A per-quote error, a universal error ack, or a timeout files nothing for that quote. `Callback` is unchanged.

Bind the note with `SetNote` (or instantiate `note`). Only that address may `Callback`. `initiator_msg` is JSON `{ market_id, source_chain, pool_id, height }`. First `(market_id, height)` still wins.

Consumers always `Latest` / `History` / `Twap`. They do not talk to Osmosis RPC.

## Fractal

Anyone instantiates `cw-oracle-aggregator` pointed at this registry and TWAPs / medians in their own contract. A second registry instance is also valid if a community wants its own markets.

## HashMerchant sidecar

Sidecars listen for work (new Osmosis height, unpaid bounty) and race `Submit`. That is the IBC-v2-relayer incentive shape without putting polytone inside Axum.

Vote-extension Connect path stays available via sudo. It is not required to use the registry.
