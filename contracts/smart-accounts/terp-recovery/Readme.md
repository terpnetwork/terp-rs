# terp-recovery

Break-glass / social recovery authenticator with **configurable rehash algorithms**.

## Challenge digest

```text
preimage = domain || 0x00 || chain_id || 0x00 || account || 0x00
           || authenticator_id || 0x00 || sign_mode_direct
digest   = rehash_N(hash_alg, preimage)
```

Default domain: `terp-recovery/break-glass/v1`

## Hash algorithms (`RecoveryHashAlg`)

| Variant | Notes |
|---------|--------|
| `Sha256` | default |
| `Sha512` | |
| `Keccak256` | Ethereum-family |
| `Sha3_256` | FIPS SHA3 |
| `Blake2s256` | |
| `Blake2b256` | BLAKE2b-512 truncated to 32 |
| `Sha256d` | double-SHA256 |

`rehash_rounds` (1..=64) applies the algorithm sequentially.

An optional `preimage_proof` on the auth payload must start its public inputs
with that challenge. Without the `zk-host` feature the proof arm fails closed.
Guardian approvals remain required.

## Approvals

1. **Ed25519** (preferred): guardian has `pubkey`; `signature` is 64-byte host `ed25519_verify` over `digest`.
2. **Address-only attestation** (MVP): `signature == rehash(hash_alg, guardian || 0x00 || digest)` when `allow_address_only_approvals`.

Auth payload must **echo** `hash_alg` and `rehash_rounds` matching on-chain config (alg confusion protection).

## Query

`Challenge { chain_id, account, authenticator_id, sign_mode_direct }` returns the digest clients must sign/attest.
