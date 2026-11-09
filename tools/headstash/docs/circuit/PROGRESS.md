# Headstash circuit progress

Product A is a clearnet claim. The proof must show ownership of an eligible secp256k1 key and a single nullifier for that note. The distribution root, denomination, value, recipient, nullifier, and note digest are public.

## Done

- [x] Poseidon-v1 distribution tree, depth 32. Leaf is `(epk_x, epk_y, nd, v, fdi, rseed_com)`. `rseed_com` hides the note's 32-byte string. Domain `poseidon-v1`.
- [x] Poseidon-v1 note commitment. Public `cmx` is the digest. The Pallas point is only there so the nullifier equation has a point to add.
- [x] Instance columns bound in the circuit: `anchor`, `nd`, `v`, `recp`, `nf`, `cmx`.
- [x] Ownership is in-circuit GLV: `epk = k1·G + k2·ψ(G)` with `k1 + k2·λ ≡ esk (mod n)`. `ψ(x, y) = (βx, y)`.
- [x] Soundness tests for the split (bounds, `k·G` via libsecp) and for the chip (MockProver accepts a real key, rejects a mismatched key).
- [x] Native release bench, 10 threads, K=18, before personal_sign: keygen 18.8s, prove 6.67s, verify 0.14s, proof 4704 bytes. `artifacts/snap-proof-bench.json`.
- [x] Ownership is personal_sign. `esk` is not a claim witness. K=19. Instance wire is 200 bytes.
- [x] Native release bench, 10 threads, personal_sign, K=19, zakura-halo2-proofs: keygen 69.1s, prove 17.6s, verify 1.27s, proof 4800 bytes. One sample after cw-orch stopped linking the zcash Halo2 crate. Prior personal-sign sample was keygen 73.5s, prove 21.1s, verify 0.57s, same proof. The prover was already zakura on both samples.
- [x] Spec rewritten to this circuit (`spec.md`).
- [x] Contract denom mask is byte 31. Claim recipient is the raw 32 bytes.

This ownership proof needs `esk` in the prover. That is fine for a host prover. It is the wrong shape for a wallet that only signs.

## Not this circuit

- [ ] MetaMask snap proving. The snap can hold the key. It cannot thread the prove, and it must not hand the key to the page.
- [ ] The fixed-base and variable-base scalar-mul challenge interfaces. Fixed-base is `[k]G` with the scalar as input (the pairing above). Variable-base is `[k]P`, which is one multiplication inside a signature check. Neither hashes a wallet message or derives a nullifier.

## Next

- [x] Offline signature message and nullifier, without `esk`. `claim_auth`: EIP-191 Keccak of `headstash-claim-v1:` plus the claim digest, and `claim_nullifier(epk_x, epk_y, nd, v, fdi)`.
- [x] `prove_ecdsa_verify` on the secp chip. `u1·G` is fixed-base GLV. `u2·Q` is variable-base GLV (`ψ(Q) = (βx, y)`). MockProver K=19 accepts a real `personal_sign`. The action circuit still proves `esk·G`. This gadget is not the claim path yet.
- [x] Hiding nullifier. `rho`, `psi`, and `rcm` are Poseidon PRFs of the note's 32-byte `rseed` only. `nk` mixes `esk` with that string. The public leaf stores `Poseidon(DST_RSEED, rseed)` and not the string. A free `nk` fails (`h_free_nk_fails`). A second `rseed` fails the published leaf (`h_rerolled_rseed_fails`). The honest claim passes (`h1_valid_claim`).
- [ ] The contract must set public `e = keccak(EIP-191(message)) mod n` and check the proof against that `e`. The hash stays out of the circuit on purpose.
