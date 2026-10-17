# Session Handoff: Headstash claim UI

**Date:** 2026-10-05
**Branch context:** `crates/headstash` `zk-mvp`; page lives in `websites/dao-dao-ui` on `release/v6.3.0-v6.4.0-wasmfold`
**Session goal:** Give the headstash page one source of truth for the locked claim design and the infra it calls.
**Restore via:** QMD lex `headstash ui handoff epoch ApplyAcceptedBatch` · vec `What does the headstash claim page call, and what must it not build?`
**Skill pattern:** `session-roundoff`

This file is the source of truth for the headstash UI. `FE-CLAIM-SPEC.md` (2026-09-22) is the older outcome sheet. Where they disagree, follow this file. Do not broadcast to `morocco-1`. Do not commit `notes.json`, verifying keys, or wasm.

---

## 1. Do not re-litigate (locked)

| Claim | Status |
|-------|--------|
| The claim pays public clearnet `uterp`. Destination and amount stay public. | **Locked** |
| The eligible key and the `personal_sign` `(r, s)` stay witnesses. The page never asks for `esk`, a seed, or a private key. | **Locked** |
| One note is paid once, on one path. A public-path note cannot also be paid by the Halo2 batch. | **Locked** |
| Halo2 is verified off-chain by a validator sidecar. The contract does not verify the proof. | **Locked** |
| Hashmerchant is the quorum. The IBC attestations light client is not mounted and is not this design. | **Locked** |
| Two algos, one `chain_uid`: `headstash-pending` is availability, `headstash-accepted` is the only pay signal. | **Locked** |
| The stored root is the Hashmerchant aggregate of one fixed attestation, not the bare batch hash. | **Locked** |
| Bandersnatch is specified and is not a working page control. | **Locked** |
| Flock stays the older clearnet claim. No in-circuit secp256k1 on Flock. No Halo2-KZG port. | **Rejected** |
| A MetaMask snap is the prover. | **Rejected** |
| The page broadcasts `ProcessHeadstash`, opens an Ethereum RPC, or builds the Penumbra IBC transfer. | **Rejected** |

### Rejected framing

- Hiding destination and amount in this cut. That is a later circuit change. The live instance still publishes them.
- Treating a pending root, an oracle hint, or a single sidecar as authorization to mint.
- Lowering the CometBFT commit quorum so a rejected vote extension can still make a block. Empty extensions are safe. Rejected extensions stall the chain.
- A second Ethereum client beside the existing svg-mint watcher.

### Chosen framing

```text
Holder picks one path before the note is reserved.
Public 1/1: Ethereum lock of a fixed denomination index plus a Pedersen
commitment of the Terp destination. The blind stays off Ethereum.
The watcher already used for Zcash minting observes the log.
The holder later opens the commitment with ApplyPublicBatch.
Fully private: the holder proves Halo2 locally. Validator sidecars
verify and Hashmerchant stores an accepted aggregate.
The holder sends ApplyAcceptedBatch only when EpochRoot matches.
The page shows the Penumbra sentence after pay and does not send it.
```

---

## 2. Stable library / integration surface

**Path:** `crates/headstash/contracts/cw-headstash`

The page talks to the contract. It does not talk to a sidecar. There is no user-facing `POST /claim`. The Docker sidecar injects a precomputed batch hash and does not verify a Halo2 proof yet, so a proof produced on this page cannot land `headstash-accepted` until that lane exists. Until then the private apply stays disabled.

| Symbol / API | Role |
|--------------|------|
| `QueryMsg::EpochRoot { algo }` | `{ algo, root, height }`. `root` is 32 bytes, JSON base64. Missing root is `"epoch root not found"`. |
| `ExecuteMsg::ApplyAcceptedBatch { allocation_root, root_id, rows }` | Pays when `attested_root(accepted_root(...))` equals the stored `headstash-accepted` root. Does not verify the proof. |
| `ExecuteMsg::ApplyPublicBatch { rows }` | Pays one opened Pedersen row per Ethereum log. Amount comes from the operator denom table, not from the row. |
| `ExecuteMsg::SetDenomTable` | **Operator.** Not a holder button. |
| `ExecuteMsg::AssignPublicNotes` | **Operator.** Reserves nullifiers for the public path. |
| `epoch::accepted_root` | Batch hash the aggregate commits to. |
| `epoch::attested_root` | The bytes `EpochRoot` must equal before apply. |
| `epoch::pedersen_commit` | Commitment the page computes before the Ethereum lock. |
| `claim_auth::claim_signing_message` | ASCII bytes MetaMask `personal_sign`s. |
| `claim_auth::challenge_scalar_be` | Public `e` inside the proof. |

