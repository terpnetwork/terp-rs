# HashMerchant sidecar HTTP API

Node config: `hashmerchant.sidecar-url` or `HASHMERCHANT_SIDECAR_URL`.  
Empty URL → empty vote extensions (safe default).

**URL base for full hash-market:** use `http://HOST:9090` (root aliases) **or**  
`http://HOST:9090/ve` (nested historical paths). Both serve `/vote-extension` when `ve_enabled=true`.

## `GET /health`

**200**

```json
{ "status": "ok" }
```

Full hash-market may also include `mode`, `ve`, `trees`, etc.

Reference: `tests/tsh/hashmerchant/mock-sidecar.py`, `crates/terp-rs/tools/hash-market`

## Lab static provider (full-sidecar e2e)

```toml
ve_enabled = true
signing_key = "<32-byte hex secp256k1>"

[[providers]]
name = "ethereum-lab"
chain_uid = "ethereum-mainnet"
algo = "keccak256"
mode = "static"          # lab/e2e only — not production
address = "lab"
interval_secs = 2
static_root = "0123…def" # hex, no 0x
static_height = 42
```

## `GET /vote-extension`

### Legacy / lab (mock sidecar)

```json
{
  "chain_uid": "ethereum-mainnet",
  "algo": "keccak256",
  "root": "<hex state root without 0x>",
  "foreign_height": 123,
  "foreign_block_time": 1700000000
}
```

### Modular / IBCv2 (keeper `sidecarResponse`)

```json
{
  "runtime_id": "optional",
  "chain_uid": "ethereum-mainnet",
  "algo": "keccak256",
  "root": "<hex aggregated root>",
  "foreign_height": 123,
  "foreign_block_time": 1700000000,
  "attestations": [
    {
      "source_id": "skip-connect",
      "value": "<hex payload>",
      "height": 123,
      "timestamp": 1700000000,
      "custody_signature": "<hex ed25519>"
    }
  ]
}
```

Price-demo payloads often use custody scope `price/oracle` and JSON value bytes  
(see `x/hashmerchant/keeper/oracle_test.go`).

If `root` is empty and attestations are present, the keeper may aggregate.

## Support checklist

1. `/health` green before enabling production sidecar-url  
2. `/vote-extension` returns well-formed JSON every block interval  
3. `chain_uid` registered on-chain  
4. Failures → node emits empty extension (does not halt consensus)
