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
