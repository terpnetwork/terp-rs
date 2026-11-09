# cw-vote-anchor

P0 **session root registry** for vote commitment tree anchors on Terp.

**Design:** [`docs/research/VOTE-TREE-HOST-TERP.md`](../../../../docs/research/VOTE-TREE-HOST-TERP.md)

## Hybrid default

| Layer | Role |
|-------|------|
| Off-chain | `vote-commitment-tree` depth-24 Poseidon; paths for prove |
| On-chain (this) | `GetAnchorAtHeight(session, height) → root[32]` |

Cast ZKP public input `vote_comm_tree_root` must equal stored anchor at `anchor_height` (fail-closed).

## Trust (P0)

**M1** — authorized admin posts checkpoint roots after off-chain `Checkpoint(height)`.

## Non-goals

- CosmWasm depth-24 append
- Path generation on-chain
- Nullifier / Path A verify

```bash
cargo test
```
