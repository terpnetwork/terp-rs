# Proof sidecar

The product steps are `CLAIM-DESIGN.md`. This note is only the accumulate
rule.

Consensus stores a batch root. It does not store Ligerito proofs and it
does not verify them inside a block. The batch itself is a second Flock
circuit: one row re-checks one claim proof under the shared `PcsParams` and
the claim registry digest, and the public output is the accepted-nullifier
root. That circuit is specified in `CLAIM-DESIGN.md`. It is not optional
for the soundness of the sidecar. Native verify plus a signed root can ship
first.

A depth-2 claim proof is about 103 KB and verifies in about 6 ms on a
native Flock host. That verify is cheap for one process and expensive as
a consensus path: every validator would re-execute it, and the proof
bytes would sit in state. The Halo2 claim was already ~1.3 s to verify,
about half a block. The Flock claim moves verify off that path. It does
not move the proof into the store.

## Decision

A native sidecar, shaped like JAM accumulate, attested the way
hash-market already attests a root.

1. Submissions carry the proof and the public words (tree root, nullifier,
   destination, amount). They are not transactions that run the verifier.
2. The sidecar buffers them until the epoch ends or the buffer hits its
   cap, whichever comes first.
3. It verifies each proof with `headstash-claim` / `verify_ligerito_union_circuit`.
   Accept inserts the nullifier into a Blake3 set. Reject records the
   nullifier and the reason and does not insert it.
4. At the boundary it signs one vote extension: epoch, accepted-nullifier
   root, rejected count, and the list of `(destination, amount)` the batch
   accepted. Hash-market already turns many attributed inputs into one
   signed root (`bound_to_vote_extension`). The destination list is the
   egress beside that root.
5. The chain persists that root, the nullifier commitment, and the
   destination list. It does not persist the proofs. A blob store can keep
   them for audit.

The active validator key signs that extension once per epoch. An ante
handler checks the nullifier is absent from the accepted set and that the
submission is a receipt (a hash), not a proof. It does not run Ligerito.

## What we are not building first

**Wasmer around the verifier.** Wasmer in this repo is a release guest and
the contract engine. Flock verify is native (the same reason a Snap cannot
host it). Wrapping it in Wasmer drops the millisecond verify and does not
make the proof smaller.

**An ante or a contract that calls `verify_ligerito` per claim.** That
puts the proof back on the block path and back into state. The existing
`flock_host` entry stays for other Flock statements. Headstash claims do
not use it one-by-one.

**Autobahn lanes.** A lane can order proof blobs later if one sidecar
buffer is not enough. It does not check the proofs. Soundness stays the
sidecar verify plus the signed root.

**A trace proof of the verifier, in v1.** Proving "this process accepted
exactly this set" is the right second statement, and it is another Flock
circuit over the verify trace. v1 ships the native verify and the signed
root. The trace proof does not gate ingress.

**DREGG as the prover.** DREGG is the product shell: watchers, clerk
attestation, local runs, no broadcast. The sidecar can be the clerk that
attests the epoch set. It does not replace `headstash-claim`.

## Attributes

Zeratul-style attributes, already how this circuit is wired:

The claim witness includes `rseed`. The public inputs are the tree root and
the nullifier. `rseed_com` is in the leaf. The raw string is not. After the
registrar encrypts it to the registered Bandersnatch key and destroys the
plaintext, the sidecar cannot match `nf` to a note either. The spent set
still rejects a repeated `nf`.

## Chain surface

Hash-market vote extension, not a new ante verifier and not a wasm
contract. One feature on the hash-market ingress: accept a
headstash batch root, persist it, and let a later claim tx name a
nullifier that must already be in that set. Single-proof host import
stays available for other circuits. It is not the headstash path.