**Holder messages.** Shapes are `EpochPayout` and `PublicOpen` in `contracts/cw-headstash/src/epoch.rs`. JSON is the CosmWasm snake_case execute body.

```json
{
  "apply_accepted_batch": {
    "allocation_root": "<base64 of 32 bytes>",
    "root_id": 0,
    "rows": [{
      "nullifier": "<base64 of 32 bytes>",
      "denom": "<base64 of 32 bytes>",
      "destination": "terp1...",
      "amount": "15"
    }]
  }
}
```

```json
{
  "apply_public_batch": {
    "rows": [{
      "index": 1,
      "destination": "terp1...",
      "blind": "<base64 of 32-byte canonical Pallas scalar, non-zero>",
      "commitment": "<base64 of 32-byte compressed commitment>",
      "tx_hash": "0xabc",
      "log_index": "2",
      "nullifier": "<base64 of 32 bytes>"
    }]
  }
}
```

```json
{ "epoch_root": { "algo": "headstash-accepted" } }
```

`headstash-pending` uses the same query. It is not a pay signal.

**Invariants:**

1. `chain_uid` is `headstash-claim`. Pay algo is `headstash-accepted`. Pending algo is `headstash-pending`. Source id and custody scope are both `headstash-batch`. Attestation height is `1`. Timestamp `1` is signed into custody and is not part of the aggregate.
2. Accepted preimage is domain `terp-headstash-accepted-v1`, then the 32-byte allocation root, then `u32` LE row count, then rows sorted by nullifier. Each row is 32-byte nullifier, 32-byte denom, `u16` LE destination length, destination UTF-8, `u64` LE amount. SHA-256 of that preimage is the batch hash.
3. The stored root is SHA-256 of the ASCII source id `headstash-batch`, then the batch hash, then height `1` as `u64` LE. `ApplyAcceptedBatch` recomputes both and rejects with `batch does not match the accepted root`.
4. Pedersen domain is `terp-headstash-pedersen-v1`. Generators are hash-to-curve of `G` and `H`. The destination scalar is `from_uniform_bytes` of two SHA-256s labeled `dest` and `wide`. The blind is a canonical non-zero `Scalar::from_repr`. The commitment is the 32-byte compressed affine point. The blind never goes to Ethereum.
5. Signing string is ASCII `headstash-claim-v1:` plus 64 lowercase hex characters of `keccak256(instance_bytes[..168])`. `e` is `challenge_scalar_be` of that 32-byte keccak. MetaMask applies the EIP-191 prefix. The page does not send `(r, s)` to the contract. `ApplyAcceptedBatch` does not check `e`. The old `ProcessHeadstash` path does, in `headstash.rs`, and the page must stop building that message.
6. Public amount is `denom_table[index]`. The exercise table was `[10, 20]`, so index `1` paid `20uterp`. That table is not a protocol constant and there is no query for it. The page must not hardcode `20`.
7. Nullifier storage key is `{root_id}:{nf_hex}`. The public path spends under `root_id` 0. A second pay returns `nullifier already exists`. The same Ethereum `(tx_hash, log_index)` returns `ethereum log already paid`.
8. Empty vote extensions are accepted and do not stall the chain. A rejected vote extension (bad custody, unregistered chain, advertised root that is not the aggregate) stalls CometBFT. The page must not produce those. Two of three equal stakes confirm only when genesis `quorum_fraction` is `0.600000000000000000`. The module default `0.667` truncates to just above two shares and never confirms two of three. The page does not change either quorum.

**Design SoT:** this file.
**Example:** `crates/ict-rs/examples/epoch_claim.rs` (local Docker, three validators, three sidecars). Contract unit of the same messages: `contracts/cw-headstash/src/epoch.rs` tests and `epoch_orch.rs`.

### Source the page is allowed to read

| What | Path |
|------|------|
| Execute and query enums | `crates/headstash/contracts/cw-headstash/src/msg.rs` |
| Batch hash, aggregate, Pedersen, apply, error strings | `crates/headstash/contracts/cw-headstash/src/epoch.rs` |
| Sudo saves a root per algo and mints nothing | `epoch::sudo` in that file |
| Signing string and challenge | `crates/headstash/circuits/halo2/src/claim_auth.rs` (`CLAIM_PREFIX`, `claim_signing_message`, `challenge_scalar_be`) |
| Why `e` is compared to the first 168 instance bytes | `process_headstash` in `contracts/cw-headstash/src/headstash.rs` |
| Instance wire, 200 bytes, K=19 | `crates/headstash/circuits/halo2/src/circuit.rs` |
| Host prove and invalid-proof times | `crates/headstash/circuits/halo2/src/snap_bench.rs` |
| Local chain exercise | `crates/ict-rs/examples/epoch_claim.rs` |
| Hashmerchant fetch, empty extension, aggregate replacement | `x/hashmerchant/keeper/vote_ext.go` (`fetchSidecar`), `x/hashmerchant/keeper/oracle.go` (`aggregateAttestationRoot`) |
| Page route | `websites/dao-dao-ui/apps/dapp/pages/headstash.tsx` |
| Page and flow | `websites/dao-dao-ui/packages/stateful/components/pages/HeadstashClaim.tsx`, `headstashClaimFlow.ts` |
| DAO module, keep it | `websites/dao-dao-ui/packages/stateful/modules/modules/Headstash/` |

