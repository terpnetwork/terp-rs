# Pointer: contract + e2e handoffs

## Product / specialist brief (original)

**`docs/bootstrap/hashmerchant-nft-oracle-minter-handoff.md`**

## Core contracts (landed in cw-infuser)

| Piece | Path under `crates/cw-infuser/` |
|-------|--------------------------------|
| Whitelist codec | `packages/hm-loyalty-whitelist` |
| Oracle price lib | `packages/hm-nft-oracle` |
| Reference minter | `contracts/cw721/hm-nft-minter` |
| Architecture | `docs/hm-nft-oracle-minter-ARCHITECTURE.md` |
| **E2E specialist handoff** | `docs/hm-nft-oracle-minter-E2E-HANDOFF.md` |

Unit tests:

```bash
cd crates/cw-infuser
cargo test -p hm-loyalty-whitelist -p hm-nft-oracle -p hm-nft-minter
```

This directory still owns offline recipes (`README.md`, `recipe.sh`).  
Lab / Path B / cw-orch / ictrs wiring is the **e2e** deliverable.
