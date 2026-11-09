# Three ways to claim

One eligible note is paid once. The holder picks the privacy level before that note is placed in a root. The three pipelines do not share a proof system, and a note is listed in only one of them.

The build priority is the fully private path: the Halo2 claim verified off-chain, aggregated by a sidecar, and committed through Hashmerchant's existing vote-extension epoch. The public path does not get a new Ethereum client. It is a filter on the svg-mint watcher that already queries Ethereum for the Zcash mint.

## The choice

| Path | What an observer learns | What pays |
|---|---|---|
| Public 1:1 | The Ethereum sender and a fixed denomination index. The Terp destination is a Pedersen commitment until the batch opens it. | Ethereum CHT lock, watched by the existing mint runtime, then one Terp batch mint. |
| Transparency-minimizing | A public `address → Y` registration. Later, a public `(destination, amount)` with no eligible key on that row. | Bandersnatch epoch. `CLAIM-DESIGN.md`. |
| Fully private | A spent nullifier and a batch commitment. Not the eligible key. The sidecar checks the Halo2 proof and does not receive the key. | Halo2 sidecar plus Hashmerchant. |

Picking a path is an assignment of the note, not a second claim. A note in the Halo2 root is rejected by the Ethereum watcher. A note paid by the Ethereum batch is not in the Halo2 root and not in the Bandersnatch root. The spent sets differ because the proofs differ. The exclusion is which root the note was put in.

## Public: fixed index, Pedersen destination, batched mint

This converts CHT already sent on Ethereum into the same amount on Terp. An ERC-20 `transfer` has no memo, so the lock is a call on a small Ethereum contract, not a bare transfer. The calldata is the denomination index and a blinded Pedersen commitment of the Terp destination. The CHT moves in that same call.

```text
amount     = DENOM[index]
commitment = dest_scalar·G + blind·H
```

`DENOM` is a published table. The contract accepts the call only when the CHT amount equals `DENOM[index]`. Any other amount reverts. Indexes are the claim unit so a one-off amount is not an identifier.

`G` and `H` are two independent generators on one group the watcher can check in ordinary code. The blind stays off Ethereum. The destination is hidden on the Ethereum log. It becomes public when the batch mints, which is the point of this path.

The svg-mint runtime, the same process that watches Zcash deposits and already issues Ethereum client queries, subscribes to that contract's events. It does not grow a second RPC client. For each event it keeps `(tx hash, log index, sender, index, commitment)`. The sender must be the eligible Ethereum address for a note of that index that was assigned to the public path. Anything else is ignored.

Openings arrive off Ethereum: the holder gives the watcher `(destination, blind)`. The watcher checks the Pedersen equation against the stored commitment. Unopened events wait. They are not minted.

Once an epoch of openings is ready, the runtime submits one Terp transaction: the list of `(destination, index)` and the Ethereum `(tx hash, log index)` of each row. The Terp contract checks the index table, rejects a repeated log, and mints `DENOM[index]` to each destination. One transaction carries the whole batch. Proof bytes are not involved.

## Transparency-minimizing: Bandersnatch epoch

This is `CLAIM-DESIGN.md`, unchanged. MetaMask derives a Bandersnatch key. A public NFT binds `address → Y`. The registrar encrypts `rseed` to `Y` and deletes the plaintext. The Flock claim publishes the nullifier, the destination, and the amount. A sidecar verifies proofs through the epoch and the chain records the nullifier root plus the payout list.

The eligible key is not on the payout row. The registration NFT is public, a unique amount still names a note, and privacy from the registrar is the deletion assumption. The Flock binary still publishes the leaf key. That is a gap in the circuit, not a reason to rebuild it before the private epoch below is running.

## Fully private: Halo2 sidecar, Hashmerchant epoch

The holder proves the live Halo2 claim on their own machine. The eligible key, `(r, s)`, and `esk` stay witnesses. `prove_ecdsa_verify` already checks the signature. The chain never calls `proof_instance_verify`.

The public instance is still the live 200-byte wire: allocation root, denomination, amount, nullifier, destination, note commitment, and the challenge. Denomination, amount, and destination are visible to whoever holds the proof, including the sidecar. Hiding those columns is a later circuit change. It is not required to run the epoch. What this path already keeps off the sidecar is the eligible key.

Hashmerchant is the epoch. `x/hashmerchant` already does the round: a validator HTTP-GETs `{sidecar-url}/vote-extension`, empty extensions still count toward total power, and a quorum of bonded power on identical root bytes writes one `HashRoot` per `(chain_uid, algo)` and sudos registered contracts (`spec/05_vote_extensions.md`, `spec/06_sudo_interface.md`). Governance registers one `chain_uid`, `headstash-claim`. Two algorithms on that id are the two roots. No module change is required for the split.

| Algo | When | Root is |
|---|---|---|
| `headstash-pending` | every block | Hash of the ordered proof blobs accepted into the lane. Availability and order only. Not a statement that any proof verified. |
| `headstash-accepted` | epoch boundary | Hash of the canonical batch: allocation root, and each accepted row `(nullifier, denomination, destination, amount)`. |

The sidecar is the process behind that URL. It is not a new chain module. On a submission it loads the same wasmvm, verifying key, and params the chain would have used, runs `proof_instance_verify`, and checks the challenge the contract already checks: `e` equals the `personal_sign` challenge of the first 168 bytes of the instance. `proof_instance_verify` alone does not bind the payout. On accept it keeps the four row fields and drops the proof. A repeated nullifier is dropped before it enters the accepted batch. At the epoch it serves the accepted root. During the epoch it serves the pending root.

The claim contract registers with `MsgRegisterContract` for `headstash-claim`. On sudo it may mint only from `headstash-accepted`, and only when the batch body hashes to the sudo root. `oracle_sources` are hints. The sudo spec says not to mint from them. The contract inserts each nullifier, rejects a repeat, and credits `(denomination, destination, amount)` from the body.

`process_headstash` is not that contract path. Its credit loop is still a stub, and the first payout is stored as twice the claimed amount (`or_insert(v)` then `+= v` in `contracts/cw-headstash/src/headstash.rs`). The epoch mint uses a new batch entry that does not double, and it does not verify Halo2.

The ibc-go attestations client stays unmounted. This epoch is Hashmerchant. `HALO2-ATTEST-CLAIM.md` describes the light-client variant and a hiding payout commitment. It is not the path being built.

## What is already built

For the private epoch:

- The Halo2 personal-sign circuit, the 200-byte instance, and `proof_instance_verify`.
- The challenge check and `spend_nullifier` in `cw-headstash`.
- Hashmerchant's sidecar URL, quorum, `HashRoot`, and sudo. The in-house sidecar shape is `crates/terp-rs/tools/hash-market`.
- A governance-registered `chain_uid` can carry both algorithms with no module fork.

Not built, and this is the work:

- A headstash handler on the vote-extension URL that verifies Halo2 and returns `headstash-pending` or `headstash-accepted`.
- The contract sudo that recomputes the accepted batch hash, spends nullifiers, and credits. Including the double-count fix.
- Assignment of each note to one path, so the Ethereum watcher and the Halo2 root cannot both pay it.

The Bandersnatch NFT, the encryption, and the Flock key staying a witness are specified in `CLAIM-DESIGN.md` and are not this build. The Pedersen lock and the batch mint are a filter on the existing Ethereum watcher, specified above, and are not a second indexer.

## Out of this build

In-circuit secp256k1 on Flock. A Halo2-KZG port. Mounting the attestations light client. Hiding destination and amount inside the Halo2 instance. A new Ethereum RPC stack beside svg-mint.
