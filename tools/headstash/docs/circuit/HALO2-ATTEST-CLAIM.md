# Halo2 attested claim

The iteration after option 3. Option 3 pays. This path is the slow claim that other users cannot match to a note. It does not replace the Flock claim, the Bandersnatch NFT, or the public `(destination, amount)` batch.

What happens, step by step. For each step: what is computed, by whom, from what, and what it proves. Where today's code differs, the section says so.

## 0. What this is

A holder proves the Halo2 claim on their own machine. Several off-chain services, each a CosmWasm VM, run the same `proof_instance_verify` the chain would have run. Each service that accepts signs an IBC attestation. The chain's attestations light client checks the quorum. The claim contract then records the nullifier and a hiding payout commitment. The block does not verify the Halo2 proof.

The pieces that already exist:

| Piece | Where | What it already does |
|---|---|---|
| Halo2 personal-sign claim | `zk-headstash`, K=19 | Inclusion, signature under the leaf key, nullifier bound to `rseed`. Prove about 17 s, verify about 1.3 s, proof 4,800 bytes. |
| Verifier entry | wasmvm `proof_instance_verify` | `cw-headstash` calls it inside the claim transaction today. Same VK, same footer, same param cache. |
| Attestations light client | `ibc-go` v11.2.0 `modules/light-clients/attestations` | Quorum of 65-byte secp256k1 signatures over an ABI packet. `go.mod` already requires this version. Rust types are `ibc.lightclients.attestations.v1`. |
| IBC v2 queries | `app/keepers/acceptedQueries.go` | Client v2 and channel v2 queries are already accepted. |
| Stake | `x/staking` | Bonded power and the operator key that owns a validator. |
| Spent set | `cw-headstash` `spend_nullifier` | One nullifier per root. |

What is not mounted yet: `app/app.go` registers the Tendermint client and the 08-wasm client. It does not register the attestations client. There is no module that binds a validator to an attestation key. The off-chain VM process does not exist. The live Halo2 instance is not private.

Hash-market vote extensions stay the option 3 ingress. They are a stake-weighted root inside CometBFT. They are not this path.

## 1. The private statement (holder)

The statement is the Halo2 claim, with a smaller public instance.

Today `HeadstashInstances` publishes the tree root, denomination, amount, recipient, nullifier, note commitment, and the two halves of the signature challenge. `process_headstash` then bank-credits `(denomination, recipient)`. That is a clearnet mint. A unique amount names the note. Running the attestation machinery on that instance makes a slow clearnet claim. It does not make a private one.

The private instance is three public words:

```text
root        the published allocation root
nf          Blake3/Poseidon nullifier of the committed rseed, as the circuit already derives it
payout_com  32-byte hiding commitment to the payout note
```

Everything else is a witness: `rseed`, the Merkle path, the leaf key, denomination, amount, the signature `(r, s)`, and the input note commitment. The circuit still checks three things.

1. The leaf opens to `root` under the published Poseidon tree.
2. `personal_sign` verifies under the leaf key. The signed message is ASCII `headstash-private-claim-v1:` plus the hex of `root || nf || payout_com`. MetaMask applies the EIP-191 prefix. `(r, s)` stay in the witness. The chain never sees them. Publishing them next to the challenge recovers the eligible key, so the receipt below does not carry them.
3. `nf` is the nullifier of this note's `rseed`, and `payout_com` commits to the same amount under a key the holder can later open. The amount is not a public input.

`payout_com` is 32 bytes because the light client rejects any other value length.

What it proves. This note is in the published set, this holder can sign for its key, the nullifier matches the committed string, and the payout commitment hides a note of that amount.

What it costs the holder. About 17 s to prove on the measured native host, and a 4,800-byte proof. The page still must not receive the Ethereum key. The proof is zero-knowledge: a verifier sees the three public words and the proof bytes, not `rseed`, `(r, s)`, the eligible key, the amount, or the destination.

What is still true from option 3's tree. The registrar who kept `rseed` can recompute `nf` and match it to a note. This path does not remove that. Option 4 is the path that does.

## 2. Off-chain CosmWasm VM (each registered validator)

Each attestor runs one process. The process embeds the same wasmvm the chain uses and calls `proof_instance_verify` with the circuit id, the proof, and the  instance bytes. It loads the VK and the Halo2 params from the same footer and the same param-cache key the chain would use (`prover_id`, `curve_id`, `k`, checksum). It does not link a second Halo2 library. It does not execute the claim contract, and it does not mint.

