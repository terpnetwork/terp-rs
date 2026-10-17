# Sidecar design

Public payouts are the product. The chain record of one claim is the nullifier, the destination, and the amount. Destination and amount stay public, and that is what the chain pays. The circuit does not hide the payout. It severs the eligible member from the authorization of that payout: the authorizing secp256k1 key, and which note that key opens. A sidecar and a chain observer see the nullifier, the destination, and the amount. They must not need the eligible public key, the signature `(r, s)`, or a commitment to the key.

Halo2 and Flock (Ligerito) prove this same statement. They are one experiment. The numbers to compare are prove time, verify time, and proof size. They are not two products and not two privacy statements. Older notes that replace the payout with a hiding commitment are not this design.

## Files read

- `crates/headstash/docs/circuit/PROOF-SIDECAR.md`
- `crates/headstash/docs/circuit/CLAIM-DESIGN.md`
- `crates/headstash/docs/circuit/AIRDROP-OPTIONS.md`
- `crates/headstash/docs/circuit/HALO2-ATTEST-CLAIM.md`
- `crates/headstash/circuits/claim-flock/src/lib.rs`
- `crates/headstash/circuits/claim-flock/src/statement.rs`
- `crates/headstash/circuits/claim-flock/src/sidecar.rs`
- `crates/headstash/circuits/claim-flock/src/batch.rs`
- `crates/headstash/circuits/claim-flock/src/tree.rs` (published leaf only; no note file was opened)
- `crates/headstash/circuits/halo2/src/circuit.rs` (instance columns, `attach_personal_sign`, `prove_ecdsa_verify`)
- `crates/headstash/circuits/halo2/src/claim_auth.rs`
- `crates/headstash/circuits/halo2/src/circuit/distro_poseidon_gadget.rs` (position bits)
- `x/hashmerchant/keeper/vote_ext.go`
- `x/hashmerchant/keeper/msg_server.go`
- `x/hashmerchant/spec/04_messages.md`
- `x/hashmerchant/spec/05_vote_extensions.md`
- `x/hashmerchant/spec/03_state_transitions.md`

The live Flock package is `crates/headstash/circuits/claim-flock`. `CLAIM-DESIGN.md` still names `crates/headstash/claim-flock`. That path is stale.

## One statement

Both provers speak one claim.

Public words of the proof:

- allocation root
- nullifier
- destination
- amount

The allocation root is a public word so the sidecar can check the proof opens the published tree. It is the same root for every note in the campaign. It is not a second field on the per-claim chain record.

Witnesses, not public words:

- `rseed`
- the Merkle siblings
- the leaf index, once the prover can treat it as a witness (Flock does not, today; see below)
- denomination and the other allocation words that are not the payout amount
- the eligible public key `Q`
- the signature `(r, s)`
- `esk` is not a witness. `claim_auth.rs` and `statement.rs` already say the prover does not receive it.

The proof shows four facts.

1. The opened note sits under the allocation root.
2. The nullifier is the nullifier of that note's `rseed`, not a hash of a public commitment and not a function of `(r, s)`.
3. The public amount is that note's amount.
4. `Q` on that note signed the `personal_sign` message below. The check is inside the proof.

What a verifier is allowed to conclude is that some eligible note authorized this public payout, and that the nullifier spends that note once. The verifier does not conclude which note, and does not receive `Q`.

`key_com` is not a public word once the signature is inside the proof. The Blake3 commitment of the leaf key exists so today's Flock sidecar can compare a key recovered beside the proof (`key_com`, `public_key_com`, `Sidecar::accept`). That comparison is the signature check standing outside the circuit. When both provers check ECDSA inside the proof, the sidecar does not need the key and does not need a commitment to it. Publishing `key_com` next to the payout would put the eligible key's identity back on the statement.

## The signed message

The holder signs with `personal_sign`. MetaMask applies the EIP-191 prefix. The message body binds four values: the allocation root, the nullifier, the destination, and the amount. A signature on any other body does not authorize this payout. Changing the destination or the amount is a different message, so it is a different signature, even when the note and the nullifier stay the same.

`(r, s)` and `Q` stay in the witness. The chain record does not carry them. The challenge scalar is not a chain word either. The message is determined by the four public values, so a published challenge would not name the key, but it is also not needed once the circuit hashes that message itself.

The code does not sign that body yet.

