# Local endpoints for IBC scripts (groot-wan -> groot2 tunnels, all bound to 127.0.0.1)

| Var | Value | groot2 target |
|---|---|---|
| MOROCCO_GRPC | http://127.0.0.1:19090 | terpd mainnet gRPC 127.0.0.1:9091 |
| (RPC) | http://127.0.0.1:26658 | terpd RPC 127.0.0.1:26657 |
| (LCD) | http://127.0.0.1:11317 | terpd LCD 1317 |
| PENUMBRA_RPC | http://127.0.0.1:27667 | cometbft-penumbra RPC 127.0.0.1:27657 |

Penumbra tunnel: `ssh -fN -o ExitOnForwardFailure=yes -L 127.0.0.1:27667:127.0.0.1:27657 groot2`.
Without MOROCCO_GRPC set, the scripts default to the LAN gRPC http://192.168.10.101:9090.
Do not combine `drift=` with `substitute_for=`: the drift is copied from the subject client.

Hermes on groot2 (hermes-penumbra.service, /storage/penumbra/hermes.toml) uses the same local
nodes: morocco-1 rpc 127.0.0.1:26657 / grpc 127.0.0.1:9091, penumbra-1 rpc 127.0.0.1:27657 / grpc 127.0.0.1:28080.

## Unified relayer (2026-10-27)
- One Hermes on groot2: `hermes-penumbra.service` running the Rotko Penumbra-native build
  (`/storage/penumbra/bin/hermes`, 1.8.2) with `/storage/penumbra/hermes.toml` and
  key store `/storage/penumbra/hermes-home/keys` (all files 600).
- Chains: penumbra-1, morocco-1 (rly-terp), osmosis-1 (rly-osmo), juno-1 (rly-juno),
  akashnet-2 (rly-akash), atomone-1 (rly-atomone). The Penumbra signer is the inline `kms_config`
  of the penumbra-1 block (same as the old groot-wan config); its view DB is `/storage/penumbra/hermes-view`.
- Spare keys also in the store (no chain block): cosmoshub-4 rly-cosmos, jackal-1 rly-jackal, bitsong-2b rly-bitsong.
- The groot-wan `~/.ibc-hermes` + `network.terp.ibc-hermes` launchd agent is the old copy: disabled, not running.
- Juno uses a pull event source (2s), because the polkachu websocket returns 400.
- The penumbra-1 packet filter is `allow` with an empty list, so nothing is relayed until a Terp channel is added.
- Penumbra fees are paid in `upenumbra`. The "denom not found" wallet warning means the relayer view
  holds no notes, so fund the relayer address before any Penumbra tx.
- groot-wan launchd `network.terp.penumbra-rpc-tunnel` keeps 127.0.0.1:27667 -> groot2 127.0.0.1:27657 up.