Computed by one service, from the proof the holder submitted:

```text
ok = proof_instance_verify(circuit_id, proof, instance)
```

`instance` is the private instance from step 1, the same bytes every service hashes. Accept is `ok == true`. Reject is any other return. A reject is not signed.

Verify is about 1.3 s. The services run in parallel, so a quorum costs one verify plus the time to collect signatures, not 1.3 s times the quorum. That cost is why the verify stays out of the block. Inside a block it is about half the block.

The service's secp256k1 attestation key signs only after accept. The key is the one registered in step 3. It is not the holder's MetaMask key, and it is not the validator's CometBFT ed25519 consensus key.

## 3. Stake-registered attestation key (validator operator, once per snapshot)

The light client trusts a list of Ethereum addresses and a count. It does not read the staking module. Stake enters by deciding who is on that list and how high the count is.

Computed by the registration module, from a transaction signed by the validator operator:

```text
address = keccak256(uncompressed secp256k1 pubkey)[12:]
record  = operator || address || compressed pubkey
```

The module accepts the record when the signer is that validator's operator, the validator is bonded, and the validator is not jailed. One attestation key per validator. A later registration replaces the key in the module. It does not change a light client that has already been created.

The snapshot, taken when a client is created:

```text
set = every registered address whose validator is bonded and not jailed
P   = their bonded power at that height
n   = smallest count such that the n smallest powers in the set
      sum to at least the quorum fraction of P
      (the same 2/3 fraction hash-market uses)
```

`n` is `min_required_sigs`. Every set of `n` attestors then carries at least that fraction of the snapshotted power, because the weakest such set does. Equal power gives the familiar two-thirds count. A heavy tail raises `n`. If the only count that works is the full set, the snapshot is still valid and the claim contract should say so. The client itself cannot weight signatures. Inventing a weight inside the client would be a different light client.

The attestor address stored in `ClientState.attestor_addresses` is the hex form `common.HexToAddress` accepts. Recovery uses that 20-byte address.

What it proves. A signature later in the quorum was produced by a key an operator bound to a bonded validator at snapshot time.

What it does not prove. The light client does not slash, does not rotate attestors, and does not freeze itself on a bad signature. Those limits are in the v11.2 package: no `RecoverClient`, no upgrade, no attestor rotation, no misbehaviour handler. A jailed or unbonded validator remains a valid signer on the current client until the claim contract moves to a new client id. The new client is how the set changes.

Accountability is the evidence in step 6, which uses this registration record to reach the operator's stake. The consensus key never signs an attestation, so a leaked attestation key is not a leaked block-signing key.

## 4. The attestation (each accepting service)

Two signatures, both from the registered key. Both are raw secp256k1 over a 32-byte digest. They are not `personal_sign`. go-ethereum recovers the address from the digest directly. A MetaMask `personal_sign` prefix will not verify.

The digest the client actually checks is in `TaggedSigningInput`:

```text
digest = SHA-256( type_tag || SHA-256(abi_bytes) )
```

`type_tag` is `0x01` for a state attestation and `0x02` for a packet attestation. The signature is 65 bytes `r || s || v`. `v` may be 27/28 or 0/1. The client maps 27 and 28 down to 0 and 1.

**State attestation**, tag `0x01`. ABI encoding of two `uint64`s, 64 bytes:

```text
height     a counter the services agree on for this batch of receipts
timestamp  unix seconds on the wire
```

`ABIEncode` expects the timestamp in nanoseconds and divides by 10^9 before packing. `ABIDecode` multiplies the wire seconds back. Callers pass nanoseconds into the encoder. On success the client stores a consensus state at revision 0 and that height. The consensus state is a timestamp. It is not a chain app hash, and it does not commit to the receipts. The v11.2 `StateAttestation` has no root field.

**Packet attestation**, tag `0x02`. ABI encoding of:

```text
height
packets[]: { path: bytes32, commitment: bytes32 }
```

For one accepted claim the service fills one entry:

```text
path_key   = "terp-halo2-claim-v1" || root || nf
path       = keccak256(path_key)
commitment = payout_com
```

