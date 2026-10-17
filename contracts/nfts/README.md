# nftfi

Ports of the Atlas raffle and NFT loan contracts (`AtlasDA0/app-contracts` at
`4b921920092004f4a79724e3c0db3f7993b64be7`) onto this workspace's CosmWasm v3
forks.

| Crate | Path |
| --- | --- |
| `nftfi-utils` | `packages/utils` |
| `nftfi-randomness` | `packages/randomness` |
| `nftfi-raffle` | `raffle` |
| `nftfi-loan` | `nft-loan` |

Draws do not call Nois or Drand. See [RANDOMNESS.md](RANDOMNESS.md). Collateral
is a cw721 token, including `cw721-svg`, pulled with `TransferNft` after
`OwnerOf` / approval checks.

Each crate has `LICENSE`, `NOTICE`, and `ORIGIN.md`.
