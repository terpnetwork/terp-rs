---
title: Headstash Server
description: Privacy note and circuit key server for the MetaMask snap integration
---

# Headstash Server

The headstash-server distributes encrypted notes and ZK circuit keys to the MetaMask snap (`snap-n-pull-js`) with secp256k1 authentication and optional PIR for metadata privacy.

## Quick start

```bash
cd tools/headstash-server

cargo build --release
cp config.example.toml config.toml
mkdir -p data/{headstash,notes,keys}

./target/release/headstash-server -c config.toml
```

## Configuration

```toml
# config.toml
bind = "0.0.0.0:8080"
data_dir = "./data"
```

For JWT auth, set the `JWT_SECRET` environment variable.

## API reference

### Public endpoints

| Endpoint | Method | Description |
|---|---|---|
| `/health` | GET | Liveness check |
| `/headstash/{id}` | GET | Fetch a headstash registration record |
| `/keys` | GET | List available circuit key IDs |

### Authenticated endpoints

All require blossom/BUD auth on the unified server (`blossom.auth.verify` — Nostr signed
events / admin key as configured), or the historical snap headers documented below
where a dedicated headstash-server is still used.

| Endpoint | Method | Description |
|---|---|---|
| `/headstash/{id}` | POST | Register a new headstash |
| `/headstash/{id}/root` | GET | Get the Merkle root for a headstash (legacy) |
| `/headstash/{id}/sync` | POST | Sync/merge state into a headstash record (legacy) |
| `/notes/{hs_id}/{addr}` | GET | Fetch encrypted note envelope |
| `/notes/{hs_id}/{addr}` | PUT / POST | **Store** encrypted note envelope (mint/bridge client) |
| `/notes/{hs_id}` | GET | List note keys for a headstash |
| `/notes/{hs_id}/pir` | POST | PIR-fetch a note (hides which address) |
| `/keys/{key_id}` | GET | Download a circuit key |
| `/keys/{key_id}/pir` | POST | PIR-fetch a key (hides which key) |

**Privacy:** private note bodies live only under `HeadstashStore` (`notes/{hs_id}/{addr}.json`).
They are **not** served on public dual-index `GET /content/{sha256}`. Optional `sha256` on
the envelope may point at a ciphertext-only BUD pin when `[distribution]` is on; primary
fetch remains auth-gated `/notes/...`.

## Cashu mesh (separate product)

Cashu e-cash uses a **different** store prefix and routes — do not conflate with Headstash notes:

- Store: `data/cashu/mints/…`, `data/cashu/wallets/…`
- HTTP: `/cashu/mints/*`, `/cashu/wallets/*`
- Schema: **canonical** mint registry (not “official”) — see
  [`docs/plans/cashu/CANONICAL-MINT-REGISTRY.md`](../../../../../docs/plans/cashu/CANONICAL-MINT-REGISTRY.md)
  and local [`cashu-mesh.md`](./cashu-mesh.md)

Wallet proof backups are always auth-gated and never dual-indexed as public `/content` SSOT.

## Authentication

### Server config (`[notes_auth]`)

| mode | Behavior |
|------|----------|
| `noop` (default) | Allow all — local/dev only |
| `bearer` | `Authorization: Bearer <token>` (`bearer_token` or env `NOTES_BEARER_TOKEN`) |
| `secp` | Snap headers; allowlist `allowed_pubkeys` (or `allow_any_secp = true`) |
| `bearer_or_secp` | Either path |

Client CLI (ops):

```bash
cargo run -p terp-seams --features cli --bin headstash-notes -- \
  recover --base http://127.0.0.1:9090 --hs-id season-1 \
  --addr cm.… --owner-key <64hex> --auth-bearer "$NOTES_BEARER_TOKEN"
```

### secp256k1 (snap client)

The snap derives a secp256k1 key via `snap_getBip32Entropy` (path `m/44'/133'/0'/0'/0'`) and signs requests:

```
msg       = "{unix_timestamp}\n{compressed_pubkey_hex}"
msg_hash  = SHA256(msg)
signature = secp256k1.sign(msg_hash, secret_key)
```

