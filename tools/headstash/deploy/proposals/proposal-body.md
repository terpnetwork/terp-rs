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
linkable (see security summary). In the final tree 17,791 identities hold 1,826 distinct
amounts; 1,563 identities have an amount no one else has.

## Eligibility method
Snapshots of 16 communities; per-project budgets split pro rata across holders with exact
decimal handling and floor rounding (dust stays unallocated). EVM leaves
`sha256("terp-hs-elig-leaf-v1"||addr20||amount_le8)`, Solana leaves add 0x02 and the 32-byte
pubkey. Eligibility root `{{ELIGIBILITY_ROOT}}`, depth {{ELIGIBILITY_DEPTH}},
{{ELIGIBILITY_IDENTITIES}} identities.

Per-community figures from the final build (`summary.json`, `eligibility/exclusions.csv`):

| Community | Budget (TERP) | EVM wallets | Solana wallets | Excluded | Allocated (TERP) | Unallocated (TERP) |
|---|---:|---:|---:|---:|---:|---:|
| buddah-bears | 7,605,710 | 2,453 | 0 | 6 | 7,581,749.61 | 23,960.38 |
| cannabuddies | 2,369,078 | 238 | 0 | 1 | 2,350,175.78 | 18,902.21 |
| carta-beta-gang | 1,924,158 | 157 | 0 | 0 | 1,924,157.99 | 0.00 |
| chronic-token | 5,440,174 | 1,255 | 0 | 3 | 5,111,793.92 | 328,380.07 |
| crypto-canna-club | 10,192,106 | 4,405 | 0 | 7 | 10,178,856.26 | 13,249.73 |
| cryptowizards | 1,138,865 | 55 | 0 | 0 | 1,138,864.99 | 0.00 |
| galacktic-gang | 7,781,951 | 2,568 | 0 | 2 | 7,770,743.86 | 11,207.13 |
| heady-pipe-society | 736,470 | 23 | 0 | 0 | 736,470.00 | 0.00 |
| hippie-life-krew | 2,839,906 | 342 | 0 | 0 | 2,839,905.99 | 0.00 |
| monster-buds | 8,934,501 | 3,385 | 0 | 4 | 8,927,234.22 | 7,266.77 |
| rebud | 3,714,231 | 585 | 0 | 0 | 3,714,230.99 | 0.00 |
| secret-sesh | 3,525,302 | 527 | 0 | 2 | 3,417,438.28 | 107,863.71 |
| shurlok | 882,161 | 33 | 0 | 0 | 882,160.99 | 0.00 |
| special-k | 1,041,525 | 46 | 0 | 0 | 1,041,525.00 | 0.00 |
| stoned-ape-club | 6,968,176 | 0 | 2,059 | 12 | 6,945,307.65 | 22,868.34 |
| wake-and-bake | 1,905,686 | 154 | 0 | 0 | 1,905,685.99 | 0.00 |
| **Total** | **67,000,000** | **16,226** | **2,059** | **37** | **66,466,301.60** | **533,698.38** |

Allocated + unallocated + rounding dust (5,863 uterp) = 67,000,000 TERP exactly. Wallet counts are snapshot rows after exclusions; they merge into 17,791 identities (15,744 EVM, 2,047 Solana).

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

## Direct on-chain claim (2026-10 update)
Claims are verified by the contract itself (Halo2 vk_id 3 note circuit stored by this proposal,
`proof_instance_verify`, six public inputs: ctx, root, nullifier, dest_lo, dest_hi, amount) and pay
the claiming address at once. The proof root must be an accepted registration anchor. Each note
pays a fixed denomination to a fresh wallet; claim fees come from a one-transaction fee grant
scoped to this contract, so the fresh wallet never has to be funded.

This proposal therefore also stores the note circuit (`MsgStoreFullCircuit`, gov is the only allowed
uploader on mainnet). Circuit vk hash `{{NOTE_VK_HASH}}`. Fee grants are paid in THIOL by the relay
operator (estimated {{FEEGRANT_BUDGET_THIOL}} THIOL for all 98,841 notes at 0.05 uthiol/gas); no
TERP from this proposal is spent on fees.

Privacy limits: the anonymity set is the set of notes of the same denomination registered by the
anchor used; a small first epoch gives a small set; claiming soon after registering can be
correlated by time (the page adds random anchor and broadcast delays).

### Fee-grant funding
Claim and registration fees are paid by sidecar operators from their own accounts through one-transaction
fee grants scoped to this contract. This proposal sends each named operator its THIOL share:
{{OPERATOR_BUDGET_TABLE}} (total {{FEEGRANT_BUDGET_THIOL}} THIOL, source {{FEEGRANT_FUNDING_SOURCE}}). An
operator can only spend what it was sent; leftovers are returned after the cutoff.
