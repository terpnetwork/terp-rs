# Pointer: contract + e2e handoffs

## Product / specialist brief (original)

**`docs/bootstrap/hashmerchant-nft-oracle-minter-handoff.md`**

## Core contracts (landed in terp-rs; not the isolated cw-infuser worktree)

| Piece | Path under `crates/terp-rs/` |
|-------|------------------------------|
| Whitelist codec | `crates/hm-loyalty-whitelist` |
| Oracle price lib | `crates/hm-nft-oracle` |
| Reference minter | `contracts/infuser/hm-nft-minter` |
| Infuser contracts | `contracts/infuser/` |
| Generated clients | `api/ts/cw-infuser`, `api/ts/hm-nft-minter` |

Unit tests:

```bash
cd crates/terp-rs
cargo test -p hm-loyalty-whitelist -p hm-nft-oracle -p hm-nft-minter
```

This directory still owns offline recipes (`README.md`, `recipe.sh`).  
Lab / Path B / cw-orch / ictrs wiring is the **e2e** deliverable.
