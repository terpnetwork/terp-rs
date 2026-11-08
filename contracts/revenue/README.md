# Revenue contracts (terp-rs)

| Package | Crate | Role |
|---------|-------|------|
| `auction/` | terp-auction | NFT auction + royalties |
| `residual-registry/` | terp-residual-registry | Residual / royalty registry |
| **`private-dex/`** | **private-dex** | Private swap **SettleSwap** (was headstash `cw-private-dex`) |
| `bounty-escrow/` | terp-bounty-escrow | DEV/TEST ONLY bounty escrow + DAO DAO proposal module (was `contracts/bounties/terp-bounty-escrow`) |

Product docs: https://docs.terp.network/research/private-dex (canonical) — lab notes `../../docs/private-bridge/`.

## Codegen

gen-tools projects `marketplace` and `reserve-auction` (Cargo packages `revenue-marketplace`, `revenue-reserve-auction`; schema bins read from `[[bin]] name`). Run `just gen nftfi-schema <project>` or `just gen nftfi-schema-all`. Clients land in `api/ts/<cargo-package>/{types,client,message-composer}/`, plus `api/{python,go,rust,proto,openapi}/<cargo-package>/`.
