# terp-bls

BLS12-381 authenticator. Two variants:

| Variant | Public key | Signature | Hash |
|---------|------------|-----------|------|
| `min_pk` | G1, 48 bytes | G2, 96 bytes | hash-to-G2, POP ciphersuite |
| `min_sig` | G2, 96 bytes | G1, 48 bytes | hash-to-G1, POP ciphersuite |

The signed message is `sign_mode_direct`. `BlsAuthPayload.key_indices` selects registered keys in parallel with `signature_data.signatures`. Each signature is checked on its own. The number of distinct valid keys must meet `threshold`.
