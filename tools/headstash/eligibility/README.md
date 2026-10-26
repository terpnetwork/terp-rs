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

Result at 67M: root 361e39a8dac874317ac046a51348bd1ae64080c771c6c2030b895231464cd419, depth 15, 17826 ids (15767 EVM / 2059 Sol),
leaves 66,999,999,994,137 + dust 5,863 = 67,000,000,000,000 uterp.
