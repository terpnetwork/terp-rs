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

- **67,000,000 TERP is the chosen production total.** Production root (after exclusions, below): `0acb1bb0e0caf6860046c163e4a5d946957ab29526db00adf9e8f6a8d8efb10b`. The earlier `361e39a8…cd419` root is superseded.
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
- Review, still eligible: 7 Safe multisig proxies (EVM) and 12 program-owned Solana accounts (5 SPL token accounts, 7 others).
- EIP-7702: 1,058 rows (~950 addresses) have `0xef0100…` delegation code. They are EOAs with keys, so they are **not**
  excluded. (The older `*.unallocated.csv` "dead:contract" labels include 610 of them.)
- Off-curve Solana entries are already dropped by the builder (0 in this snapshot).
- Caveat: code is checked at the latest block, not the snapshot block.

Result: root `0acb1bb0e0caf6860046c163e4a5d946957ab29526db00adf9e8f6a8d8efb10b`, depth 15, 17,810 identities
(15,751 EVM / 2,059 Sol). Leaves 66,557,443,517,619 + dust 5,863 + unallocated 442,556,476,518 = 67,000,000,000,000 uterp.

| community | excluded rows | unallocated uterp |
|---|---:|---:|
| buddah-bears | 4 | 6,845,823,580 |
| chronic-token | 3 | 328,380,077,032 |
| crypto-canna-club | 6 | 12,230,527,200 |
| galacktic-gang | 1 | 7,004,456,345 |
| monster-buds | 3 | 6,540,098,332 |
| secret-sesh | 1 | 81,555,494,029 |
| **total** | **18** | **442,556,476,518** |
