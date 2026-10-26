# Headstash eligibility budget

`./eligibility/budget.sh` rebuilds `target/eligibility/` (tree.json, paths.jsonl, summary.json).

- BASE_ALLOCATION = 67,000,000 TERP = 67,000,000,000,000 uterp (TOTAL_SUPPLY 420,000,000 TERP). Matches docs/circuit/README.md total `66999999.999954 TERP & THIOL`.
- Per-community budget = floor(total * allocation_percentage) from headstash.yaml (exact decimals, normalised by their sum).
  The remainder goes 1 uterp at a time by largest fractional part, ties broken by name asc. For 67M the remainder is 0 and every budget is a whole TERP.
- chronic-token uses `--decimals chronic-token=18`. Leaf = floor(budget*balance/community_total), and floor dust is not distributed.
- n8 has a directory under headstash/communities but no entry in headstash.yaml, so it is left out of the staged dir and gets no allocation.

Governance context (morocco-1):
- Prop #32 "Fund Headsdtash Contract" (CommunityPoolSpend, PASSED, voting end 2023-12-18 20:43 UTC): 43,476,089.028199 TERP + same uthiol
  to terp1qeyjez6a9dwlghf9d6cy44fxmsajztw257586akk6xn6k88x0gusk40ehd. https://explorer.anode.team/main/terp/governance/32/
  Forum: https://commonwealth.im/terp-network/discussion/12372-proposal-airdrop-cycle-2-the-headstash
  The proposal amount is NOT 67M. It gives no per-community split.
- Prop #25 v3-pigeonfall upgrade "includes logic handling the headstash allocation" (PASSED 2023-10-21). It has no amount.

Pre-exclusion result at 67M (superseded): root 361e39a8dac874317ac046a51348bd1ae64080c771c6c2030b895231464cd419, depth 15, 17826 ids (15767 EVM / 2059 Sol),
leaves 66,999,999,994,137 + dust 5,863 = 67,000,000,000,000 uterp.

## Decision (2026-10-26)

- **67,000,000 TERP is the chosen production total.** Production root (after exclusions, below): `ffad92213f2c2001d73e56938064b0f2764535f220e16256abcd235f20338a57`. The earlier `361e39a8…cd419` and `0acb1bb0…b10b` roots are superseded.
- Prop #32 funded the headstash-patch contract with 43,476,089.028199 TERP and 43,476,089.028199 THIOL
  (`43476089028199` uterp/uthiol): https://explorer.anode.team/main/terp/governance/32/. That figure is cited for context and is not the budget used.
- **n8 is excluded by decision.** It has no headstash.yaml entry and gets no allocation.
- The builder also writes `allocations.jsonl`, one row per (community, identity) leaf contribution. The dao-dao-ui landing snapshot reads this file.

## Exclusions (decision 2026-10-26)

Burn, zero and contract addresses get **no leaf**, and their share is **left unallocated** (not redistributed).
`budget.sh` passes `--exclude eligibility/exclusions.csv` (override with `EXCLUDE=none`). The pro-rata split still uses the
original community total as denominator, so every other holder's amount is identical to the pre-exclusion build
(verified: 18 rows removed, 0 rows changed).

- `scan_addresses.py`: read-only `eth_getCode(latest)` on Ethereum mainnet for every EVM row and Solana
  `getMultipleAccounts` for every Solana row (cache: `target/address-scan.json`). No keys, no transactions.
- `make_exclusions.py`: writes `exclusions.csv` (excluded) and `review.csv` (flagged, NOT excluded, needs a decision).
- Excluded (18 rows, 16 identities): zero address (monster-buds, secret-sesh), `0x…dEaD` (buddah-bears, monster-buds),
  and 14 non-wallet contracts (incl. the chronic-token contract itself holding 20,109,025.22 and the crypto-canna-club
  NFT contract). No Solana incinerator/System addresses are in the snapshot.
- Review list (first pass): 7 Safe multisig proxies (EVM) and 12 program-owned Solana accounts (5 SPL token accounts, 7 others); excluded in the second decision below.
- EIP-7702: 1,058 rows (~950 addresses) have `0xef0100…` delegation code. They are EOAs with keys, so they are **not**
  excluded. (The older `*.unallocated.csv` "dead:contract" labels include 610 of them.)
- Off-curve Solana entries are already dropped by the builder (0 in this snapshot).
- Caveat: code is checked at the latest block, not the snapshot block.

First pass (superseded): root `0acb1bb0…b10b`, 17,810 identities, 18 rows excluded, 442,556,476,518 uterp unallocated.

### Second decision (2026-10-26, t49s298): review list excluded too

The 7 Safe multisigs (`safe-multisig`) and the 12 program-owned stoned-ape-club Solana accounts
(5 `spl-token-account` owned by `TokenkegQ…`, 7 `program-owned-solana`) are now in `exclusions.csv`; `review.csv`
keeps them with `status = resolved: excluded per user decision t49s298`. Same method: no leaf, share unallocated,
original community denominator. **Kept**: the 1,058 EIP-7702 delegated EOA rows and the 190 Solana addresses with no
on-chain account. Verified vs the first pass: 19 rows removed, 0 added, 0 changed (91,141,910,241 uterp moved to unallocated).

Result: root `ffad92213f2c2001d73e56938064b0f2764535f220e16256abcd235f20338a57`, depth 15, 17,791 identities
(15,744 EVM / 2,047 Sol). Leaves 66,466,301,607,378 + dust 5,863 + unallocated 533,698,386,759 = 67,000,000,000,000 uterp.

| community | excluded rows | unallocated uterp |
|---|---:|---:|
| buddah-bears | 6 | 23,960,382,535 |
| cannabuddies | 1 | 18,902,218,085 |
| chronic-token | 3 | 328,380,077,032 |
| crypto-canna-club | 7 | 13,249,737,800 |
| galacktic-gang | 2 | 11,207,130,152 |
| monster-buds | 4 | 7,266,775,924 |
| secret-sesh | 2 | 107,863,717,909 |
| stoned-ape-club | 12 | 22,868,347,322 |
| **total** | **37** | **533,698,386,759** |
