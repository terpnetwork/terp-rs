# Headstash claim frontend — outcome contracts

The UI source of truth is [HEADSTASH-UI-HANDOFF-2026-10-05.md](HEADSTASH-UI-HANDOFF-2026-10-05.md). Where this 2026-09-22 sheet disagrees with that handoff, follow the handoff. The snap prover and `ProcessHeadstash` sign path below are not the page to build.

Canonical plan for the claim page, the note fetch, the proof the page can actually produce, and the Penumbra hop after mint. Written 2026-09-22 from the locked product session (`CLAIM-WORKFLOW.md`, 2026-09-21) and the circuit as it verifies today.

This file is the acceptance layer. It does not script implementation steps. One canonical version. Branches return evidence and are merged or discarded. No live `morocco-1` broadcast. Do not overwrite community CSVs.

## What still holds from 2026-09-21

| Decision | Still true |
|----------|------------|
| Product | Claim mints clearnet `uterp`. Private liquidity is the next hop, not the claim. |
| Page | Dedicated DAO DAO page **and** the existing Headstash DAO module. Both exist: `websites/dao-dao-ui/apps/dapp/pages/headstash.tsx` renders `HeadstashClaimPage`. Module stays at `packages/stateful/modules/modules/Headstash`. |
| Notes | hash-market `HeadstashStore`. Blossom / NIP-BUD stays the default serve path. PIR is Cargo feature `pir`, not a second server. Route `/notes/{hs_id}/pir` is 404 unless that feature is on. |
| Broadcast | Keplr signs `ProcessHeadstash`. The page signs Amino and does not broadcast (`memo` says sign only). |
| After mint | IBC `uterp` Terp → Penumbra, then shield in Prax. The page only shows that sentence (`POST_CLAIM_CTA`). It does not build the transfer. |
| Not this | Stake, SVG mint, private DEX, encrypted note hosting, live channel open. |

## What the circuit changed, and the page has not caught

The page still treats a MetaMask snap as the prover (`invokeHeadstashSnap`, `SnapProveResult`). That is not the circuit that verifies.

| Fact | Consequence for the page |
|------|--------------------------|
| Ownership on the live path is in-circuit GLV: `epk = esk·G`. Native release, 10 threads, K=18: prove about 6.7s, proof 4704 bytes. | A snap worker cannot run this prove. The snap must not return the ETH private key. |
| `prove_ecdsa_verify` accepts a real `personal_sign` in MockProver. `synthesize` does not call it. | A wallet signature is not yet a claim. Do not tell the user the page is signature-only until `synthesize` checks it. |
| Nullifier is hiding. `rho` / `psi` are the two halves of a 32-byte `rseed`. `rcm` is their sum. `nk` is Poseidon of `esk` and that string. The leaf stores `rseed_com = Poseidon("terp-hs-rseed-v1", lo, hi)`, not the string. | The page must not derive `nf` from `(epk, nd, v, fdi)`. `claim_nullifier` is not the claim path. |
| Published `artifacts/community-notes/` has 51,807 notes, root `0x6bb155b7…`, domain `poseidon-v1`, and **no `rseed` field**. | That root will not verify against the current leaf. Regenerating writes a new tree under `artifacts/community-notes` and must not touch the CSVs. Do not point the page at the stale root and call it a claim. |
| Chain rejects a repeated nullifier. The circuit is what makes a second nullifier for the same leaf impossible. | The page sends the proof. It does not invent a second nullifier scheme. |

Blindness: an observer of the tree and the published nullifier cannot recompute that nullifier. The eligible key coordinates stay in the public leaf. Publishing a signature `(r, s)` next to the challenge would recover the key. The indexer may store `rseed` (it is sampled at note generation). Without `esk` the indexer still cannot compute `nf`. PIR's job is to hide **which** note was fetched, not to hide the tree.

## Harness this run inherits

| Category | Rule | Source |
|----------|------|--------|
| Product lock | Claim → clearnet `uterp`, then Penumbra. Not a private DEX. | `CLAIM-WORKFLOW.md` |
| No live broadcast | No `morocco-1` tx, no channel open. | same, and every circuit session |
| Key handling | Never return or paste `esk`. Snap docs: do not return the private key to a dapp. | MetaMask snap guidance; `assertNoSecretFields` already throws |
| Leaf format | Arity 7: tag, `epk_x`, `epk_y`, `nd`, `v`, `fdi`, `rseed_com`. | `docs/circuit/spec.md` |
| One claim path | Hiding nullifier is the default. Do not add a parallel public-leaf nullifier on the page. | circuit `synthesize` |
| Code rules | Fail loud. Parse at the boundary. One implementation per concept. No silent defaults for missing contract address, root, or `rseed`. No `git add -A`. | this session's operating rules |
| Proof budget | Host prove is the number we accepted (~6.7s, 4704 bytes, K=18). Do not spend the page on a second prover. | `artifacts/snap-proof-bench.json` |

