# Halo2 claim review

Reviewed from source. This pass does not change the Halo2 circuit, `cw-headstash`, or the 200-byte instance. Public destination and amount are the expected payout, not a defect. Halo2 and Flock are one statement for a benchmark, not two products.

## ECDSA already inside the proof

`Circuit::synthesize` in `crates/headstash/circuits/halo2/src/circuit.rs` calls `Secp256k1Chip::prove_ecdsa_verify` in `circuit/gadget/secp256k1_chip.rs`, in the `personal_sign` region. `Circuit` has no `esk` field. `synthesize` does not call `prove_key_pairing`. The note-commit region assigns a cell constrained to `pallas::Base::ZERO` under the label `"note commit does not take esk"`.

`prove_ecdsa_verify` loads `e`, `r`, `s`, `Q.x`, and `Q.y` with `load_private`. It never calls `constrain_instance`. The checks in the body are:

```text
s_inv = fq.div(one, s)                         // s⁻¹
u1    = fq.mul(e, s_inv)                       // "u1 = e/s"
u2    = fq.mul(r, s_inv)                       // "u2 = r/s"
r_g   = scalar_mul_glv(u1)                     // "u1·G"
r_q   = scalar_mul_glv_at(u2, Q)               // "u2·Q"
R     = add_point(r_g, r_q)                    // "R = u1G + u2Q"
enforce_carry_equal "R.x = m·n + r"
```

`m` is a bit that is 1 only when `R.x >= n`, so `R.x = m·n + r` is `R.x ≡ r (mod n)`. The function comment states the same equation: `R = (e·s⁻¹)·G + (r·s⁻¹)·Q`, and `R.x ≡ r (mod n)`.

The function returns `(Q.x native, Q.y native, e)`. `synthesize` feeds those coordinates to `distro_poseidon_gadget::derive_leaf_poseidon` and publishes only the Merkle root. `(r, s)` are not returned and are not copied to an instance row.

## Witnesses

`Q` is `Circuit.epkx` / `Circuit.epky`. `(r, s)` are `Circuit.sig_r` / `Circuit.sig_s`. All four are `Value`s loaded as advice. `esk` is not a witness. The Merkle path, position, `fdi`, and the `rseed` halves are advice.

`e` (`Circuit.sig_e`) is a witness and a public column. `synthesize` packs it with `pack_u128` and constrains the halves at `E_LO` and `E_HI`.

## Instance columns

One instance column, `Config.primary`. `Circuit::synthesize` constrains eight rows. `ProvingKey::build_and_write` writes footer `i = 8` with the comment `anchor, nd, v, recp, nf, cmx, challenge lo, challenge hi`. `Instance::to_halo2_instance` allocates `[[vesta::Scalar; 9]; 1]` and leaves index 8 at zero. That ninth scalar is not a `constrain_instance` row.

| Row | Name | Cell constrained in `synthesize` | What is public |
|---|---|---|---|
| 0 | `ANCHOR` | Poseidon distro root | Allocation root |
| 1 | `HS_ND` | `nd` | Denomination, `NoteDenom::to_fp` |
| 2 | `HS_V` | `v` as base | Amount |
| 3 | `RECP` | `recp` | Payout destination as `RecpAddr::to_fp` (`spec::recp_to_fp`, Poseidon of the two little-endian u128 halves). Not the eligible key. |
| 4 | `NF_OLD` | `derive_nullifier(...).extract_p()` | Nullifier |
| 5 | `CMX` | Poseidon note `cmx` | Note commitment |
| 6 | `E_LO` | low 128 bits of reduced `e` | Challenge low half. `to_halo2_instance` takes big-endian `e[16..32]`. |
| 7 | `E_HI` | high 128 bits of reduced `e` | Challenge high half. Big-endian `e[0..16]`. |

`RK_X` and `RK_Y` are comments at the old offsets. They are not rows.

`HS_ND`, `HS_V`, and `RECP` are the public payout. That is expected.

## 200-byte wire

