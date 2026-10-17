# Headstash claim

What happens cryptographically, step by step. For each step: what is computed, by whom, from what, and what it proves. This is the design to critique. Where the code differs, the section says so.

The code that exists today is `crates/headstash/claim-flock`. It is the Blake3 claim circuit and the batch fold. It is not the NFT, not the encryption, and not a sidecar process. The other ways to run this airdrop are `AIRDROP-OPTIONS.md`. This file is option 3.

## 0. Setup (once)

Three objects, and they are not interchangeable.

**secp256k1.** MetaMask signs with this. A secret is the Ethereum account key. A public key is `epk`. The only operation the wallet performs is an ECDSA signature of an EIP-712 digest. It does not decrypt, and it does not export the key. Nonces are RFC 6979 in MetaMask, so the same account and the same message yield the same `(r, s)`. That determinism is a wallet property. It is not a chain rule. A wallet with a random nonce will not reproduce the claim key. `v` is not used, because wallets encode it as 27/28 or 0/1.

**Bandersnatch.** The claim key lives here. Secret `x` is a scalar. Public key `Y = x·G`, 32 bytes compressed. `x` is derived from a secp256k1 signature. It is not the Ethereum key. Encryption of `rseed` is to `Y`, so opening it needs `x`, which the holder can rebuild by signing again.

**Flock / Ligerito.** The proofs live here. There is no elliptic-curve SRS and no Halo2 proving key. The reusable setup is `PcsParams`: `m`, `log_inv_rate`, `log_batch_size`, `profile`, `num_lanes`, `merkle_hash`. The circuit id is `Registry::digest`, a BLAKE3 of the tables and the matrices. The witness commitment is a Merkle root of Reed–Solomon codewords. Two circuits share params when those six fields match. They do not share a digest. Detail is `LIGERITO-CACHE.md`.

Blake3 is the hash inside the claim and the batch. One row is one compression of a 64-byte message under the Blake3 IV, counter 0, block length 64. Flags separate the rows:

| Flag | Value | Row |
|---|---:|---|
| `FLAG_RSEED` | 1 | commitment of the secret string |
| `FLAG_NULLIFIER` | 2 | nullifier |
| `FLAG_LEAF` | 3 | allocation leaf |
| `FLAG_NODE` | 4 | Merkle node |
| `FLAG_PAYOUT` | 5 | one accepted payout |
| `FLAG_BATCH` | 6 | fold into the batch root |

There is no trusted setup. A forger who wants a false claim has to break Blake3, the Ligerito code, or the signature. Nothing is toxic waste except the per-note `rseed`, and that waste is the registrar's plaintext, which step 4 deletes.

## 1. The public allocation set (registrar, once)

The registrar samples 32 bytes `rseed` per eligible note. The note that is published is not that string.

Computed by the registrar, from the eligible list and the sampled string:

```text
rseed_com = Blake3(rseed)           under FLAG_RSEED
leaf      = Blake3(rseed_com || epk || nd || v || fdi)   under FLAG_LEAF
parent    = Blake3(left || right)   under FLAG_NODE
```

The index bit at each level chooses which side the running digest is on. The tree root is public. Every leaf preimage except `rseed` is public: eligible key, denomination, amount, piece index, and `rseed_com`. Anyone can recompute the root from the published notes and check an allocation is in the set.

What this proves: the set of allocations is fixed. Adding a note later is a different root.

What it does not prove: who will claim, or that the registrar deleted `rseed`.

The current circuit wires a path of depth 2. The index is compiled into the wiring, so a proof for index 1 is a different circuit shape from a proof for index 0. That publishes the position. A review should treat depth-2 fixed wiring as a stand-in, not as a hidden index. A real tree is depth 32 with the index as a witness and a mux, which this circuit does not have.

The current builder also marks `epk` and the packing `[nd, v, fdi]` as public inputs, because that is how the wire is pinned. A critique should not describe the implemented proof as hiding the eligible key. The intended public inputs are only the tree root, the nullifier, the destination, and the amount. `epk` and the rest of the allocation have to become witnesses before that is true.

## 2. Deriving the claim key (browser + MetaMask)

The Ethereum key stays in the wallet. The browser asks MetaMask to sign one fixed EIP-712 message.

Domain: `{name: RingAirdrop, version: 1}` with no `chainId`.
Message: `{campaign, purpose: "bandersnatch-key-derivation"}`.

MetaMask returns `(r, s, v)`. The browser computes

```text
seed = SHA-256(r || s)
x    = hash_to_scalar(seed)
Y    = x·G                         on Bandersnatch
```

