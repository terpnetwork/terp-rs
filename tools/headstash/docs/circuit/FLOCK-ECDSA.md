# Flock in-circuit secp256k1 verify

This is the statement the Flock claim must constrain. It is not implemented.
No Rust in this note is a patch, and no timing below is a measurement.
Caracal7 (`src/workloads/ecdsa_k1.mojo`) is a walk to adapt. It is not a
port, and its public inputs are not ours.

## 1. Public words and witnesses

Public words, and only these as the claim statement:

| Word | What it is |
|---|---|
| allocation root | Merkle root of the opened leaf |
| nullifier | nullifier of that leaf's `rseed` |
| destination | payout address, five `u32` words (20 bytes) |
| amount | the leaf amount, one `u32` |

Witnesses:

| Witness | What it is |
|---|---|
| signature `(r, s)` | secp256k1 scalars of the `personal_sign` |
| pubkey `Q = (x, y)` | full affine point, 32-byte coordinates |
| leaf opening | `rseed`, denomination, piece index, leaf index, and the full 32-byte `Q.x` bound into the leaf |
| Merkle siblings | one sibling digest per level |

`e` is not a public word. It is the challenge derived inside the circuit
from those four public words. Do not publish `r`, `s`, `e`, or `Q`.
`esk` is not a witness. There is no `esk·G` pairing and no recovery byte
`v` in the statement.

Circuit constants that do not depend on the holder (Blake3 IV and flags,
`p`, `n`, `b`, `G`, `β`, `λ`, the lattice basis) may sit on public wires.
They are not claim words. A public wire that copies `r`, `s`, `e`, or
either coordinate of `Q` is the wrong statement.

Caracal's `EcdsaK1` publishes `(r, s, e, Q)`: `public_inputs` is
`public_bytes(r, s, e, q)`, and a short buffer is rejected with
"public inputs are r, s, e, x_Q, y_Q". That split is the wrong one for
this claim. Publishing `(r, s)` next to `e` lets a verifier recover the
eligible key. Publishing `Q` names it directly. This circuit keeps those
values in the witness and puts the mint data (root, nullifier,
destination, amount) in the clear, because the contract has to pay that
destination that amount once for that nullifier.

This is also not the private payout statement. That statement's public
words are root, nullifier, and a hiding `payout_com`, and its signed body
is `headstash-private-claim-v1:` over those three. Destination and amount
are witnesses there. They are public here.

The live Flock claim does not check this equation. It slots a Blake3 gate
and publishes a Blake3 image of the first 16 bytes of the key. The sidecar
recovers the key beside the proof. A Blake3 hash of the signature, or of
`Q`, is not this constraint. The leaf binding in this statement is the
full 32-byte `Q.x`, not that 16-byte prefix and not `key_com`.

## 2. Equation

Same check as Halo2 `Secp256k1Chip::prove_ecdsa_verify`
(`crates/headstash/circuits/halo2/src/circuit/gadget/secp256k1_chip.rs`).
Halo2 names the scalars `u1` and `u2`. This note uses `w` and `v` for the
same two products.

```text
s_inv = s^{-1} mod n
w     = e · s_inv mod n
v     = r · s_inv mod n
R     = w·G + v·Q
check R.x mod n = r
```

`0 < r, s < n`. `Q` is not the point at infinity and lies on the curve,
`y² = x³ + b` with `b = 7`, coordinates in the base field. Because
`p < 2n`, the check is the Halo2 one: `R.x = m·n + r` with `m ∈ {0, 1}`.
The equation accepts both `s` and `n − s`. Low-`s` is not an extra
constraint.

`e` is the challenge of a `personal_sign` (EIP-191) over a message that
binds the four public words. The envelope is the one
`personal_sign_hash` already uses:

```text
keccak256( "\x19Ethereum Signed Message:\n" || decimal_length || body )
```

`e` is that 32-byte digest as a big-endian integer, reduced modulo `n`.

The body this circuit hashes, and not `headstash-private-claim-v1` and
not `headstash-claim-v1`, is ASCII:

```text
headstash-flock-ecdsa-v1:
```

then the hex of the public words in order: eight `u32` for the root,
eight for the nullifier, five for the destination, one for the amount.
Each word is `{word:08x}` with no separators. MetaMask signs `body`.
The circuit hashes the envelope. A different destination or a different
amount is a different `e`, so the same `(r, s)` fails the equation inside
the circuit. Checking the string in a sidecar does not count.