- Halo2 `attach_personal_sign` (`circuit.rs`) hashes the first 168 bytes of the instance, with the challenge still zero, and signs `headstash-claim-v1:` plus the hex of that 32-byte digest (`claim_auth.rs`). Those 168 bytes are anchor, denomination, amount, nullifier, recipient, and note commitment. The circuit then publishes the challenge as instance columns `E_LO` and `E_HI`. `prove_ecdsa_verify` does check `(r, s)` under `Q`. The message it checks is the current instance prefix, not the four-word body above.
- Flock `statement.rs` builds `headstash-private-claim-v1:` plus the hex of root, nullifier, and a hiding payout commitment. `build_private` does not constrain the signature. `Sidecar::accept` recovers `Q` beside the proof and checks it against the published key commitment. The Flock comment in `sidecar.rs` says the Flock circuit does not check the signature.

Those two messages are not this statement. The private-claim prefix and the payout commitment are the hiding-payout draft. This design does not use them.

## Chain record

One claim, one record:

```text
nullifier || destination || amount
```

One claim per nullifier. A second submission with the same nullifier is rejected, including one that names a different destination or amount. `Sidecar::accept` already does this with `SidecarError::Spent`. The chain stores the same rule.

The epoch batch is an ordered list of those records, plus a rejected count for submissions that did not enter the list. `batch.rs` folds each accepted `(nullifier, destination, amount)` under `FLAG_PAYOUT` and `FLAG_BATCH` and publishes the running root, the rejected count, and every accepted destination and amount. The chain list keeps the nullifier on each row. A root alone is not a substitute for the list: the payout is public, and the nullifier is what makes the spend once.

The chain does not store proof bytes, `(r, s)`, `Q`, or `key_com`. `PROOF-SIDECAR.md` already says the proofs stay off the block path. That part stands. What changes is the record: it is the triple, not a destination list plus a hiding commitment.

## What the sidecar does

The sidecar is one process that verifies proofs and emits the batch. It is not a consensus verifier and not a CosmWasm replay of each proof.

For each submission it checks:

- the proof verifies against this statement
- the allocation root is the published campaign root
- the nullifier is not already spent
- the destination and the amount are the public words of that proof

On accept it appends the triple and inserts the nullifier. On reject it increments the rejected count and inserts nothing. It then drops the proof bytes. It does not keep `rseed`.

It is not given `Q`, `(r, s)`, or `key_com`. Today's `Sidecar::accept` still takes `leaf_epk_x`, the message, and the signature, and it compares `public_key_com` to `key_com`. That is the current binary, not the statement this design locks. The signature has to move inside the proof before that argument list can go away.

`batch.rs` folds the triples. It does not yet replay each Ligerito claim proof inside the batch circuit. Until that row exists, the chain trusts the sidecar's native verify plus the signed batch, which is the split `PROOF-SIDECAR.md` already describes. The batch circuit is not a second privacy statement. Its public output is the list of public payouts.

## Leaf index

Flock still compiles the leaf index into the circuit shape. `Opening::index` selects the Merkle side at each level while `build` is wiring the gates (`lib.rs`). `verify_claim` rebuilds the circuit from the index alone, and the test `circuit_publishes_the_derived_claim` shows that flipping the index changes `circuit.digest()`. Private `rseed` and the siblings do not. The Flock sidecar must be told the index. Without it, `verify_claim` cannot rebuild the shape it checks.

What that leaks: the leaf slot, to the sidecar, and to anyone else who sees the submission. `PublicLeaf` in `tree.rs` is `(index, rseed_com)` and does not carry the key, the denomination, or the amount. Told the index, the sidecar can point at that published leaf and its `rseed_com`. It cannot, from the index alone, read `Q`. The chain record does not include the index, so a chain observer who sees only the triple does not learn the slot.

Halo2 is not in that shape. `Circuit.pos` is a witness. `calculate_distro_root_poseidon` takes bit `l` of that position and swaps with `CondSwapChip`. The instance column list has no position. One Halo2 verifying key covers every leaf. Flock does not, until the index is a witness and a mux instead of a wiring choice.

The suite's Ligerito accept is fixture depth 2 (`FIXTURE_DEPTH` in `lib.rs`, used by `sidecar.rs`). `DEPTH` is 16 because 51,807 notes need sixteen levels in the tree builder. That constant is not a measured Ligerito prove or verify. Do not quote depth-16 Ligerito timings. The depth-2 sizes in `PROOF-SIDECAR.md` and `CLAIM-DESIGN.md` are measurements of an earlier circuit, and `CLAIM-DESIGN.md` already says not to reuse them for a deeper path or for a proof that also hides the key.

## Two bars that are not sidecar-blind

Sidecar-blind means the proof and the chain record do not carry `Q`, `(r, s)`, or `key_com`. Two other links remain, and neither is fixed by moving the signature inside the proof.