`v` is dropped. The signature is not sent and is not a nullifier. Determinism is only so the same account can rebuild `x` by signing that message again.

What this gives: a key only that account can reproduce. What it costs: anyone who later sees that signature owns `x`. The message is signed only in this step.

This step is specified. It is not in `claim-flock`.

## 3. NFT registration (browser + MetaMask → chain)

This step is public on purpose. `Y` is a key later protocols can use. It is not the private spend.

MetaMask signs a second EIP-712 message.

Domain: `{name: RingAirdrop, version: 1, chainId}`.
Message: `{campaign, bandersnatchPubkey: Y}`.

The `chainId` is replay protection. The mint carries `(address, Y, signature)`. The contract recovers the signer and mints only when all of these hold:

- the recovered signer is `address`
- `address` is in the eligible set
- `address` has not minted
- `Y` has not been minted
- `Y` decodes to a Bandersnatch point

The token stores `address → Y` in public state.

What this proves: the holder of that Ethereum key bound exactly one claim key. One address, one `Y`. What it leaks: who registered, and which `Y` they bound. It does not leak `rseed`, the destination, or the amount. Those are not in the mint.

The double-spend of the *note* is not this NFT. A second mint fails here. A second *claim* fails later, on the nullifier. A registrar who skips the contract and writes their own `Y` for an address cannot, because the contract checks the signature.

This step is specified. It is not implemented.

## 4. Encrypt `rseed` to `Y`, then destroy it (registrar)

`Y` does not exist when the tree is built, so the ciphertext cannot be made in step 1. A ciphertext under the Ethereum key cannot be opened: MetaMask will not decrypt, and a signature is not a decryption.

After the NFT exists, the registrar encrypts that note's `rseed` to `Y` and returns the ciphertext. Then it destroys the plaintext.

What the holder does: sign the derivation message again, rebuild `x`, decrypt.

What this proves, if the plaintext is actually gone: the registrar can no longer compute `Blake3(rseed)` and match a later nullifier to a note. Other users never could. What it does not prove: that the registrar kept no copy. There is no cryptographic deletion. The assumption is the registrar's. A copy that survives links every claim from that note, forever.

This step is specified. It is not implemented. The encryption scheme on Bandersnatch (ECIES against `Y`) is not chosen in code.

## 5. The claim proof (holder, locally)

The holder has `rseed`, the public leaf, the siblings, a destination address, and the claim key. The Ethereum key is not an input.

Computed inside the Flock circuit, from the witness `rseed` and the public allocation:

```text
rseed_com = Blake3(rseed)                              FLAG_RSEED
nf        = Blake3(rseed)                              FLAG_NULLIFIER
leaf      = Blake3(rseed_com || epk || nd || v || fdi) FLAG_LEAF
root      = Merkle fold of leaf and siblings           FLAG_NODE
```

`nf` and `rseed_com` are the same compression of the same string under two flags, so they are not equal and one does not reveal the other. The nullifier is not `Blake3(rseed_com)`. That commitment is public. Hashing it would name the note.

The amount published is the amount word inside the leaf message. In the circuit that word is the same wire the leaf hashes (`tail` packs `[nd, v, fdi]`, and `v` is the amount). The prover cannot name a larger amount without opening a different leaf.

The destination is a 20-byte address, five words, published beside the amount. It is not inside the leaf. Changing it does not change `nf`. The proof is bound to it because it is a public input of this proof.

What the proof is supposed to show: this `nf` opens exactly one committed string in the published tree, the amount is that note's amount, and the holder named this destination. What it is supposed to hide: which eligible key, and therefore which address.

What the current circuit actually publishes, because `public_value` publishes at creation: the IV, the flags, `epk`, the packing `[nd, v, fdi]`, the destination, `nf`, and the root. `rseed` and the siblings are the private wires. Until `epk` and `[nd, fdi]` stop being public inputs, an observer reads the eligible key off the proof and the hiding spend is not what this binary does. The tests check the nullifier, the root, and that the published amount word equals `v`. They do not check that `epk` is hidden.

A second `rseed` fails the leaf and produces a different `nf`. The same `rseed` repeats `nf`. That is the double-spend fact. It does not depend on the NFT.

The Fiat–Shamir domain is `terp-headstash-flock-v1`. One measured sample of the earlier five-gate circuit, before the destination wires: build 1.63 s (the Blake3 table, once), prove 17.2 ms, verify 5.8 ms, proof 103,194 bytes. Depth 2. Do not quote that size for a depth-32 path or for a proof that also hides `epk`.

## 6. Accumulate and the batch proof (sidecar, then one chain write)

