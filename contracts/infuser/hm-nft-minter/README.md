# hm-nft-minter

Reference CosmWasm minter on the **HashMerchant perch**:

- **Loyalty whitelist** via `hm-loyalty/v1` (`hm-loyalty-whitelist`)
- **Oracle-priced** native payment bounds (`hm-nft-oracle`)
- **CW721 mint** into a linked collection (e.g. `cw721-svg`)

Hard rule: oracle/sudo never mints supply or increases `max_tokens`.

## Unit tests

```bash
cargo test -p hm-loyalty-whitelist -p hm-nft-oracle -p hm-nft-minter
```

## Docs

- Architecture: `docs/hm-nft-oracle-minter-ARCHITECTURE.md`
- E2E handoff: `docs/hm-nft-oracle-minter-E2E-HANDOFF.md`
- Product brief: terp-core `docs/bootstrap/hashmerchant-nft-oracle-minter-handoff.md`

## Schema

```bash
cargo run -p hm-nft-minter --bin schema
```