`Instance::to_bytes` / `from_bytes` are 200 bytes. Order is not the row order. `nf` is before `recp` on the wire. `RECP` is before `NF_OLD` in the column. Do not rearrange this.

```text
anchor                 32
nd.to_fp().to_repr()   32
v little-endian         8
nf                     32
recp canonical         32
cmx                    32
e                      32
```

The signed prefix is `to_bytes()[..168]`, which is everything except `e`. `e` is the last 32 bytes.

`HeadstashInstances` carries the same fields. `HeadstashNote::verify_recp_posiedon_hash` compares `rr` to the 32 canonical `recp` bytes. The comment there says the wire stores canonical bytes, and the circuit binds `to_fp()` of those bytes to column `RECP`.

## The signed message binds the public payout

Yes, on the live claim path. It is not the Flock message.

`Circuit::attach_personal_sign` does:

```text
body = keccak256(instance.to_bytes()[..168])
(e, r, s) = sign_personal_claim(secret, &body)
```

`sign_personal_claim` sets `e = reduce_mod_n(claim_challenge(body))` and signs `Message::from_digest(claim_challenge(body))`. `claim_challenge` is `eip191_keccak(claim_signing_message(body))`. `claim_signing_message` is ASCII `headstash-claim-v1:` plus the lowercase hex of `body`. `CLAIM_PREFIX` is `"headstash-claim-v1:"`. EIP-191 is `keccak256("\x19Ethereum Signed Message:\n" || decimal length || message)`. `challenge_scalar_be` subtracts `n` once when the digest is not smaller than `n`. The scalar in the circuit is that reduced value. The wallet signs the unreduced digest. Those are the same ECDSA challenge.

The 168-byte preimage is `anchor || nd || v || nf || recp || cmx`. Changing the allocation root, the nullifier, the destination, or the amount changes `body`, the personal_sign string, and `e`. Denomination and `cmx` are in that preimage as well. `e` is not.

`process_headstash` repeats the check before `proof_instance_verify`:

```text
body = keccak256(&inst.to_bytes()[..168])
expect = challenge_scalar_be(&body)
reject unless inst.e == expect
```

`prove_ecdsa_verify` does not hash the other columns. It checks the equation for the witnessed `e`, and `synthesize` publishes that `e`. The binding of this `e` to this payout is `attach_personal_sign` plus `process_headstash`, not a gate. A caller that only runs `proof_instance_verify` does not run `challenge_scalar_be`. The live contract does.

`claim_signing_message`'s comment says `body` is "the Poseidon of the public claim". The live caller passes `keccak256` of the 168-byte prefix, not a Poseidon digest. The function itself only hex-encodes the 32 bytes it is given.

## Halo2 `e` is not the Flock message

`circuits/claim-flock/src/statement.rs` `private_sign_message` builds `headstash-private-claim-v1:` plus the hex of `root`, `nullifier`, and `payout_com`. `PRIVATE_SIGN_PREFIX` is `"headstash-private-claim-v1:"`. The comment says MetaMask applies EIP-191 on top. The body is the hex of three `[u32; 8]` words. It is not `keccak256` of the Halo2 prefix. `payout_com` is not Halo2 `cmx`. The prefix is different.

The Halo2 challenge `e` is not the challenge of `private_sign_message`.

`build_private` publishes the nullifier, the allocation root, and `payout_com`. Its comment says the amount, the destination, and the leaf key are witnesses, and that the circuit does not check secp256k1. That check is `prove_ecdsa_verify` on the Halo2 side and a missing Ligerito gate on the Flock side. Those are not the same public words. Halo2 already publishes denomination, amount, and recipient, and it already checks the ECDSA equation.

Flock's message binds a hiding payout commitment. It does not bind the public destination and amount. That is a leftover on the Flock message. Halo2's message does bind them.

## What a sidecar still learns

From the eight columns and the 200-byte wire, a sidecar learns:

