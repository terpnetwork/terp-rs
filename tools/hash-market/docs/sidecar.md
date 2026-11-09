---
title: Sidecar
description: Running and configuring the hash-market validator sidecar
---

# Sidecar Setup

The hash-market sidecar runs alongside a Terp validator. It consists of two binaries that can run on the same or separate machines.

## Quick start

```bash
cd tools/hash-market

# Build both binaries
cargo build --release --features server
cargo build --release --features client

# Copy example configs
cp config.example.toml config.toml
cp client.example.toml client.toml

# Generate a signing key
openssl rand -hex 32
# Put the output in config.toml → signing_key

# Start the server (alongside your validator)
./target/release/hash-market-server -c config.toml

# Start the client (can run anywhere with ETH RPC access)
./target/release/hash-market-client -c client.toml
```

## Server configuration

The server uses a `[[providers]]` array — each entry is a named data source for a specific foreign chain. Multiple providers run concurrently, similar to how [Skip Connect](https://github.com/skip-mev/connect) registers multiple oracle providers.

```toml
# config.toml — non-loopback bind refuses to start unless allow_public_bind
bind = "127.0.0.1:9090"
chain_id = "terp-mainnet-1"
signing_key = "abcdef..."

[[providers]]
name = "ethereum-mainnet"
chain_uid = "ethereum-mainnet"
algo = "keccak256"
mode = "grpc"
address = "0.0.0.0:9091"

[[providers]]
name = "arbitrum-one"
chain_uid = "arbitrum-one"
algo = "keccak256"
mode = "grpc"
address = "0.0.0.0:9092"

[[providers]]
name = "cosmoshub"
chain_uid = "cosmoshub-4"
algo = "sha256"
mode = "http_poll"
address = "http://cosmos-poller:8080/data"
interval_secs = 6
```

Each provider has:

| Field | Required | Description |
|---|---|---|
| `name` | yes | Human-readable label (shown in logs and `/providers`) |
| `chain_uid` | yes | Must match a `RegisteredChain` in the hashmerchant module |
| `algo` | no | Hash algorithm, default `"keccak256"` |
| `mode` | yes | Transport: `"grpc"`, `"http_poll"`, or `"websocket"` |
| `address` | yes | Listen address (grpc) or remote URL (http_poll, websocket) |
| `interval_secs` | no | Polling interval for http_poll, default `12` |

### Transport modes

| Mode | `address` field | Use case |
|---|---|---|
| `grpc` | Listen address (e.g. `0.0.0.0:9091`) | Client on same machine or LAN |
| `http_poll` | URL to poll (e.g. `http://client:8080/data`) | Client behind a firewall |
| `websocket` | WebSocket URL | Real-time streaming (requires `tokio-tungstenite`) |

### API endpoints

| Endpoint | Method | Purpose |
|---|---|---|
| `/health` | GET | Liveness + provider status + oracle bounds summary |
| `/providers` | GET | List all providers with status, last update time, foreign height (under `/ve` when nested) |
| `/oracle/bounds` | GET | Connect-style aggregated mid (`?market_id=ETH/USD`); `role=bound_only` |
| `/oracle/ticks` | GET | Raw attributed source ticks (debug) |
| `/extend-vote` | POST | Produce a signed vote extension for a chain |
| `/verify-vote-extension` | POST | Verify a peer's signed extension |

### Price bounds (Connect-style)

Multi-source pricing aggregates **off-chain** in the sidecar (Skip Connect pattern): many HTTP tickers → median → **one mid per `market_id`**. See `oracle-connect-bounds.md`.

Ticker URL bodies should look like `{"price":"3450.12"}` or `{"price":3450.12,"timestamp":…}`.

### `/extend-vote` request

```json
{
  "height": 12345,
  "chain_uid": "ethereum-mainnet",
  "algo": "keccak256"
}
```

`chain_uid` is required when multiple providers are registered. If only one provider has data, it is used automatically. The response includes the chain metadata:

```json
{
  "chain_uid": "ethereum-mainnet",
  "algo": "keccak256",
  "foreign_height": 19500000,
  "extension": "0a09...",
  "signature": "3045...",
  "public_key": "02ab..."
}
```

### `/providers` response

```json
[
  {
    "name": "ethereum-mainnet",
    "chain_uid": "ethereum-mainnet",
    "algo": "keccak256",
    "running": true,
    "last_update": 1700000000,
    "foreign_height": 19500000
  },
  {
    "name": "arbitrum-one",
    "chain_uid": "arbitrum-one",
    "algo": "keccak256",
    "running": true,
    "last_update": 1700000012,
    "foreign_height": 250000000
  }
]
```

## Client configuration

```toml
# client.toml
eth_rpc = "https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY"
sidecar_url = "http://localhost:9090"
runtime_id = "eth-poller-1"
chain_uid = "ethereum-mainnet"
interval_secs = 12
account_address = "0x..."
storage_keys = []
```

The client polls `eth_getBlockByNumber` and `eth_getProof` every `interval_secs`, transforms the state root through Keccak256 → Pallas Fp reduction, and sends the result to the server's transport endpoint.

Run one client instance per foreign chain. Each connects to a different provider port on the server:

```bash
# Client for Ethereum mainnet → server provider on :9091
./hash-market-client -c eth-mainnet.toml

# Client for Arbitrum → server provider on :9092
./hash-market-client -c arbitrum.toml
```

## Custody

See the [Custody](./custody) page for the full guide. Summary:

| Backend | Config | Use when |
|---|---|---|
| Local secp256k1 | `signing_key = "hex..."` | Devnet, testing |
| TKMS | TCP to external KMS | Mainnet, shared infra |
| Custom | Implement `Custody` trait | Cloud KMS, HSM, threshold |

## Feature flag reference

Build only what you need:

```bash
# Types only (for importing in other crates)
cargo build --features msg

# Server without client deps
cargo build --features server

# Client without server deps
cargo build --features client

# Everything
cargo build --features server,client
```

| Feature | Pulls in | Size impact |
|---|---|---|
| `msg` | anybuf | Minimal |
| `custody` | k256, ed25519-dalek, tokio, serde | ~2MB |
| `ve` | custody + sha2 | +100KB |
| `eth` | reqwest, serde_json | +4MB (TLS) |
| `pallas` | tiny-keccak | +50KB |
| `transport` | tokio (rt, net, sync) | +1MB |
| `server` | transport + custody + ve + axum | ~8MB total |
| `client` | eth + pallas + reqwest + tokio | ~6MB total |

## Logging

Both binaries use `tracing` with `RUST_LOG` env filter:

```bash
RUST_LOG=info ./hash-market-server -c config.toml
RUST_LOG=hash_market=debug ./hash-market-client -c client.toml
```