Headers sent:

```
X-Auth-Type: secp256k1
X-Pubkey:    <66-char compressed hex>
X-Timestamp: <unix seconds>
X-Signature: <128-char compact r||s hex>
```

**The server uses `PrehashVerifier::verify_prehash()`**, not `Verifier::verify()`. The client already hashed the message before signing. Using `verify()` would double-hash and reject every request.

### JWT (service-to-service)

```
Authorization: Bearer <HS256 JWT>
```

Secret is read from `JWT_SECRET` env var.

## PIR (Private Information Retrieval)

PIR lets the snap fetch a note or key without the server learning which item was requested.

**Protocol:**

1. Client calls `GET /notes/{hs_id}` to get the list of keys
2. Client builds a selector: `[0, 0, 1, 0, ...]` (1 at the target index)
3. Client POSTs `{ "selector": [0, 0, 1, 0, ...] }` to `/notes/{hs_id}/pir`
4. Server XORs all blobs where `selector[i] == 1` and returns the result
5. Since only one bit is set, the result is the target blob

The server processes every blob regardless of the selector, so it cannot distinguish which item was requested from timing or access patterns.

```
POST /notes/my-headstash/pir
Content-Type: application/json
X-Auth-Type: secp256k1
X-Pubkey: 02abc...
X-Timestamp: 1700000000
X-Signature: deadbeef...

{ "selector": [0, 0, 1, 0] }

→ { "result": "aabbccdd..." }   // hex-encoded XOR result
```

## File storage layout

```
data/
  headstash/
    {id}.json               # Registration records
  notes/
    {hs_id}/
      {addr}.json           # Encrypted notes (ciphertext + nonce + scheme)
  keys/
    {key_id}.bin            # Circuit proving/verifying keys
    {key_id}.blake3         # Integrity hash
```

All IDs are validated against `[a-zA-Z0-9_\-\.]{1,200}` to prevent path traversal. Writes use atomic temp-file-then-rename.

## Populating data

### Headstash records

```bash
# Create a headstash entry
curl -X POST http://localhost:8080/headstash/season-1 \
  -H "Content-Type: application/json" \
  -H "X-Auth-Type: secp256k1" \
  -H "X-Pubkey: 02abc..." \
  -H "X-Timestamp: $(date +%s)" \
  -H "X-Signature: ..." \
  -d '{"root": "aabb...", "total_accounts": 1000}'
```

### Notes

**Production write path (preferred):** mint/bridge client encrypts `SEAM-NOTE-OUT` cleartext
(opaque 382B) to the owner, then:

```bash
curl -X PUT "http://localhost:8080/notes/{hs_id}/{addr}" \
  -H "Content-Type: application/json" \
  -H "Authorization: …" \
  -d '{
    "ciphertext": "<hex|b64>",
    "nonce": "…",
    "scheme": "xchacha20poly1305",
    "cleartext_layout": "SEAM-NOTE-OUT-V0",
    "cleartext_len": 382
  }'
```

| Field | Convention |
|---|---|
| `hs_id` | cw-headstash contract bech32 **or** season slug registered in `headstash/{id}.json` |
| `addr` (claim) | claimant bech32 / `0x`+40 hex as in inclusion leaf |
| `addr` (bridge) | `cm.` + hex(note_commitment) or `pk.` + hex(pk_d) — `validate_id` charset only |

Caller: **client that holds plaintext** (rcm / note material). Indexer is metadata-only;
hash-market never re-derives secrets. Chain never sees plaintext.

**Genesis / bootstrap only:** place encrypted note JSON files directly in the data directory:

```bash
echo '{"ciphertext":"...","nonce":"...","scheme":"xchacha20poly1305"}' \
  > data/notes/season-1/0xabc123.json
```

**L0 smoke (no Docker):** `HeadstashStore::set_note` → `get_note` round-trip
(`cargo test -p hash-market set_note_get_note --features server`).

### Circuit keys

Copy proving/verifying key binaries:

```bash
cp circuit.pk data/keys/poseidon-2.bin
blake3 data/keys/poseidon-2.bin > data/keys/poseidon-2.blake3
```
