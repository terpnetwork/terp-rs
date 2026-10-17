# Origin

- Repo: https://github.com/AtlasDA0/app-contracts
- Commit: `4b921920092004f4a79724e3c0db3f7993b64be7`
- Paths: `contracts/nft-loan/` (`contract.rs`, `execute.rs`, `msg.rs`, `state.rs`, `query.rs`, `collection_offer.rs`, `lender_offer.rs`, `helpers.rs`, `examples/schema.rs`)

## What changed

- CosmWasm 1.4 / Stargaze messages replaced with the workspace CosmWasm v3 fork (`Uint256` coin amounts, `cw-storage-plus`, `cw721` from `../cw-nfts`).
- Collateral is cw721 only. Listing checks `OwnerOf` (and `Operator` if needed) and records an approval for this contract. `AcceptOffer` / `AcceptLoan` pulls the token with `TransferNft`. Repay returns it to the borrower. Default sends it to the lender. `cw721-svg` implements those messages.
- The listing stays non-custodial until the loan starts, as in Atlas.
- Protocol fee is `fee_rate` of the interest, taken on repay. Principal is paid to the borrower when the loan starts.
- A lender can `CancelOffer` after the borrower withdraws the listing, so the deposit is not stuck. Atlas only allowed cancel while the listing was still `Published`.
- Collection offers are not ported.
- Schema binary is `src/bin/schema.rs`. cw-orch interface is `NftfiLoan` behind the `interface` feature.