### What the page renders

Route stays `apps/dapp/pages/headstash.tsx`, which renders `HeadstashClaimPage`. The DAO module stays. The holder types the contract address. An empty address stops with that fact. There is no default contract and no hardcoded community root. `DEFAULT_PIR_HS_ID = 'community-notes'` must not be labeled as the live claim root.

After a pay, the page shows this sentence and does not build the transfer:

```text
IBC uterp from Terp to Penumbra, then shield in Prax. Not sent from this page.
```

**Public 1/1, holder steps**

1. The holder names the Terp destination. The page samples a non-zero canonical Pallas blind and computes `pedersen_commit`. No contract message. The blind is not sent to Ethereum.
2. MetaMask submits the existing Ethereum lock: denomination index plus that commitment. The svg-mint watcher observes the log. The page does not open an Ethereum RPC. No CosmWasm message.
3. The holder sends `ApplyPublicBatch` with the opening, the watcher `tx_hash` and `log_index`, and the reserved nullifier. This step has no watcher return on the page yet, so it stays disabled until that identity is supplied. Operator `SetDenomTable` and `AssignPublicNotes` must already have succeeded. They are not on this page.

**Fully private, holder steps**

1. MetaMask `personal_sign`s `claim_signing_message(keccak256(instance_bytes[..168]))`. `(r, s)` stay witnesses. No contract message.
2. The holder machine runs the host prove. The page shows the wait. It does not call a snap. No contract message. Quiet native numbers, 10 threads, 2026-10-05: keygen 65.8s once per process, prove 16.1s, verify 0.30s, proof 4800 bytes, K=19. A flipped proof fails in about 1ms. A well-formed proof under the wrong instance costs about 0.22s.
3. Submit that proof to a sidecar. **Blocked.** No `POST /claim`. The exercise sidecar does not verify Halo2.
4. Query `EpochRoot` for `headstash-accepted`. A missing root stops with that fact.
5. Send `ApplyAcceptedBatch` only when that root equals `attested_root` of this batch. **Blocked** until step 3 can write the root. The page does not send an apply that will miss.

**Errors the page shows as returned.** Do not invent a second code.

| Text | When |
|------|------|
| `epoch root not found` | No sudo root for that algo |
| `no accepted root` | Apply before any accepted sudo |
| `batch does not match the accepted root` | Rows are not the attested batch |
| `nullifier already exists` | That note was already paid |
| `note is on the public path` | Halo2 apply of a reserved note |
| `duplicate nullifier in batch` | Same nullifier twice in one apply |
| `public batch is empty` | `ApplyPublicBatch` with no rows |
| `denomination table is not set` | Operator has not called `SetDenomTable` |
| `denomination index out of range` | Index past the table |
| `pedersen opening does not match` | Blind, destination, and commitment disagree |
| `blind must be non-zero` / `blind is not a canonical scalar` | Bad blind |
| `missing ethereum log` | Empty `tx_hash` |
| `ethereum log already paid` | Same `(tx_hash, log_index)` |
| `note is not on the public path` | Nullifier was not reserved |
| `incorrect token denom` | Row denom is not an instantiated strategy |

---

## 3. Shipped this session

| Item | Evidence |
|------|----------|
| Accepted aggregate pays once; replay, mismatch, public-path collision, bad opening, and unreserved notes are rejected | `cargo run --example epoch_claim --features epoch-claim` from `crates/ict-rs/ict-rs`, image `terp-core:v630-arm64`, wasm `crates/headstash/artifacts/cw_headstash.wasm`. Printed `claim workflows passed`. |
| Three validators and three sidecars. One dissenter, two liars, one publisher, and a three-way split do not move the accepted root. Pending quorum does not replace it. Two of three then pays. | Same run. Genesis quorum in that example is `0.600000000000000000`. |
| Flood of 12 mismatched and replayed batches rejected in 12.7s. Height 35 to 40. Balance unchanged. A later two-of-three batch paid `15uterp` once. | Same example, after the flood edit. |
| Native proof timing, chain stopped | `cargo test -p zk-headstash --release --lib snap_bench -- --ignored --nocapture` from `crates/headstash`. Keygen 65.8s, prove 16.1s, verify 0.30s. |
| `hs-page` acknowledged this sequence and did not edit | Herdr agent `hs-page`, workspace HEADSTASH, 2026-10-05. |