The registrar who keeps the note file can still join a nullifier to a note. The nullifier is a hash of `rseed`. `tree.rs` keeps `rseed` on `SecretNote` and off `PublicLeaf`. `CLAIM-DESIGN.md` step 4 encrypts that string to the Bandersnatch point `Y` and then destroys the plaintext. That step is specified and not implemented. Until the ciphertext exists and the plaintext is gone, the registrar recomputes the nullifier from the file and learns which note paid which public destination. A copy that survives keeps that link. This is the registrar's bar. It is not the sidecar's. The sidecar never needed the file to verify the proof, and the proof does not become sidecar-blind by deleting the file.

A unique public amount can still name a note. The allocation amount is the payout amount, and the payout is public, so an amount that appears on one note appears on one chain row. `AIRDROP-OPTIONS.md` states that shared fact. Repeated amounts do not name a note. Unique amounts do. That leak is accepted because the payout is public. Hiding the amount would be a different product.

## Hashmerchant ingress

Hashmerchant is vote extensions on one governance-registered `chain_uid`. It is not a permissionless mempool lane. `MsgRegisterChain` is governance-only: `msg_server.go` requires the module authority, and `spec/04_messages.md` says that authority is the `x/gov` address. `VerifyVoteExtension` accepts an empty extension and rejects a `chain_uid` that is missing or disabled. A contract registration with escrow (`MsgRegisterContract`) only subscribes that contract to a sudo callback. It does not let the sender write a root.

What the module does, in `ProcessVoteExtensions`:

- Votes are grouped by `(chain_uid, algo)`, then by the hex of the root bytes. Identical root bytes share one bucket. Different bytes do not.
- `totalPower` is the sum of every vote's power in the injected `ExtendedCommitInfo`. The threshold is `QuorumFraction` times that sum. `spec/05_vote_extensions.md` calls this a quorum of the power in the commit and notes the default is about two thirds.
- An empty extension adds its power to `totalPower` and then continues. It contributes no root. Empty extensions still count toward total power, which raises the bar for every root.
- Only those votes are tallied. The sidecar HTTP handler fills the local validator's extension during `ExtendVote`. A fetch error returns an empty extension. A process that is not a bonded validator has no vote in that commit, so it cannot move a root by itself.

The ingress discussed for this sidecar is two roots under that one `chain_uid`, not a second module. The module stores a `HashRoot` at `(chain_uid, algo)`, so the two roots are two algos. Neither root is a proof and neither root is a mint.

- Pending, each block: the ordered lane of proof blobs. It commits to availability and order. It does not mean a proof verified, and it does not insert a nullifier.
- Accepted, at the epoch boundary: the claims whose proofs the sidecar verified. It authorizes the spent-set insert of each `(nullifier, destination, amount)`.

The public payout list is that accepted record. It is not replaced by an IBC attestations light client. `HALO2-ATTEST-CLAIM.md` is a different path: off-chain `proof_instance_verify`, a quorum of secp256k1 attestation signatures, and a chain record of a hiding payout commitment. `AIRDROP-OPTIONS.md` option 2 is that path. This design does not take it. Hashmerchant vote extensions stay the ingress, as option 3 already says.

The chain mints the public destination and the public amount. It does not mint a hiding payout commitment. `statement.rs`, `build_private`, and `HALO2-ATTEST-CLAIM.md` still describe that commitment as the public word (`payout_com`, flag `FLAG_PAYOUT_COM`). That was the draft that hides the payout. It is not the chain output of this design.

## Where the binaries differ

The locked statement is not what either binary publishes today. A review that treats the gaps below as already closed is reviewing a different protocol.

Flock `build` publishes the nullifier, `key_com`, the amount, the destination, and the root, and it also publishes the IV and the flags because `public_value` publishes at creation (`PUB_NF`, `PUB_KEY`, `PUB_AMOUNT`, `PUB_DEST_LO`, `PUB_DEST_HI`, `PUB_ROOT`). The amount and the destination are published twice and `claim_from_public` requires the copies to match. `build_private` publishes the nullifier, the root, and `payout_com`, and it hides the key, the amount, and the destination. Neither circuit checks secp256k1. The index is still the circuit shape.

Halo2 has one instance column. The constrained offsets in `circuit.rs` are `ANCHOR` (0), `HS_ND` (1), `HS_V` (2), `RECP` (3), `NF_OLD` (4), `CMX` (5), `E_LO` (6), `E_HI` (7). The footer counts eight instance values. That is the tree root, denomination, amount, recipient, nullifier, note commitment, and the two halves of the signature challenge. `(r, s)` and `Q` are witnesses, and `prove_ecdsa_verify` checks them. The challenge halves are still public. Denomination, recipient, note commitment, and the challenge are not public words of the locked statement. The position is already a witness, which is the one place Halo2 is ahead of Flock on this statement.

No prove, verify, or proof-size number for the locked statement is claimed here. The experiment is to measure those three on this statement, on both provers, and not to treat a depth-2 Flock sample or a K=19 Halo2 sample of a larger instance as the result.
