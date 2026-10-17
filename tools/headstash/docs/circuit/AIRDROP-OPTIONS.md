# Airdrop options

Four ways to run the same product: a public list of eligible keys and amounts, a spend that other users cannot match to a row, and one claim per note. They differ in who learns `rseed`, whether the tree can be built before anyone shows up, and what the chain does at claim time.

The option written out step by step is option 3, in `CLAIM-DESIGN.md`. The Flock crate implements the Blake3 half of that option. It does not implement the NFT, the encryption, or the sidecar.

Shared facts, so they are not repeated under every option:

- A secp256k1 signature is not a nullifier. A second nonce is a second `(r, s)`. If the nullifier were derived from the signature, that would be a second spend.
- `nf = Blake3(rseed_com)` is not private. `rseed_com` is in the public leaf, so anyone with the list can recompute it.
- `nf = Blake3(rseed)` is private against anyone who does not have the string. The same string always repeats `nf`. A different string fails a leaf that committed to the original string.
- A unique amount on a public payout list names the note. Repeated amounts do not.
- The chain should store a spent-nullifier set and, when a payout is public, the `(destination, amount)` list. It should not store proof bytes.

## 1. Private NFT, interactive registration

The holder must register before the note can be opened. The registrar is the only party who sees who is claiming. The public does not.

What happens. The registrar samples `rseed` and publishes `rseed_com` in the tree. The holder derives a claim key, or shows the eligible Ethereum key, in a registration the registrar keeps off chain. The registrar records `address → claim key` privately and does not mint a public token. It then encrypts `rseed` to that claim key. The claim proof publishes `nf` and the tree root. It does not publish the Ethereum address.

What it proves. One registration per address, if the registrar enforces it. One spend per note, if `nf` is `Blake3(rseed)` and the spent set is public.

Who learns what. The registrar learns who intends to claim. After it deletes `rseed`, it cannot match `nf` to a note. Other users cannot either. The public learns that a claim happened, not which address.

What it costs. Registration is a round trip. The tree cannot encrypt to a claim key that does not exist yet, so `rseed` stays on the registrar until that round trip finishes. A public NFT is a different option: option 3 publishes `address → Y`.

What exists. The private registration record does not. The Blake3 circuit does.

## 2. Slow, and private against other users: Halo2, attested by an IBC v2 light client

The proof system is the Halo2 claim. Prove is on the order of 17 s. Verify is on the order of 1.3 s, about half a block, so the chain does not verify the proof inside the block. The iteration after option 3 is specified in `HALO2-ATTEST-CLAIM.md`. Off-chain CosmWasm VM services run `proof_instance_verify`. Validators register a secp256k1 attestation key against their stake. A quorum of those keys signs an IBC packet, and the ibc-go v11.2 attestations light client authorizes the spent-set update. Hash-market vote extensions stay the option 3 ingress. They are not this path.

What the current Halo2 instance actually publishes: tree root, denomination, amount, recipient, nullifier, note commitment, and the signature challenge. Denomination, amount, and recipient are public inputs today. Calling this option "purely private" is the goal, not the binary. A review that needs the spend hidden has to take denomination, amount, and recipient out of the public instance, or accept that those fields name the payout and, if the amount is unique, the note.

What it proves, once those inputs are actually hidden. Inclusion in the published tree, a signature under the leaf key, and a nullifier bound to the committed `rseed`. The spent set rejects a repeat. The public instance for that private statement is the tree root, the nullifier, and a 32-byte hiding payout commitment. The attestations client is what makes the slow verify fit in a block: the block checks the quorum, not the Halo2 proof.

Who learns what. Other users do not learn which note, if the public instance is only the root, `nf`, and the hiding payout commitment. The registrar who kept `rseed` does. Attestors see the proof bytes and those public words. They do not see the witness. The chain stores the quorum result and the spent-set insert, not the proof.

What it costs. Prove time. A 4,800-byte proof. One wasmvm verify per attestor, in parallel. It does not require a Bandersnatch key.

What exists. The personal-sign Halo2 circuit, native benches, `proof_instance_verify`, and the ibc-go v11.2 attestations client library. Not mounted in `app/app.go`. Not a private instance. Not the off-chain VM, and not the stake-to-attestation-key registration. The full step-by-step is `HALO2-ATTEST-CLAIM.md`.

## 3. Fast, leaky, and trusted: Bandersnatch registration, encrypt, discard

