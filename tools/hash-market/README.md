# hash-market

**Unified Rust sidecar for vote extensions (ABCI++), BUD-compatible blob serving, and Merkle tree operations on the Terp Network.**

A modular, feature-gated server that acts as a validator sidecar (producing and verifying `ExtendVote` extensions for CometBFT) while simultaneously serving as a BUD/NIP-compatible blob store with file-based persistence, a QMD-style content-ID index, and Merkle tree CRUD for headstash/whitelist workflows.

---

## Feature Support Matrix

| Feature | Flag | Description | Binaries |
|---|---|---|---|
| **Server** | `server` | Axum HTTP server core with routing, middleware, tower services | `hash-market-server` |
| **Onboard** | `onboard` | DAO feegrant plane at `/onboard/*` (included with `server`; not a second process) | `hash-market-server`, `prepare-authz` |
| **Vote Extension** | `ve` | ABCI++ ExtendVote / VerifyVote extension signing + verification | `hash-market-server` |
| **Blossom (BUD)** | `blossom` | BUD-02/06/07 blob storage protocol (auth, payment, hash-addressed blobs) | `hash-market-server` |
| **Client** | `client` | ETH proof polling, hash data collection, HTTP client to sidecar | `hash-market-client` |
| **Nostr** | `nostr` | NIP-77 relay integration, NEG-ENTROPY, root discovery via relays | `hash-market-relay` |
| **Custody** | `custody` | secp256k1 / ed25519 key management and signing backend | — (lib) |
| **FROST / Noise privval** | `privval` | FROST-Ed25519 coordinator + Noise XX in this sidecar (not a second kms ELF) | `hash-market-server` |
| **Transport** | `transport` | gRPC / HTTP poll / WebSocket data ingestion modes | `hash-market-server` |
| **ETH** | `eth` | `eth_getProof` client for EVM chain state extraction | `hash-market-client` |
| **Pallas** | `pallas` | Keccak → Pallas field element transform (Verkle-ready) | `hash-market-client` |
| **Commonware** | `commonware` | Commonware-storage-backed persistence (future) | — (lib) |

### Default features

```toml
default = ["nostr", "blossom"]   # server without vote extensions
```

### Mint / whitelist host (VE off)

Primary production mode for NFT mint pages:

```toml
# config.toml
ve_enabled = false
bind = "0.0.0.0:9090"
chain_id = "morocco-1"
data_dir = "/var/lib/hash-market"
# no signing_key or [[providers]] required
```

```bash
# Build image without VE (Dockerfile default FEATURES)
docker build -t hash-market:local -f tools/hash-market/Dockerfile .

# Public APIs for frontends (terp.network svg mint)
# GET  /health
# GET  /trees
# GET  /trees/{id}
# GET  /trees/{id}/members/{addr}  → proof_hashes for fee-bypass mint
# POST /trees/{id}  (auth)         → upload gen_merkle JSON
```

Frontend: `websites/terp.network/lib/merkle-server.js` + `config.services.merkleServer`.

### Validator sidecar (VE on)

```bash
cargo build -p hash-market --features full-sidecar --release
# config: ve_enabled = true, signing_key, [[providers]]
# FROST privval: [privval] enabled=true, frost shares + group (see reviews/HANDOFF-FROST-SIDECAR-DOCS-2026-09-03.md)
# Exercise: ./scripts/e2e-frost-privval.sh
```

---

## Binaries

| Binary | Required Features | Entry Point | Purpose |
|---|---|---|---|
| `hash-market-server` | `server` | `src/bin/server.rs` | Unified VE + Blossom + headstash server |
| `hash-market-client` | `client` | `src/bin/client.rs` | ETH proof poller → sidecar feeder |
| `hash-market-relay` | `nostr` | `src/bin/relay.rs` | Nostr relay interaction (root posting) |

