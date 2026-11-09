# Headstash claim workflow (curated)

Locked from product session 2026-09-21. Product A remains **claim → clearnet `uterp`**, then **IBC + Penumbra shield** for private liquidity.

## Decisions

| Topic | Choice |
|-------|--------|
| Claim UI | **Dedicated page** in `websites/dao-dao-ui` (`apps/dapp/pages/headstash.tsx`). Keep existing **Headstash DAO module** (`packages/stateful/modules/modules/Headstash`) for DAO-home cards. |
| Notes serve | **hash-market** `HeadstashStore`. Default remains blossom/NIP-BUD. **PIR is a Cargo feature** on hash-market (not a second server, not NIP-BUD replacement). |
| Indexer PIR | **Merkle path + note blob only** (public leaf data). User already holds `esk` (ETH secp256k1). PIR hides *which* note they fetch. |
| Proof key | **MetaMask Snap** — prove with account `esk` **without** exporting the private key |
| Broadcast | Keplr / Terp account signs `ProcessHeadstash` |
| Post-claim v1 | **IBC `uterp` Terp → Penumbra, then shield in Prax** |
| Later | Stake, SVG/artwork mint (not this slice) |
| Not this slice | Live mainnet broadcast; private-dex; serving encrypted note payloads |

## Data already in tree

- Genesis notes: `artifacts/community-notes/notes.json` + `merkle_output.json` (`distro_hash_domain=poseidon-v1`, root in merkle JSON).
- Each note already has `elig_pk_compressed`, `epk_x`/`epk_y`, `nd`, `v`, `fdi`, `leaf` — the PIR record is this blob plus a Merkle path.
- Burn set: `headstash/communities/*.unallocated.csv` + `UNALLOCATED.md`.

## Slices

1. **hash-market `--features pir`** — ingest `notes.json` + tree into `HeadstashStore`; `/notes/{hs_id}/pir` returns `{leaf, path, fdi, nd, v, recp, epk_*}` without NIP-BUD. Direct GET-by-addr stays for lab. XOR PIR already in `hash-market/src/pir`; elevate to feature-gated ingest of genesis notes (vote-nullifier-pir is a pattern source, not the runtime).
2. **MetaMask Snap** — `snap.prove`: `esk` stays in snap; never paste `eth_privateKey`. See `circuit/TODO.md` snap-n-pull.
3. **DAO DAO dedicated `/headstash` page** — Snap + Keplr; PIR fetch; prove; `ProcessHeadstash`; mint `uterp`. Keep DAO **module** for embed on a DAO home.
4. **Penumbra hop** — IBC `uterp` Terp → Penumbra; shield in Prax. ict-rs dry-run before live channel.

## PIR threat model (indexer)

- Server **may** see the full public tree.
- Server **must not** learn which `elig_pk` / leaf index a claimant requested.
- Records are **not** secret; privacy is **access pattern** only.
- `esk` never leaves the Snap.

## Out of scope until claim works

Stake-to-validator, SVG mint, encrypted note hosting, live morocco-1 claim.