This is `CLAIM-DESIGN.md`.

The holder derives a Bandersnatch key from one MetaMask EIP-712 signature (`seed = SHA-256(r || s)`, RFC 6979). A second signature mints a public NFT, `address → Y`. The registrar encrypts `rseed` to `Y` and is supposed to destroy the plaintext. The claim is the Flock Blake3 circuit: private `rseed`, public `nf`, destination, and amount. The sidecar batches proofs. The chain stores the nullifier root and the `(destination, amount)` list.

Fast means the measured Flock verify, about 6 ms, and prove about 17 ms, on the depth-2 circuit. Leaky means the NFT is public, the payout list is public, and a unique amount names the note. Trusty means privacy from the registrar is the assumption that it deleted `rseed`. Encryption hides the string from other users. It does not hide it from the sampler, and it does not prove deletion.

What it proves. The eligible key bound one `Y`. The opened string matches the leaf. The amount is the leaf amount. `nf` has not been seen. A batch proof, once the replay row exists, shows the sidecar checked those claims.

What exists. The Blake3 claim circuit and the batch fold. Not the NFT, not encryption, not the sidecar process, not a batch row that re-verifies a Ligerito proof. The implemented circuit still publishes `epk` and `[nd, v, fdi]`. Until those are witnesses, this binary is leakier than the spec.

## 4. Fast, and the registrar never knows `rseed`

The holder samples `rseed`. They publish only `rseed_com`. The registrar never sees the string, so it cannot match `nf = Blake3(rseed)` to a note even if it keeps every file it was given.

The tree cannot be fixed before those commitments exist. At the end of an epoch the registrar rebuilds the root from the public allocations plus the `rseed_com` values that showed up. A note with no commitment is not in that epoch's tree. This is a registration of a commitment, not of an identity. It is the option that was ruled out when non-interactive issuance was required. It is the option that actually removes the registrar's knowledge.

How the holder can deliver `rseed_com` without delivering `rseed`:

- Send the 32-byte commitment and nothing else. No key exchange is required. The registrar inserts it and rebuilds the root. This is the whole mechanism.
- PeerDAS plus KZG is a way to *publish* a batch of those commitments so the chain can check the blob was available and the commitment opens. It does not hide a preimage from the party the user sent the blob to. If the blob is `rseed_com`, the registrar still does not learn `rseed`. If the blob is `rseed`, KZG does not help.
- A blinded Diffie–Hellman exchange matters only when the registrar must contribute entropy and must not learn the result. Each side multiplies a point by a secret, the shared string is the secret, and neither side's scalar leaves its machine. That is the same shape as n-of-n shares. It is extra machinery if the holder is willing to sample `rseed` alone. It is the right machinery if the registrar's randomness has to be inside `rseed` and the registrar must still be unable to compute `nf`.

What it proves. Same claim circuit as option 3, because the leaf still commits to `rseed_com` and `nf` is still `Blake3(rseed)`. The difference is who sampled the string and when the root was built.

What it costs. The root waits for the epoch. Addresses that never send a commitment are absent. The proof is still the fast Flock circuit.

What exists. Nothing of the epoch rebuild. The circuit would not change.

The product choice is `THREE-CLAIMS.md`: one note, one of three paths. The build is the Halo2 sidecar on Hashmerchant. The four rows below are the earlier mechanism notes, not three parallel implementations.

## How to choose

| | Registrar knows `rseed` | Root before anyone arrives | Public `address → Y` | Chain verifies each proof | Payout list on chain |
|---|---|---|---|---|---|
| 1. Private registration | yes, until it deletes it | no | no | no | choose |
| 2. Halo2 + IBC attestation | yes, if it sampled `rseed` | yes | no | no, one quorum | hiding commitment |
| 3. Bandersnatch, encrypt, discard | yes, until it deletes it | yes for the allocation, no for the ciphertext | yes | no, one batch | yes |
| 4. Holder samples `rseed` | no | no, rebuild at epoch | optional | no | choose |

Option 3 is the one the long spec describes. Option 4 is the one that answers "the registrar must not know `rseed`" without a threshold ceremony. Options 1 and 3 are the same cryptography with a different publication rule for `Y`. Option 2 is the same privacy story on the slow prover, and only if its public instance stops carrying denomination, amount, and recipient. The step-by-step for that iteration is `HALO2-ATTEST-CLAIM.md`.
