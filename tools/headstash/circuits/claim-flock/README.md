# Headstash claim on Flock

`headstash-claim` is the claim statement that can be proved without putting
`esk` or a Poseidon tree in the circuit. Flock is a library (`flock-core`,
`flock-hash`, `flock-prover`). This crate does not live inside the prover.

The locked product is `docs/circuit/CLAIM-DESIGN.md`. This crate is the
claim circuit, the Blake3 tree, and the native sidecar. The collection
contract is `cw721-headstash`. Encryption of `rseed` to `Y` is still the
registrar's step before a public claim window. It is not what mints the
first balance. Do not quote the depth-2 timings below for a depth-16 proof.
Rebench before calling the claim page fast.

The chain does not verify these proofs one by one. The sidecar verifies each
Ligerito proof natively, checks the MetaMask signature beside it, inserts
each nullifier once, and signs one batch. The batch circuit folds those
tuples. It does not replay each Ligerito proof. Which bytes are a reusable setup, and which are the
circuit, is `docs/circuit/LIGERITO-CACHE.md`.

## What this crate proves

`rseed` is a private witness. `rseed_com` is inside the leaf. The nullifier
is Blake3 of the string, not of the commitment. The leaf key, denomination,
and piece index are witnesses. The proof publishes a Blake3 commitment of
the key, which the sidecar checks against the MetaMask signature and then
drops. The chain batch carries the nullifier, the destination, and the
amount. The production depth is `DEPTH` (16), which holds the 51,807-note
set. Depth 2 is the four-leaf fixture. The published tree is
`tree::PublicLeaf`: index and `rseed_com` only.

| Row | Input | Output |
|---|---|---|
| Commitment | private `rseed` | `rseed_com` |
| Nullifier | the same `rseed`, different flag | public nullifier |
| Key | private leaf key | public Blake3 key commitment |
| Leaf | `rseed_com`, private key, private denomination and piece index | inner digest |
| Bind | that digest plus the public amount | Merkle leaf |
| Node | `left ‖ right`, side chosen by the public index bit | next digest |
| Root | last node | public root |

A second string is a different nullifier and a different leaf. The same
string always produces the same nullifier. Someone who has only the public
allocation cannot recompute it. The registrar who sampled the string can,
until that plaintext is destroyed.

Key ownership is not this circuit. The sidecar recovers the leaf key from
MetaMask `personal_sign` beside the proof. `esk` is not a witness.

## Flags

Each compression is domain-separated by the Blake3 flags word, not by a
second hash function.

| Flag | Value | Row |
|---|---:|---|
| `FLAG_RSEED` | 1 | commitment |
| `FLAG_NULLIFIER` | 2 | nullifier |
| `FLAG_LEAF` | 3 | leaf |
| `FLAG_NODE` | 4 | Merkle node |
| `FLAG_KEY` | 7 | key commitment |
| `FLAG_BIND` | 8 | amount binding |

The chaining value is the Blake3 IV. The message is one 64-byte block
(`block_len = 64`, counter 0).

## Proof system

`prove_and_verify` builds the circuit, proves it with
`prove_fast_ligerito_union_circuit`, and verifies it with
`verify_ligerito_union_circuit`. The Fiat–Shamir domain is
`terp-headstash-flock-v1`. A public root with the last word flipped is
rejected.

The Blake3 table is the one already in `flock-prover`. This crate only
supplies the wires. Capacity is `nu = 8` (256 rows). A claim uses
`5 + DEPTH` rows: commitment, nullifier, key commitment, allocation, and
the amount binding, then one node per level.

## Bench

`artifacts/flock-claim-bench.json` is the depth-16 sample from before the
key and the allocation became witnesses. This shape has two more Blake3
rows, so that file is not the number for the private claim. Rebench before
quoting a prove or verify time.

## What this is not

- Not a depth-32 community tree. The published Poseidon root in
  `artifacts/community-notes/` is a different statement.
- Not on-chain `proof_instance_verify`.
- Not a Wasmer guest. The verifier is native Flock.
- A note file that carries a secret `rseed` is not this statement. The
  published tree is a function of the public allocation only.