The amount word in the signature is the amount word in the leaf. The
destination is not inside the leaf. It is bound only by this message, so
the hash has to be a constraint, not a comment.

## 3. What the frontend is today

Three pieces exist. None of them is this secp256k1 check.

`CircuitBuilder` (`crates/flock/crates/flock-core/src/circuit/builder.rs`)
is the claim frontend. The headstash claim slots `Blake3Gate` and nothing
else (`crates/headstash/circuits/claim-flock/src/lib.rs`, `build` and
`build_private`). The claim gate today is Blake3. The builder has no
secp256k1 gate, no foreign-field mod-`n` mul, and no Keccak permutation.
Hints are invisible to the statement: a wrong hint can only make the row
fail. Slopes and window points belong there, not on the public tape.

`binary_sumcheck` (`crates/flock/crates/flock-core/src/binary_sumcheck.rs`)
is an unwired 8-bit packed lookup. `SEGMENT_BITS = 8`, so each pair of
input segments is one of `2^{16}` patterns. The `b`-sum is a lookup plus
an accumulator instead of a field multiply per gate per bit. The module
is public from `flock-core` and is called only by its own tests. It is
not connected to `CircuitBuilder`, to the claim, or to a curve. It is
characteristic-2 machinery (Lagrange weights use `+` because subtraction
is addition in `GF(2^128)`). It is not a secp256k1 multiplier.

`crates/flock/benchmarks/binius64/keccak_permutations.rs` proves `N`
independent Keccak-f[1600] permutations on `binius_frontend::CircuitBuilder`
(`binius_circuits::keccak::permutation::Permutation`), default
`N_PERMUTATIONS = 4096`. There is no sponge, no padding, and no message.
That file is not secp256k1 and not Ligerito. It does not supply the
EIP-191 challenge to this circuit. Flock still needs its own constrained
Keccak if `e` is to be derived inside the statement. Pointing the witness
at that benchmark is not the constraint.

## 4. Caracal walk, then the F128 adaptation

Caracal is a hash-based SNARK over an F127-family field. The workload
runs a fixed mulmod chain (`fp_k1`, `ecurve`, `mulmod`). This frontend's
arithmetic is `GF(2^128)` (`flock_field::F128`). A native `F128` product
is not an integer product and not a product modulo `p` or `n`. Do not
drop Caracal `Op`s, `mulmod_statement`, or `circuit_public_data` onto
these wires. Integer adds, multiplies, and reductions modulo `p` and `n`
have to be emulated. The packed 8-bit lookup above is not that emulator.

The walk, as inspiration:

1. Invert `s` modulo `n`.
2. GLV-split both scalars. For `k ∈ {e·s⁻¹, r·s⁻¹}`,

   ```text
   k = k1 + k2·λ  (mod n)
   ```

   with both halves under `2^128`. `φ(x, y) = (β·x, y)` on the base field,
   and `φ(P) = λ·P` on the group.
3. The four halves are `split(r·s⁻¹)` on the bases `Q` and `φ(Q)`, and
   `split(e·s⁻¹)` on the bases `G` and `φ(G)`. Caracal's order is
   `(k1, k2)` of `r·s⁻¹` on `(Q, φ(Q))`, then `(k1, k2)` of `e·s⁻¹` on
   `(G, φ(G))`.
4. `WINDOWS = 22 = ceil(128 / 6)`. Each half is 22 digits of 6 bits
   (`22 · 6 = 132 ≥ 128`). Window `w` is the Straus-Shamir point
   `P_w = Σ d_w,i · T_i` over those four bases, from four small window
   tables. The addition chain sums the windows back to `R`.

Caracal's `walk` then calls `check_inputs` on `(r, s, e, Q)` and, when
live, publishes the factor values. Each window point is a public point
in that schedule, and the blinding point is
`blinding(public_bytes(r, s, e, Q))`. None of that publication carries
over. Here `r`, `s`, `e`, and `Q` stay witnesses. Window points, digits,
GLV halves, and addition slopes are hints. The constraints recompute
`e` from the four public words, check the GLV relation
`k = k1 + k2·λ (mod n)` with both magnitudes under `2^128`, check the
window sum, and check `R.x mod n = r`. A hint is not a check.