The contract credit insert no longer stores the first payout as `2v` (`add_credit` in `headstash.rs`). `ApplyAcceptedBatch` on the Docker chain uses `ExistingFungible` and `BankMsg::Send`.

---

## 4. Deferred / do not claim

| Item | Notes |
|------|--------|
| Sidecar Halo2 verify and a holder `POST /claim` | Private apply cannot complete from the page until this exists. |
| Host prover invoked by the page | The wait state is specified. Wiring the 16s prove without putting `esk` in page state is still to build. |
| Watcher return of `(tx_hash, log_index)` | Public apply stays disabled without it. Do not add an Ethereum RPC on the page. |
| Denom-table query | Page must not invent the public amount. |
| Bandersnatch epoch, encrypt-rseed, NFT address to Y | Specified elsewhere. Not a button. |
| Hiding destination and amount inside the Halo2 instance | Later circuit change. Not required to run the epoch. |
| Halo2-KZG on-chain verify, attestations light client, Flock in-circuit ECDSA | Rejected for this cut. |
| Penumbra IBC and Prax shield | Sentence only. |
| Live `morocco-1` broadcast, commit, or push | Not part of the page work. |

---

## 5. Residual risks / reviews

- `FE-CLAIM-SPEC.md` still describes a snap, `ProcessHeadstash`, and K=18. Implementors who follow it will rebuild the wrong page.
- The live page still calls `invokeHeadstashSnap` method `prove` and `signClaim` still builds `ProcessHeadstash`. That walk is the false prover this file replaces.
- `claim_signing_message`'s comment says the body is a Poseidon digest. `process_headstash` passes `keccak256(wire[..168])`. The page follows `process_headstash`.
- Quorum `0.6` is the exercise genesis, not the module default. A chain left on `0.667` will not confirm two of three equal stakes. The page cannot repair that.
- The wasm the Docker image accepts was rewritten with `wasm-opt --enable-reference-types -O0`. That is an operator artifact, not a page dependency.
- Proof bytes and `notes.json` stay off the page and out of git.

---

## 6. Next session cold-start prompt

```
Restore the headstash claim page from
crates/headstash/docs/circuit/HEADSTASH-UI-HANDOFF-2026-10-05.md.
That file wins over FE-CLAIM-SPEC.md.

Stable surface: QueryMsg::EpochRoot, ExecuteMsg::ApplyAcceptedBatch,
ExecuteMsg::ApplyPublicBatch. Operator-only: SetDenomTable, AssignPublicNotes.
Signing string is claim_signing_message of keccak256 of the first 168
instance bytes. Pedersen is epoch::pedersen_commit. The page never holds esk.

Shipped: three-validator epoch exercise pays once and rejects the listed
misuse. Host prove is about 16s and verify about 0.30s. There is no POST /claim.

Next: implement /headstash against that file. Remove the snap prove.
Keep the DAO module. Disable ApplyAcceptedBatch until an accepted root
matches. Disable ApplyPublicBatch until a watcher supplies the log identity.
Show POST_CLAIM_CTA and do not build the IBC transfer. No morocco-1 broadcast.

Do not re-open: public destination and amount, Hashmerchant instead of the
attestations client, no Halo2-KZG, no Flock ECDSA, no snap, quorum math.
```

---

## 7. QMD index hints

| Field | Value |
|-------|--------|
| **Title** | Headstash claim UI handoff 2026-10-05 |
| **Tags** | `headstash`, `claim-ui`, `hashmerchant`, `halo2`, `pedersen`, `epoch` |
| **lex** | `ApplyAcceptedBatch ApplyPublicBatch EpochRoot headstash-accepted headstash-claim pedersen personal_sign` |
| **vec** | What contract messages does the headstash claim page send, and which steps are blocked until a sidecar verifies the Halo2 proof? |
| **Anti-claim** | This is not a snap prover, not ProcessHeadstash, not an on-chain Halo2 verify, not a Penumbra transfer, and not a hidden payout. |

**Index paths:** `crates/headstash/docs/circuit`

---

## 8. Links

- Older page sheet, superseded for UI: `crates/headstash/docs/circuit/FE-CLAIM-SPEC.md`
- Product checkpoint, historical: `crates/headstash/docs/circuit/CHECKPOINT-PRODUCT-A-POSEIDON-2026-07-30.md`
- Exercise: `crates/ict-rs/examples/epoch_claim.rs`
- Bench report written by the ignored test: `crates/headstash/circuits/artifacts/snap-proof-bench.json`