`path_key` is the single key the chain will pass to `VerifyMembership`. The client hashes that key with keccak256 and compares it to `path`. `commitment` is compared raw to the 32-byte value. A batch is several entries in one packet attestation. Every accepting service signs the same ABI bytes. A service that disagrees on the height, the path, or `payout_com` produces a different digest and does not count toward this quorum.

What one signature proves. This registered key's service ran `proof_instance_verify` on this proof and instance, and the VM accepted. The chain does not re-check that fact. The quorum is the check.

What the signature hides. It does not contain the proof, `rseed`, the eligible key, the amount, or the destination. The packet carries `root`, `nf`, and `payout_com` in the clear, because those are the public instance.

## 5. The chain action (relayer, then the claim contract)

Anyone can relay. The relayer is not an attestor. They submit two messages.

1. `UpdateClient` with an `AttestationProof` whose data is the state attestation and whose signatures meet `min_required_sigs`. The client writes the consensus state at that height.
2. The claim transaction. It calls the attestations client's `VerifyMembership` with that height, the packet `AttestationProof`, `MerklePath` of one key (`path_key`), and the 32-byte `payout_com`.

`VerifyMembership` accepts only when all of these hold:

- the client is not frozen
- a consensus state exists at that height
- the proof signatures are tag `0x02`, unique, in the attestor set, and at least `min_required_sigs`
- the packet height matches
- some entry has `path == keccak256(path_key)` and `commitment == payout_com`

The claim contract then performs the action, and only this action:

```text
require client_id is the contract's current attestations client
require VerifyMembership(root, nf, payout_com)
require nf is absent for this root
insert nf
insert payout_com into the escrow's unspent output set
```

It does not call `proof_instance_verify`. It does not bank-send. It does not write the proof into state. The escrow already holds the allocation. `payout_com` is the holder's claim on a note inside that escrow. Spending `payout_com` is a later private spend. It is not this transaction.

A second quorum on the same `nf` fails the spent-set check. The quorum is not the double-spend lock. The spent set is.

`app/app.go` has to `AddRoute` the attestations light client next to the Tendermint and 08-wasm routes before any of these messages succeed. The query entry that maps `ibc.lightclients.attestations.v1.Query/Checksums` onto the wasm checksum response is not that route.

## 6. Evidence (anyone, later, off the claim path)

The light client will accept a quorum even if every signer lied, as long as the signatures are well formed. The correction is evidence against the registered key.

Computed by anyone, from a stored signature and the proof the service claimed to have checked:

```text
re-run proof_instance_verify(circuit_id, proof, instance)
```

If the VM rejects, every attestor who signed the packet commitment for that instance signed a false accept. The registration record names their operator. The stake to slash is that validator's bonded stake. One proof per evidence proposal. This re-verify is the 1.3 s path, and it is not a user claim transaction.

The stock client will not freeze itself from this evidence. The claim contract stops honoring the client id, and the next snapshot builds a new client without those attestors. That is the rotation the v11.2 client does not have.

A signature on a reject is not evidence of anything. Rejects are not signed, so they cannot be replayed as accepts. The path prefix `terp-halo2-claim-v1` keeps this packet out of any other attestation the same keys sign.

## Who learns what

| Party | Learns | Does not learn |
|---|---|---|
| Other users | `root`, a spent `nf`, a hiding `payout_com` | which note, the amount, the destination, the eligible key |
| One attestor | the proof bytes and the three public words | the witness, if the instance was actually cut down to those three words |
| The registrar who kept `rseed` | which note `nf` opens | the holder's signature |
| The chain | the quorum and the spent-set insert | the proof bytes |

An attestor who is given today's instance also learns denomination, amount, recipient, note commitment, and the challenge. Privacy starts when those leave the public instance. It does not start when the verifier moves off chain.

## What is sound today, and what this spec adds

Sound today, on the clearnet circuit: personal-sign Halo2 verify inside `cw-headstash`, the spent-nullifier set, and the ibc-go v11.2 quorum rules (unique recovered addresses, attestor set, `min_required_sigs`, keccak path, 32-byte value).

This spec adds, and nothing here is built yet: the three-word public instance, the output commitment `payout_com`, the off-chain wasmvm process, the operator-signed attestation key, the power snapshot that sets `min_required_sigs`, mounting the attestations client, and `VerifyMembership` as the claim authorization instead of `proof_instance_verify`.

Option 3 remains the payout that ships first. This file is the private Halo2 iteration after that payout path is complete.