## Team

Four focus panes. Styles are how they judge, not a script of edits. First return from each pane is a gap list with file paths and command output. No edits on that first return. Merge one pane's change at a time into this checkout. A second pane does not start writing while another's diff is unmerged.

| Pane | Name | Style | Territory |
|------|------|-------|-----------|
| Page | `hs-page` | Site-by-site. Read the component it is about to change. Browser-check the route. | `websites/dao-dao-ui/**/headstash*` and the Headstash module only where the page and the module share a helper |
| Notes | `hs-pir` | Fail loud. Feature gate stays off by default. | `crates/terp-rs/tools/hash-market` PIR feature and the note JSON shape it ingests |
| Proof | `hs-prove` | Anti-Postel. The circuit instance is the only witness contract. | The bytes the page must produce: instance columns, `rseed`, proof. No circuit redesign |
| Hop | `hs-hop` | Boundary owner. Dry-run only. | Post-claim copy and an ict-rs plan. No packet relay, no channel tx |

Orchestrator is the session that wrote this file. It merges. It does not let two panes edit the same file.

---

## 1. Page — a person can walk the claim without a false prover

```
OUTCOME: /headstash shows a claim that matches the circuit, signs ProcessHeadstash, and does not broadcast.
ACCEPTANCE:
- The route apps/dapp/pages/headstash.tsx still renders, and the DAO module still exists.
- The page does not ask for esk, a seed phrase, or a private key. assertNoSecretFields still rejects those field names.
- The prove control does not call a snap method that returns a Halo2 proof or a private key.
- The sign control still builds ExecuteMsg::ProcessHeadstash and stops at Keplr signAmino. No broadcast client call is added.
- POST_CLAIM_CTA still says the Penumbra shield is not sent from this page.
- A missing contract address, a missing root, or a note without rseed fails with that fact in the message. No default root, no default contract.
- The stale community-notes root is not labeled as the live claim root.
TERRITORY: dao-dao-ui Headstash page, its flow module, and its unit test. Do not edit the circuit. Do not edit hash-market.
BUDGET: One page, one flow module. No new wallet library. No second claim UI.
EVIDENCE: The test file's new assertions, and a click-through note of what the page shows at each step (or the command that rendered it). "It works" without that is not evidence.
```

Negative: do not remove the DAO module. Do not add a broadcast button. Do not add a snap prover back.

## 2. Notes — the fetched record opens the leaf the circuit checks

```
OUTCOME: A PIR fetch returns one public leaf opening plus the claimant's rseed, and the leaf hash includes rseed_com.
ACCEPTANCE:
- cargo build -p hash-market without --features pir still builds, and /notes/{hs_id}/pir is still 404 on that build.
- With --features pir, the ingest reads rseed from the note file. A note file missing rseed is rejected by name, not skipped.
- The served record contains the Merkle path, the public leaf fields, rseed_com, and rseed. It does not contain esk.
- GET-by-address remains the lab path. The claim page is specified to use POST /notes/{hs_id}/pir.
- NIP-BUD / blossom routes are still the default and are not deleted.
- The current artifacts/community-notes/notes.json is not treated as a successful ingest: it has no rseed. Evidence of ingest uses a fixture with one note that has rseed, not the 51,807-row file.
TERRITORY: hash-market feature pir, its docs/headstash.md, and a small fixture. Do not regenerate the community tree. Do not overwrite CSVs. Do not edit the DAO page in this contract.
BUDGET: Feature-gated. No new server binary. No second PIR scheme.
EVIDENCE: A cargo test or a recorded request/response pair showing the 404 without the feature and a record with rseed with the feature. The fixture's leaf preimage lists rseed_com.
```

Negative: do not put `rseed` in the public Merkle leaf. Do not replace Nostr.

## 3. Proof — the page's bytes are the circuit's instance