---

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                  hash-market-server                  │
│                                                      │
│  ┌────────────────────┐   ┌──────────────────────┐  │
│  │  VE Router         │   │  Blossom Router      │  │
│  │  /ve/health        │   │  GET/POST /blobs     │  │
│  │  /ve/providers     │   │  GET/DELETE /blobs/h │  │
│  │  /ve/extend-vote   │   │  POST /upload        │  │
│  │  /ve/verify        │   └──────────┬───────────┘  │
│  └────────┬───────────┘              │              │
│           │                          │              │
│           ▼                          ▼              │
│  ┌──────────────────────────────────────────────┐  │
│  │              AppState (Arc)                   │  │
│  │  ┌──────────┐ ┌───────────────────────────┐  │  │
│  │  │VE Handler│ │ BlossomState              │  │  │
│  │  │(custody) │ │  └─ store: BlobStore trait │  │  │
│  │  └──────────┘ │     └─ TreeStore ───────┐ │  │  │
│  │               │                          │ │  │  │
│  │  ┌────────────┴─────────────────────────┘ │  │  │
│  │  │ TreeStore (sole BlobStore impl)        │  │  │
│  │  │  ├── {id}.json        — Merkle trees   │  │  │
│  │  │  ├── f/{hash}         — Blob files     │  │  │
│  │  │  └── ContentIndex     — QMD-style idx  │  │  │
│  │  └────────────────────────────────────────┘  │  │
│  └──────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────┘
```

### Key design decisions

- **Blossom state delegates to TreeStore.** There is no separate in-memory blob store. `BlossomState::new(tree_store.clone())` receives an `Arc<dyn BlobStore>` pointing at `TreeStore`, the sole `BlobStore` implementation. The two previous implementations (`HashMarketBlobStore`, `QmdbBlobStore`) have been removed.
- **`/f/` prefix + QMD content-ID index.** Blobs are stored at `{data_dir}/trees/f/{hex(sha256)}` for standard file-store durability. A `ContentIndex` (lazily synced from disk, SHA256 → metadata) provides O(1) in-memory lookups without filesystem scans — mirroring the QMDB authenticated key-value pattern.
- **One `Arc<AppState>` for all routes.** The unified router in `src/server.rs` builds a single `Router` with VE routes nested at `/ve/*` and BUD/tree/headstash routes at root level — all sharing the same state.
- **BUD helpers from `cw721-nips`.** `server.rs` imports `check_scope`, `check_action`, `parse_hash`, `extract_payment_proof` from `nips::buds` rather than duplicating them.

---

## Requirements

| Dependency | Minimum | Notes |
|---|---|---|
| Rust | 1.84+ | Edition 2021, nightly not required |
| OpenSSL | 1.1+ | For `tokio-tungstenite` native-tls |
| musl-tools | — | Only for Alpine Docker build |
| Docker | 24+ | Only for container build / e2e tests |

**Optional runtime dependencies:**
- Ethereum JSON-RPC endpoint (for `hash-market-client`)
- Nostr relay (for `hash-market-relay`)
- CometBFT/Terp node (for VE sidecar pattern)

---

## Installation

### From source

```bash
# Clone the workspace
cd terp-core/crates/terp-rs

# Build the server with all features
cargo build --release --features "server,ve,blossom,client,nostr" -p hash-market

# Binaries land at:
# target/release/hash-market-server
# target/release/hash-market-client
# target/release/hash-market-relay
```

### Docker

```bash
# From crates/terp-rs workspace root
docker build -t hash-market:local -f tools/hash-market/Dockerfile .

# Run (config mount — never put signing_key in env)
docker run --rm -p 9090:9090 \
  -v /path/to/config.toml:/etc/hash-market/config.toml:ro \
  -v /path/to/data:/var/lib/hash-market \
  hash-market:local
```

### Akash via o-line (recommended)

```bash
# SDL: crates/o-line/templates/sdls/oline/hashmerchant.yml
# Secrets are SFTP'd after lease — never provider env
oline hashmerchant render
oline hashmerchant deploy
oline hashmerchant upload --dseq <D> --config ~/.oline/hashmerchant/config.toml
```

See `o-line` docs/examples/hashmerchant.md.

### Minimal build (server only)

```bash
cargo build --release --features "server" -p hash-market
```

---

## Configuration

The server expects a TOML config file (default `config.toml`, override with `-c` / `--config`):

```toml
bind = "0.0.0.0:9090"
chain_id = "terp-testnet-5"
signing_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
data_dir = "/var/lib/hash-market"

[[providers]]
name = "eth-mainnet"
chain_uid = "eip155:1"
algo = "keccak256"
mode = "http_poll"
address = "http://127.0.0.1:8545"
interval_secs = 12

[[providers]]
name = "arbitrum-one"
chain_uid = "eip155:42161"
algo = "keccak256"
mode = "grpc"
address = "0.0.0.0:9091"
```

### Fields

| Field | Required | Description |
|---|---|---|
| `bind` | Yes | Listen address `host:port` |
| `chain_id` | Yes | CometBFT chain ID for domain separation |
| `signing_key` | Yes | Hex-encoded secp256k1 private key |
| `data_dir` | No | Data directory (default: `data/`) |
| `providers` | Yes | Array of provider transport configs |

### Provider transport modes

| Mode | Description |
|---|---|
| `grpc` | gRPC listener at the given address |
| `http_poll` | Polls a remote HTTP endpoint at `interval_secs` |
| `websocket` | WebSocket subscription to a remote endpoint |

---

## API Endpoints

### Blossom / BUD (blob storage)

| Method | Path | Description | BUD Spec |
|---|---|---|---|
| `GET` | `/blobs/{hash}` | Retrieve blob by SHA256 hash | BUD-01 / BUD-02 |
| `DELETE` | `/blobs/{hash}` | Delete blob by SHA256 hash | BUD-02 |
| `GET` | `/blobs` | List all stored blob hashes | BUD-06 |
| `POST` | `/blobs` | Upload a blob (binary body) | BUD-07 |
| `POST` | `/upload` | Alias for `/blobs POST` | — |

All blob endpoints require JWT or secp256k1 auth via the `X-Auth-Type` header.

### Merkle trees

| Method | Path | Description |
|---|---|---|
| `GET` | `/trees/{id}` | Load a merkle tree by ID |
| `POST` | `/trees/{id}` | Upload/overwrite a merkle tree |
| `DELETE` | `/trees/{id}` | Delete a merkle tree |
| `GET` | `/trees` | List all tree IDs |

Upload format matches `gen_merkle --output`:

```json
{
  "merkle_root": "<64-char-hex>",
  "accounts": {
    "terp1abc...": { "tier": 1, "allocation": 5000, "proof_hashes": ["..."] }
  }
}
```

### Headstash

| Method | Path | Description |
|---|---|---|
| `GET` | `/headstash/{id}` | Get headstash registration |
| `POST` | `/headstash/{id}` | Save headstash registration |

### Vote Extension (VE)

| Method | Path | Description |
|---|---|---|
| `GET` | `/ve/health` | VE subsystem health + active providers |
| `GET` | `/ve/providers` | List registered providers |
| `GET` | `/ve/vote-extension` | Latest provider data (called by terpd each block) |
| `POST` | `/ve/extend-vote` | Produce signed vote extension |
| `POST` | `/ve/verify-vote-extension` | Verify a peer's signed extension |

### Health

| Method | Path | Description |
|---|---|---|
| `GET` | `/health` | Unified health: VE status + blob count + store info |

---

## BUD / NIP Compatibility

The blobs subsystem is designed for compatibility with:

| Spec | Description | Status |
|---|---|---|
| BUD-01 | Blob storage upload/download | ✅ Implemented |
| BUD-02 | Blob deletion, owner auth via NIP-98 | ✅ Implemented |
| BUD-06 | Blob list with scoped auth | ✅ Implemented |
| BUD-07 | Paid blobs (Lightning + Cashu) | ✅ Implemented |
| NIP-98 | HTTP Auth via Nostr event signing | ✅ Implemented via `AuthVerifier` |

BUD implementation lives in the `cw721-nips` crate at `crates/nips/src/buds/` and is re-exported through `nips::buds`. The canonical `blossom_router`, auth helpers, and blob handers are there; `hash-market` delegates to them.

---

## Testing

### Unit / integration tests (Rust)

```bash
# Check compilation with all features
cargo check --features "server,ve,blossom,client,nostr" -p hash-market

# Run library tests (if any)
cargo test --features "server,ve,blossom,client,nostr" -p hash-market
```

### E2E integration test (ict-rs + Docker)

The workspace `tests/` crate contains a full end-to-end test that:

1. Spawns a `hash-market-server` subprocess via `SubprocessConfig` / `SidecarBinaryResolver`
2. Uploads a mock merkle tree via `POST /trees/{id}`
3. Uploads blobs via `POST /blobs`
4. Verifies blob retrieval via `GET /blobs/{hash}`
5. Validates Merkle proof generation via `GET /trees/{id}`
6. Produces signed vote extensions via `POST /ve/extend-vote`
7. Tears down the sidecar

```bash
# Run the e2e binary directly
cd tests
cargo check --bin hashmerchant-e2e

# Build release
cargo build --release --bin e2e -p terp-scripts --features docker,nostr

# The main e2e.rs spins up the full stack (Terp chain, Anvil, sidecars)
# via ict-rs Docker orchestration.
```

### Binary path resolution

The test infrastructure resolves the server binary via:

```rust
SidecarBinaryResolver::cargo_binary([
    "../target/debug/hash-market-server",
    "../target/release/hash-market-server",
], "hash-market-server")
```

This resolves from the `tests/` manifest directory to the workspace `target/` directory.

---

## Data directory layout

```
{data_dir}/
  trees/
    {tree_id}.json        — Merkle trees (JSON)
    f/                    — Blob store (SHA256-addressed)
      {sha256_hex}        — Blob files
  headstash/
    {hs_id}.json          — Headstash registrations
  notes/
    {hs_id}/
      {addr}.json         — Encrypted notes per address
  keys/
    {key_id}.bin          — Circuit key binaries
    {key_id}.blake3       — BLAKE3 hash of the key file
```

---

## Feature flags reference

```toml
[features]
default = ["nostr", "blossom"]
custody  = ["k256", "ed25519-dalek", "rand_core", "async-trait", "tokio"]
ve       = ["custody", "sha2", "client"]
eth      = ["reqwest"]
pallas   = ["tiny-keccak"]
transport = ["tokio/rt", "tokio/sync", "tokio/net", "tokio/io-util",
             "tokio/time", "tokio/signal", "tokio/macros"]
server   = ["transport", "custody", "ve", "axum", "tower-http",
            "clap", "tracing", "tracing-subscriber", "toml",
            "tokio/full", "bytes"]
client   = ["eth", "pallas", "reqwest", "clap", "tracing",
            "tracing-subscriber", "toml", "tokio/full"]
nostr    = ["client", "custody", "tokio/full", "tokio-tungstenite",
            "futures-util", "url", "sha2", "async-trait"]
blossom  = ["server", "clap", "tower", "pin-project", "tokio-stream"]
commonware = ["server"]
```

---

## Development

### Module structure

```
src/
  bin/
    server.rs      — binary entry point (config parsing, startup)
    client.rs      — ETH proof poller binary
    relay.rs       — Nostr relay binary
  lib.rs           — crate root, feature-gated module exports
  fields/          — field arithmetic (Pallas/Vesta)
  hash/            — hash utilities
  msg/             — protobuf-style message types
  server.rs        — unified router, AppState, all HTTP handlers
  store.rs         — TreeStore, HeadstashStore, ContentIndex, BlobStore impl
  custody/         — key management (local secp256k1)
  transport/       — data ingestion (gRPC, HTTP poll, WebSocket)
  middleware/      — auth middleware (secp256k1, JWT)
  client/          — ETH, Nostr, hashmerchant, MinIO/IPFS clients
  ve/              — VE server routes, provider feeder
```

### Key constraints

- **Blob storage at `/f/` prefix** — filesystem-backed, SHA256 content-addressed. The `ContentIndex` is a QMD-style overlay for O(1) hash lookups that reindexes from disk on startup.
- **`BlossomState::store` is `Arc<dyn BlobStore>`.** Any existing implementation of the trait can be swapped in; `TreeStore` is the current (and sole) implementation.
- **Domain-separated VE signing.** The signing scheme uses `SHA256("terp/hashmerchant/ve/v1" || chain_id || height_be8 || ext_bytes)` so extensions cannot be replayed across chains or blocks.
- **Atomic writes.** Both tree saves and blob writes go through `.tmp` + `rename` to avoid partial writes.

---

## License

Same as the Terp Network workspace.