Claims sit in a buffer until the epoch ends or the buffer is full. The sidecar verifies each claim proof under the shared `PcsParams` and the claim `Registry::digest`. Accept inserts `nf` into the spent set and keeps `(destination, amount)`. Reject increments a counter and keeps nothing else. Proof bytes are dropped. `rseed` is not in the buffer.

The batch circuit then folds the accepted rows. For each payout:

```text
digest = Blake3(nf || destination || amount)     FLAG_PAYOUT
root'  = Blake3(root || digest)                  FLAG_BATCH
```

Amount sits in the last word of the payout message. Changing it changes the root. Rejected claims are a public count. They are not hashed.

The chain record for the epoch is:

- the accepted-nullifier root
- the rejected count
- the list of `(destination, amount)` the batch accepted

That list is the sentence "these addresses and these amounts were checked as payouts of valid claims." It is not a list of eligible addresses. The eligible address and `Y` are not on it.

What the batch circuit does today: it folds those tuples and publishes the root. What it does not do: replay each Ligerito claim proof inside the circuit. A row that re-checks one claim proof, under the same `PcsParams` and the claim registry digest, is the missing piece. Until it exists, the chain trusts the sidecar's native verify. The signed batch root is the hash-market vote-extension shape. It is not yet wired.

A local test file `claim-flock/tests/batch_egress_record.rs` writes an egress record. It is not a cw-orch or ict-rs run, and it is not in the checkpoint commit.

## How the spend is hidden, and where it is not

Three separate facts. Each covers one link an observer would otherwise make.

**a. The nullifier is not a function of the public leaf.** `nf = Blake3(rseed)` under `FLAG_NULLIFIER`. The public leaf has `rseed_com = Blake3(rseed)` under a different flag. Finding `rseed` from `rseed_com` is a Blake3 preimage. An observer with the eligible set can recompute every `rseed_com`. They cannot recompute `nf` unless they have the string. After step 4, the registrar is in the same position, if the plaintext is gone.

**b. The signature is not the nullifier.** A second secp256k1 nonce would be a second `(r, s)` and, if `nf` were derived from it, a second spend. `nf` does not depend on `(r, s)`. The derivation signature is not sent. The registration signature is a different message, so it does not reveal `x`.

**c. The batch does not publish the eligible key.** The intended egress is destination and amount. It is not `epk` and not `Y`. Paying address `D` the amount `v` does not, by itself, say which eligible account owned the note.

What the hiding does not cover:

- The anonymity set is the set of notes with the same amount. A unique amount on the egress names the note. Repeated amounts do not.
- The registrar's copy of `rseed` names the note. Destruction is an operational claim.
- The NFT publishes `address → Y`. Anyone who later sees `Y` inside a proof links the claim to the Ethereum account. The claim proof must not publish `Y`.
- The implemented circuit publishes `epk` and `[nd, v, fdi]`. That is a direct link, and it is the first thing a review should refuse to call private.
- The implemented path fixes the index in the wiring. The circuit shape identifies the leaf position at depth 2.
- Side channels are outside the math. Registration time, claim time, IP, and a payout of a unique amount correlate. Batching removes payout timing. It does not remove a unique amount.
- The derivation signature is the crown jewel for the claim key. Phishing that one message reveals `x` and opens the ciphertext. It does not reveal the Ethereum key.

## Who learns what

| Party | Learns | Does not learn, if the proof hides `epk` and the plaintext is gone |
|---|---|---|
| MetaMask | that two EIP-712 messages were signed | `rseed`, destination, amount, which note |
| Public | eligible set, tree root, NFT `address → Y`, spent `nf`, accepted `(destination, amount)` | which note an `nf` came from, unless the amount is unique |
| Registrar after destruction | who registered which `Y`, and the public payout list | which `nf` is which note |
| Registrar if it kept `rseed` | the note behind each `nf` | a signature it can spend with |
| Holder | their `rseed`, their destination | other holders' strings |

## What is sound, and what is only specified

Sound in the current tests:

- The same `rseed` repeats `nf` and the root.
- A second `rseed` changes both.
- The published amount word is the leaf's `v`.
- The batch root changes when an amount changes.
- A Flock footer round-trips, and its param file key differs from a Halo2 footer with the same `k` and the same checksum.

Specified, not built:

- EIP-712 derivation and the NFT mint.
- Encryption to `Y` and destruction of plaintext.
- A sidecar process.
- A batch row that re-verifies a claim proof.
- A vote extension carrying the destination list.
- Depth 32 with a hidden index.
- Removing `epk` and `[nd, fdi]` from the public inputs.

A review that treats the last group as already true is reviewing a different protocol.