- The allocation root (`ANCHOR`).
- The denomination (`HS_ND` and the wire bytes).
- The amount (`HS_V` and the `u64`).
- The payout destination: raw 32-byte `recp` on the wire and in `HeadstashNote.rr`, and `RecpAddr::to_fp` in column `RECP`.
- The nullifier (`NF_OLD`).
- The note commitment (`CMX`).
- The reduced challenge, split across `E_LO` and `E_HI`.

It can recompute `challenge_scalar_be(keccak256(wire[..168]))` and see that this `e` is the personal_sign challenge of this root, denomination, amount, nullifier, destination, and `cmx`. That says an eligible key authorized this payout. It does not say which key.

This review found nothing in the public instance that identifies the eligible key.

- No column is `Q`, `epk_x`, `epk_y`, `esk`, `r`, or `s`.
- Native `Q` from `prove_ecdsa_verify` goes into `derive_leaf_poseidon` (`tag, epk_x, epk_y, nd, v, fdi, rseed_com`). The instance gets the Merkle root, not the leaf. `epk`, `fdi`, and `rseed_com` stay advice.
- `bind_hiding_nullifier_inputs` sets `rho = rseed_lo`, `psi = rseed_hi`, `rcm = rseed_lo + rseed_hi`, and `nk = Poseidon(DST_HKDF, rseed_lo, rseed_hi)`. Its comment says the eligible key is not an input.
- `gadget::derive_nullifier` is `Extract_P(cm + [Poseidon(nk, rho) + psi] NullifierK)`. It does not take `Q`.
- `note_commit_poseidon` hashes `tag, nd, v, fdi, recp, esk = 0, rho, psi, rcm`. It does not take `Q`.

Publishing `(r, s)` next to `e` would recover `Q`. Those scalars are not columns.

`HS_ND` and `HS_V` are the denomination and amount inside the public leaf. `synthesize` says the distro tree is public and that binding `nd`, `v`, and `recp` is what lets a clearnet claim mint the opened allocation. A sidecar that also has the leaf list can filter leaves by denomination and amount. `fdi` and `rseed_com` are not on the instance, so that pair is not the leaf. The product lock treats those payout columns as expected. They are not a key column.

## Left to iterate

Not this pass. Do not rebuild Halo2 for these.

1. Same statement, different message. Halo2's `e` is `headstash-claim-v1:` plus the hex of `keccak256` of the 168-byte prefix (root, denomination, amount, nullifier, destination, `cmx`). Flock's `private_sign_message` is `headstash-private-claim-v1:` plus the hex of root, nullifier, and a hiding `payout_com`. The Flock string does not bind the public destination and amount. Halo2's string does.
2. Flock does not yet check the equation Halo2 checks. `build_private` does not constrain secp256k1, and it still hides amount and destination. The benchmark statement is the Halo2 one: public destination and amount, and `prove_ecdsa_verify`.
3. The hash that binds `e` is outside the circuit. `claim_auth` says the chain checks the challenge so Keccak stays out of the circuit. `process_headstash` does that check. A later verifier that only calls `proof_instance_verify` has to repeat `challenge_scalar_be`, or a proof whose public `e` signs some other challenge will verify. That is a call-site leftover, not a missing key column.
4. Stale layout test. `suite::test_zkvm_circuit_public_layout` still expects `K == 18`, `to_bytes().len() == 168`, and footer `i = 6`. Live `circuit::K` is 19, `Instance::to_bytes` is 200, and `build_and_write` writes `i = 8`. The `K` comment says one GLV multiplication fit in 18 and personal-sign verify does not. Do not shrink the instance to satisfy that test.
5. `process_headstash` doubles the first payout. `*cts.entry((nd, rr)).or_insert(claim.i.v) += claim.i.v` inserts `v` and then adds `v` again. One claim of amount `v` is enqueued as `2v`. This is contract accounting, not a circuit column.

## Not leftovers

- Public denomination, amount, and recipient.
- Public `e`. It is the challenge, not the key. `(r, s)` stay witnesses.
- `Q` as a witness that is also the Merkle leaf key. The proof shows some eligible key signed this payout. The instance does not say which key.