```
OUTCOME: A written witness contract the page can satisfy, matching synthesize, with no second proof system.
ACCEPTANCE:
- Lists the public instance columns in order: anchor, nd, v, recp, nf, cmx. States that to_bytes is 168 bytes and stores raw recipient bytes, while the Halo2 column recp is recp_to_fp.
- States that nf and cmx are circuit outputs, not inputs the page hashes from the leaf.
- States that the prover input includes esk and the 32-byte rseed, and that esk stays in the proving process, not in the page state and not in a snap RPC result.
- States that the live prover is the host K=18 Groth/IPA path (4704-byte proof), and that personal_sign is not accepted on this path yet.
- Names the negative tests that already exist: h1_valid_claim, h_rerolled_rseed_fails, h_free_nk_fails.
TERRITORY: This section of the spec, plus a fixture or type the page can import later. Do not change synthesize. Do not add a wasm prover.
BUDGET: Read the circuit and write the contract. No bench unless the proof bytes or K change.
EVIDENCE: Column list cited to circuit.rs instance indices, and a sentence each for what the page holds versus what the host prover holds.
```

Negative: do not document the snap as the prover. Do not document `claim_nullifier(epk, nd, v, fdi)` as the spent nullifier.

## 4. Hop — after the mint, one sentence and a dry-run plan

```
OUTCOME: The page tells the truth about the next hop, and a dry-run plan exists for IBC uterp to Penumbra then a Prax shield. Nothing is sent.
ACCEPTANCE:
- The page string still matches the decision: IBC uterp from Terp to Penumbra, then shield in Prax, not sent from this page.
- The plan names the existing ict-rs dry-run path and states that no live channel is created.
- No packet, no channel-open tx, no mainnet RPC write appears in the diff.
TERRITORY: The CTA string if it drifts, and a short section in this file or the existing IBC dry-run doc. Do not implement a bridge UI.
BUDGET: Documentation and a cited dry-run command only.
EVIDENCE: The command that would be run locally, and the statement that it was not run against a live chain. If it was not run at all, say so.
```

Negative: do not start stake, SVG mint, or a private DEX from this hop.

---

## Operating loop

- Pre-run: this file is the acceptance list. A pane that needs a new condition stops and asks. It does not invent one.
- First return: gaps only, with paths. No edits.
- Receipts: command and output, or a failing test name. No "it works".
- Merge: one pane at a time. The orchestrator reads the diff against the contract above.
- Capture: a correction that would otherwise be repeated goes in the harness table, not only in chat.
- Stop conditions: ambiguous product choice, a prompt that contradicts this file, a pane about to touch a CSV or broadcast.

## First return (2026-09-22, read-only)

The four panes confirmed the contracts against the tree. No files were edited.

| Pane | Already true | Gap |
|------|----------------|-----|
| `hs-page` | Route and DAO module both exist. No secret field. Keplr `signAmino` only. CTA rendered, Penumbra and Prax buttons disabled. Missing contract and missing PIR root already throw. | `prove` calls `invokeHeadstashSnap(..., 'prove')`. `PublicLeafBlob` has no `rseed`. `DEFAULT_PIR_HS_ID` is `community-notes`, and that root is sent as the anchor with no stale warning. |
| `hs-pir` | Default build has no `pir`. `POST /notes/{hs_id}/pir` is 404 (`note_persist.rs`). Blossom stays on the default features. | `GenesisNote` has no `rseed` and is not rejected. The served record has no `rseed` and no `rseed_com`. Ingest of `crates/headstash/artifacts/community-notes/notes.json` (51,807 notes, no `rseed`) is treated as success. |
| `hs-prove` | Instance columns 0–5 are anchor, `nd`, `v`, `recp.to_fp()`, `nf`, `cmx`. Wire is 168 bytes with raw recipient bytes. Live path is `prove_key_pairing`. `prove_ecdsa_verify` is test-only. Nullifier is the hiding one. | This is the contract the page does not satisfy yet. `esk` stays in the host process. |
| `hs-hop` | CTA text matches. Nothing on the page sends the transfer. | The dry-run that exists is Terp→Terp: `ICT_MOCK=1 make e2e-ibc` (`crates/ict-rs/examples/ibc_transfer.rs`, `uterp`, `channel-0`). `crates/ict-rs/ict-rs/src/chain/penumbra.rs` is a stub. No Prax step. Not run. |

## Decision the orchestrator still owes the team

The host prover needs `esk` in its process. The page must not hold it. The snap must not return it. Those three sentences are not yet one design. Choices that are **not** authorized by this file: exporting the key to the page, proving inside the snap, or pretending `personal_sign` verifies today. The orchestrator picks the host-prover boundary before `hs-prove` and `hs-page` are allowed to write code.
