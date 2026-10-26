# Headstash 2026: fund the private TERP airdrop to 16 partner communities

## Summary
This proposal registers the `headstash-claim` oracle source with the hashmerchant module and
sends {{FUNDING_AMOUNT_UTERP}} uterp from the community pool to the new cw-headstash contract
`{{CONTRACT_ADDR}}`. Eligible holders from 16 partner communities (Ethereum/EVM and Solana)
register once with a wallet signature, then claim TERP to any Terp address with a
zero-knowledge proof that does not reveal which eligible wallet they own.

The eligibility tree leaves total {{ELIGIBILITY_TOTAL_UTERP}} uterp, slightly under the 67M TERP
budget. Addresses that cannot produce a wallet signature were excluded: burn and zero addresses,
EVM contracts, Safe multisigs and program-owned Solana accounts. Their share is left unallocated
in the community pool (67,000,000 TERP minus the leaf total, from the build output). The full list
is published in `eligibility/exclusions.csv`.

## What changed since Proposal #32 (Dec 2023)
- #32 sent 43,476,089.03 TERP to `terp1qeyjez...40ehd` (code 8, admin gov). That contract now
  holds 0 uterp, so nothing is recoverable and funding comes from the pool.
- Claims are private: registration links a wallet to a Poseidon key commitment; claiming proves
  membership in the registration tree (Halo2, K=11, vk id 2) and reveals only a nullifier.
- Payouts are batched and attested by validators through hashmerchant vote extensions.
- Wasm admin is the gov module. Unclaimed funds are swept after the cutoff.

## Privacy design
Registration message: `headstash-register-v1:{ctx}:{key_com}:{amount}` (EIP-191) or
`headstash-register-sol-v1:...` (ed25519). Claim proves knowledge of the secret behind a leaf
in the registration tree anchor, binds destination and amount, and publishes
`nullifier = Poseidon(tag, secret, ctx)`. `ctx` is derived from chain id, contract and label.
Known limit: the payout amount is public, so identities with a unique allocation amount are
linkable (see security summary).

## Eligibility method
Snapshots of 16 communities; per-project budgets split pro rata across holders with exact
decimal handling and floor rounding (dust stays unallocated). EVM leaves
`sha256("terp-hs-elig-leaf-v1"||addr20||amount_le8)`, Solana leaves add 0x02 and the 32-byte
pubkey. Eligibility root `{{ELIGIBILITY_ROOT}}`, depth {{ELIGIBILITY_DEPTH}},
{{ELIGIBILITY_IDENTITIES}} identities.

Per-community budgets (before exclusions; refresh from `summary.json` and
`exclusions.csv` after the final build, before posting):

| Community | Budget (TERP) | EVM wallets | Solana wallets |
|---|---:|---:|---:|
| buddah-bears | 7,605,710 | 2,453 | 0 |
| cannabuddies | 2,369,078 | 238 | 0 |
| carta-beta-gang | 1,924,158 | 157 | 0 |
| chronic-token | 5,440,174 | 1,255 | 0 |
| crypto-canna-club | 10,192,106 | 4,405 | 0 |
| cryptowizards | 1,138,865 | 55 | 0 |
| galacktic-gang | 7,781,951 | 2,568 | 0 |
| heady-pipe-society | 736,470 | 23 | 0 |
| hippie-life-krew | 2,839,906 | 342 | 0 |
| monster-buds | 8,934,501 | 3,385 | 0 |
| rebud | 3,714,231 | 585 | 0 |
| secret-sesh | 3,525,302 | 527 | 0 |
| shurlok | 882,161 | 33 | 0 |
| special-k | 1,041,525 | 46 | 0 |
| stoned-ape-club | 6,968,176 | 0 | 2,059 |
| wake-and-bake | 1,905,686 | 154 | 0 |
| **Total** | **67,000,000** | | |

## Security summary
Internal review 2026-10-26 (`tools/headstash/docs/security-review-2026-10-26.md`): no Critical
findings. Fixed: owner handoff (two-step, no renounce). Open and addressed operationally: owner
trust (owner = {{CONTRACT_OWNER}}, admin = gov), Merkle path delivery, unique-amount
linkability, oracle source registration (this proposal). Funds can only leave the contract via
attested batches or the post-cutoff sweep to `{{SWEEP_RECIPIENT}}`; gov can migrate.

## Links
- Contract: `{{CONTRACT_ADDR}}` (code id `{{CODE_ID}}`, wasm sha256 `{{WASM_SHA256}}`)
- Source: https://github.com/terpnetwork/terp-core (commit `{{SOURCE_COMMIT}}`)
- Forum: {{FORUM_URL}}
- Prior: Proposal #26, #31, #32
