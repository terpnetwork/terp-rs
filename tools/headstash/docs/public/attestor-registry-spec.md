# Spec (draft, not implemented): decentralized attestor registry for cw-headstash

Status: proposal for review. No contract changes have been made.

## Summary
1. Validators register a sidecar attestor public key on-chain in cw-headstash,
   proven by a signature from that key over a domain-separated message and bound
   to their validator operator address.
2. A campaign snapshots the registered set (or its hash) at creation, alongside the
   locked `vk_hash`, so the attestor set cannot change under an open campaign.
3. Payout keeps x/hashmerchant quorum as the gate; the registry adds curation,
   rotation and removal without config-file edits.

## Problem
Today attestation trust is implicit: any validator whose sidecar returns a valid
custody signature counts toward the x/hashmerchant quorum, and operator keys for
campaign/seal routes live in each sidecar's `operator_pubkeys`. There is no public,
auditable list of which sidecar keys attest for a Headstash campaign.

## Data flow
1. Validator runs `hash-market-server` and obtains its custody pubkey.
2. Validator operator (valoper account) sends
   `RegisterAttestor { pubkey, algo: "secp256k1"|"ed25519", signature }` where
   `signature` = attestor key over
   `headstash-attestor-v1:{chain_id}:{contract}:{valoper}:{pubkey_hex}:{nonce}`.
3. Contract checks: sender is the valoper's account (staking query), validator is
   bonded/in the active set, signature verifies (native `secp256k1_verify` /
   `ed25519_verify`), pubkey not already registered, nonce unused.
4. Stored: `ATTESTORS: Map<valoper, Attestor { pubkey, algo, since_height, status }>`.
5. `SetEpochCampaign` records `attestor_set_hash` = sha256 of the sorted active
   attestor list at creation, immutable like `vk_hash` ("attestors are fixed").
6. Sidecars read the registry (smart query `Attestors {}`) to know peer keys.
   x/hashmerchant keeps verifying vote extensions as today.
7. `RemoveAttestor` (self) and `PruneAttestors` (permissionless; drops validators
   no longer bonded) keep the set current between campaigns.

## Security notes
- Domain separation (chain_id, contract, valoper, nonce) prevents replay across
  chains, contracts and validators.
- Binding to valoper plus active-set check gives sybil resistance by stake.
- Snapshot-at-creation avoids moving the trust set during a campaign; rotation
  applies to new campaigns only.
- Liveness: a stale registry must not block payout; quorum stays in
  x/hashmerchant. The registry is additive curation, not a second quorum.
- No user data flows through it; claim privacy is unchanged.
- Gas: one map write per registration; queries paginated.

## Fit with cw-headstash
- New module `attestors.rs` next to `registration.rs`; new execute variants
  `RegisterAttestor`, `RemoveAttestor`, `PruneAttestors`; queries `Attestors`,
  `Attestor { valoper }`; one optional field on the campaign. Migration-free for
  new code ids; existing stored code ids unaffected.
- Tests: bad signature, wrong sender, unbonded validator, duplicate key, replay,
  set immutability after `SetEpochCampaign`, prune.

## Open questions for approval
- Should payout also require the attesting keys to be in `attestor_set_hash`
  (needs x/hashmerchant to expose signer keys to the contract)?
- Weight by stake or one-validator-one-vote inside the registry?
- Should `operator_pubkeys` (campaign/seal) also move on-chain?
