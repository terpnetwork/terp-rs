# marketplace-mirror

Nostr-mirrored marketplace inventory (NIP-15) with **buffer-escrow** purchases and **hashmerchant** root sudo.

| Concern | Behavior |
|---------|----------|
| Content plane | Prefer bare **sha256** `content_cid` from `bind_offchain_body` / `bind_listing_body` (BUD primary). TreeStore is sole BlobStore — see `tools/hash-market/docs/content-distribution.md` |
| Inventory | **Contract-local** SKU stock map (demo). `available = stock - reserved`. Future v2: proof-gated restock from hashmerchant roots — **not** wired today |
| Purchase | Locks funds + reserves qty; status `Reserved` (mini-escrow) |
| Buffer | Default **`min_buffer_blocks = 7`** (allowed **5–10**). `settle_after = reserve_height + min_buffer_blocks` |
| Settle | Only at/after `settle_after`; pays seller (optional protocol fee to admin); decrements stock |
| Cancel | **Buyer or admin only** while `Reserved` (seller cannot grief-cancel). After settle/cancel: fails |
| Hashmerchant sudo | Stores last foreign root by `(chain_uid, algo)`. **Does not mint supply** |
| Nostr | **Egress** (chain→NIP-15 notes) + **ingress-intent** parse. Auto-execute is **off** unless admin sets `auto_execute_ingress` |

## Messages

### Instantiate

```json
{
  "admin": "terp1…",
  "min_buffer_blocks": 7,
  "accepted_denom": "uterp",
  "protocol_fee_bps": null
}
```

### Execute

| Msg | Who | Notes |
|-----|-----|-------|
| `ListProduct { sku, stock, unit_price, content_cid?, d_tag? }` | any (becomes seller) | `content_cid` = bare sha256 from content plane; optional NIP-15 d-tag |
| `UpdateStock { sku, stock }` | seller/admin | Local stock only; cannot go below reserved |
| `Purchase { sku, qty }` + funds | buyer | Funds ≥ `unit_price * qty` in `accepted_denom` |
| `Settle { order_id }` | anyone | Height ≥ `settle_after` |
| `Cancel { order_id }` | **buyer/admin only** | Only while `Reserved`; seller unauthorized |
| `SetAutoExecuteIngress { enabled }` | **admin only** | Governance flag for relayer auto-`Purchase`. Default **false**. Contract does not pull Nostr. |

### Query

`Config`, `Product{sku}`, `Order{id}`, `ListOrders`, `ReservedQty{sku}`, `GetRoot{chain_uid,algo}`

### Sudo (hashmerchant)

```json
{
  "hash_merchant": {
    "chain_uid": "nostr-marketplace",
    "algo": "marketplace-inv-v1",
    "height": 42,
    "root": "<base64>",
    "attestation_count": 3,
    "block_time": 1700000000
  }
}
```

Algo examples: `sha256`, `marketplace-inv-v1`. Stored for **future v2** proof-gated restock — **not** used to invent or update SKU stock today (local `UpdateStock` remains authoritative).

## Register with hashmerchant

Same pattern as [loyalty-verifier](../loyalty-verifier/):

1. Store + instantiate this wasm on Terp.
2. Register:

```sh
terpd tx hashmerchant register-contract \
  <CONTRACT_ADDR> \
  <CHAIN_UID> \
  <SUBSTORES_JSON> \
  <ESCROW_AMOUNT>uterp \
  --from <key> -y
```

3. Maintain escrow so sudo callbacks stay active.
4. Spec: `x/hashmerchant/spec/06_sudo_interface.md`, `04_messages.md`.

## Demo flow

```text
1. instantiate (min_buffer_blocks=7, accepted_denom=uterp)
2. ListProduct sku=tee stock=10 unit_price=100 content_cid=<bare-sha256>
3. Purchase qty=3 + 300uterp  → order#1 Reserved, reserved=3, available=7
4. Settle before +7 blocks    → error BufferNotElapsed
5. wait ≥7 blocks
6. Settle order#1             → seller paid, stock=7, reserved=0, Settled
7. (optional) Cancel in buffer → buyer refunded, reservation released
8. (optional) hashmerchant sudo → GetRoot shows last inventory root
```

## Build / test

```sh
# Unit tests (buffer, reserve, oversell, sudo)
cargo test

# REQUIRED for wasmd / Docker e2e (optimizer + bulk-memory lower)
just wasm

# Faster host path (still runs wasm-opt bulk-memory lower)
just wasm-dev
```

**wasmd 0.61 rejects bulk-memory** (`memory.copy` / `memory.fill`).

| Build | Command | Notes |
|-------|---------|--------|
| ✅ wasmd-safe | `just wasm` | Docker rustc 1.86 + host `wasm-opt` lower |
| ✅ wasmd-safe | `just wasm-dev` | Host rustc + same `wasm-opt` lower |
| ❌ not storeable | plain `cargo build --target wasm32…` | bulk-memory → store fails |

Needs: Docker (`terpnetwork/optimizer-arm64:0.17.0` or cosmwasm) + host **binaryen ≥120** (`brew install binaryen`).

Artifact: `artifacts/marketplace_mirror.wasm`

## Nostr bridge (thin helpers — not a full service)

| Direction | Module | Role |
|-----------|--------|------|
| Chain → relay | `tools/hash-market/src/client/nostr/marketplace_egress.rs` | NIP-15 product / purchase-confirmation notes after chain events |
| Relay → intent | `tools/hash-market/src/client/nostr/marketplace_ingress.rs` | Parse order-shaped Nostr JSON → `PurchaseIntent` for demo/relayer |

**Ingress auto-execute is off by default** (`auto_execute_ingress: false`). Relayer / operator must submit funds + `ExecuteMsg::Purchase` unless admin flips the flag **and** the hash-market ingress policy is `On`. The contract never executes from a Nostr event by itself.

List product body → content plane:

```text
bind_listing_body(store, product_json)  // alias of bind_offchain_body
  → chain_cid (bare sha256)
  → ListProduct { content_cid: Some(chain_cid), … }
```