Halo2's chip reaches the same relation with
`secp256k1_scalar_split_lambda`: it precomputes
`round(2^384 · b2 / n)` and `round(2^384 · (−b1) / n)` (`GLV_G1_HEX`,
`GLV_G2_HEX` in the chip) and folds the signs into the base points.
Those two 384-bit round-div constants are an implementation of the
split, not a second lattice. The lattice this circuit names is the one
in the next section. Either split is acceptable only if the constrained
relation is `k = k1 + k2·λ (mod n)` and both halves are under `2^128`.

`φ` is a base-field multiply by the constant `β`, then the same `y`.
It is not a second scalar multiply.

## 5. Curve constants

Read from Caracal `secp256k1()` and `Glv`, and checked against the Halo2
chip where that chip has the same constant. Hex is big-endian. Do not
substitute a guessed endomorphism.

| Name | Value |
|---|---|
| `p` | `fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f` |
| `n` | `fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141` |
| `b` | `7` |
| `G.x` | `79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798` |
| `G.y` | `483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8` |
| `β` | `7ae96a2b657c07106e64479eac3434e99cf0497512f58995c1396c28719501ee` |
| `λ` | `5363ad4cc05c30e0a5261c028812645a122e22ea20816678df02967c1b23bd72` |
| `a1` | `3086d221a7d46bcde86c90e49284eb15` |
| `b1` | `−e4437ed6010e88286f547fa90abfe4c3` |
| `a2` | `114ca50f7a8e2f3f657c1108d9d44cfd8` |
| `b2` | `3086d221a7d46bcde86c90e49284eb15` |

`β` and `λ` match the chip's `GLV_BETA_HEX` and `GLV_LAMBDA_HEX`.
`−b1` matches the chip's `GLV_MINUS_B1_HEX`. `a1 = b2`. The chip does
not store `a2`; `a2` is the Caracal / libsecp256k1 lattice value, not a
value derived here. Caracal's split is

```text
c1 = round(b2 · k / n)
c2 = round((−b1) · k / n)
k1 = k − c1·a1 − c2·a2
k2 = −c1·b1 − c2·b2
```

with the signs as stored (`b1` is already the negative lattice entry).

## 6. In-circuit checks

All of these are rows of the Flock statement. Failing any one of them
rejects the proof. None of them is a Blake3 compression of `(r, s)` or
of `Q`.

1. The leaf opens to the public allocation root under the witnessed
   siblings. The leaf binds the public amount and the full `Q.x`.
2. The public nullifier is the nullifier of the witnessed `rseed`.
3. `e` is Keccak of the EIP-191 envelope of the body in section 2, then
   reduced modulo `n`. The body is the public root, nullifier,
   destination, and amount.
4. `Q` is on-curve, and `(r, s, e)` are in range.
5. `R = w·G + v·Q` through the four GLV halves and the 22-window
   Straus-Shamir sum, and `R.x mod n = r`.

The Merkle index may be part of the leaf-opening witness, or it may be
baked into the circuit shape the way the Blake3 claim bakes path bits
today. It is not a fifth public word. Fixture depth is a legal shape for
the tests. Production depth is not a number this document claims a cost
for. Do not quote a depth-16 timing file, including any earlier Blake3
claim bench, as the cost of this circuit. This circuit has not been built
and has not been timed.

## 7. Test plan

Fixture depth is allowed. Do not report a depth-16 proof size or a
depth-16 prove time as the result of these tests.

1. Satisfying witness verifies. A real `personal_sign` under `Q` of the
   section 2 body, a leaf that binds that `Q.x` and the public amount,
   and siblings that open to the public root. `esk` is absent from the
   witness.
2. The same `(r, s)` under a different destination fails inside the
   circuit. The Merkle opening is unchanged. The failure is the equation
   (or the hash that feeds it), not a check outside the proof.
3. A different amount fails inside the circuit. Changing only the public
   amount breaks the signed message and the leaf binding.
4. A wrong public key fails. `(r, s)` produced by `Q` does not verify
   under `Q' ≠ Q`, and a `Q'` that does verify does not open the leaf.
5. `r`, `s`, and `Q` are absent from the public words. So is `e`.
   Destination and amount are present, and they are the destination and
   the amount the leaf and the signed body used.
