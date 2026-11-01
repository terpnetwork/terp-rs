# 03 — Components

Each section is one scoping step from `02-scoping.md`: the argument a
reviewer can check by hand, the under-constraint that was looked for, the
Lean block that already exists, and what is still missing. Line numbers are
the production `PIN = true` path unless a vk_id 2 contrast is named.

The chip sources live in zakura, outside this worktree:

- Poseidon chip: `/Users/returniflost/abstract/terp-core/crates/zakura-common/crates/halo2_gadgets/src/poseidon/pow5.rs`
- Poseidon spec: `.../crates/halo2_poseidon/src/lib.rs` and `p128pow5t3.rs`
- Swap chip: `.../crates/halo2_gadgets/src/utilities/cond_swap.rs`
- Boolean polynomial: `.../crates/halo2_gadgets/src/utilities.rs:169-239`

## 1. Poseidon permutation and sponge

### Argument

The note circuit does not reimplement Poseidon. `poseidon::<L>`
(`note_membership.rs:206-215`) builds `PoseidonHash<_, _, P128Pow5T3,
ConstantLength<L>, 3, 2>` on one `Pow5Chip` configured at
`note_membership.rs:125-131`.

The permutation spec is the Zcash Pasta instance: `R_F = 8`, `R_P = 56`,
width 3, rate 2, S-box `x ↦ x^5`
(`p128pow5t3.rs:8-24`). The chip requires both round counts even and lays
out one full round per row and two partial rounds per row
(`pow5.rs:64-68`, `pow5.rs:92-158`). Partial rows witness `partial_sbox`
and use `MDS` and `MDS_INV` so the middle state of the pair is not stored.
That rewrite is the standard halo2_gadgets optimisation; it is correct if
and only if those matrices are inverses and the round constants in `rc_a` /
`rc_b` are the spec constants. Both are fixed columns, assigned by the
chip, not advice the prover picks.

The sponge is `ConstantLength<L>` (`halo2_poseidon/src/lib.rs:385-412`):

- initial capacity = `L * 2^64` (output length hard-coded to 1),
- message padded with zeroes to a multiple of the rate,
- each block is added into the rate and the capacity is unchanged,
- the squeezed output is the first rate element after the last permutation.

The pad-and-add gate is `initial + input - output = 0` on the rate and
`initial_capacity - output_capacity = 0` (`pow5.rs:160-184`). Padding words
are `assign_fixed` on `rc_b` and then `constrain_equal`'d to the advice
input (`pow5.rs:334-354`). That is the post-fix shape of halo2 PR 646
(https://github.com/zcash/halo2/pull/646): a padding word is a fixed cell,
not a `copy_advice` of an `assign_fixed` value the prover treats as
unknown. The empty-message panic (halo2 PR 948) does not apply: every call
here has `L ∈ {2, 3, 4}`.

Domain tags are 32-byte little-endian ASCII, high bytes zero, parsed with
`from_repr` (`distro_poseidon.rs:165-175`). The four strings in this
circuit are at most 22 bytes, so they sit below `2^176` and are canonical.

### Under-constraint looked for

| Risk | Result |
| --- | --- |
| Prover chooses the tag (vk_id 2, finding N1) | Closed for this key. `fixed::<true>` calls `constrain_constant` (`note_membership.rs:198-199`). Test `pinned_tags_reject_a_forged_nullifier` (`note_membership.rs:566-585`) shows the unpinned circuit accepts a forged nullifier tag and the pinned circuit rejects it. |
| Prover chooses the layer index and slides a sibling to another level | Closed. `lc` is `constrain_constant(Fp::from(layer))` inside the loop (`note_membership.rs:163`). |
| Prover uses a different capacity and collides `L=3` with `L=4` | The capacity cell is `assign_advice_from_constant(D::initial_capacity_element())` (`pow5.rs:278-293`). It is not witness input. |
| Padding word left free on odd-length inputs (`L=3`) | The padding cell is fixed, then equality-constrained to the absorbed advice (`pow5.rs:334-354`). |
| `Pow5Config.secure_mds` unimplemented (`p128pow5t3.rs:30-32`) | Not on the synthesize path. `constants()` returns the precomputed MDS. A proof must still show that matrix matches the Poseidon spec, which is an equality check on the constant table, not a runtime call. |

### Lean

Ironwood already has the value-level hash and the chip's round structure:

- `Zcash/Circuits/Poseidon/Pow5.lean`, `Pow5Theorems.lean`, `Rounds.lean`, `Constants.lean`
- `Zcash/Circuits/Poseidon/Hash.lean` `ConstantLength.value`, generic in `L`
- `Zcash/Circuits/Poseidon/SpongeTheorems.lean` `AddInput.Spec` (rate addition, capacity fixed)

### Gap

No lemma yet says the note circuit's four call sites are
`ConstantLength.value` at `L = 2, 3, 4` on **these** round constants. Batch
0 diffs zakura `pow5.rs` against `halo2_gadgets-0.5.0` (the revision
Ironwood's comments name). If the polynomials match, batch 2 is an import,
not a new proof. The partial-round `MDS_INV` rewrite is the part that must
be in that diff: it is the only place a transcription error stays silent
and still accepts honest witnesses.

## 2. Merkle path selector and swap

### Argument

For each layer `ℓ ∈ 0..16` the circuit witnesses one sibling and one bit,
swaps, then hashes (`note_membership.rs:157-165`):

```text
bit_ℓ = 1    ⇔    (position >> ℓ) & 1 = 1          -- host encoding
(L, R) = bit_ℓ = 1 ? (sibling, node) : (node, sibling)
node' = Poseidon_L4(tag_crh, Fp(ℓ), L, R)
```

The bit is not decomposed from a range-checked integer. Seventeen
independent booleans **are** a 17-bit index. High bits of a `u32` position
cannot affect the root, because the loop never reads them. `prove_note`
rejects `position >= 2^17` on the host (`note_membership.rs:299-301`); the
circuit does not need that check for soundness.

The swap gate (`cond_swap.rs:256-284`), selector `q_swap`, all at the
current row:

```text
a' - (swap·b + (1-swap)·a) = 0
b' - (swap·a + (1-swap)·b) = 0
swap · (1 - swap) = 0
```

`bool_check` is `range_check(v, 2)`, and `range_check` folds
`v * (1 - v) * (2 - v) * ...` (`utilities.rs:169-171`, `utilities.rs:236-239`).
For range 2 the product is exactly `v * (1 - v)`. `ternary(a, b, c)` is
`a·b + (1-a)·c` (`utilities.rs:177-179`). With the boolean constraint, the
pair `(a', b')` is a permutation of `(a, b)`, not a mix.

The running node is `copy_advice`'d into column `a` (`cond_swap.rs:100`).
The sibling is fresh advice (`cond_swap.rs:103`). That matches the
`CondSwapInstructions` contract: the second element is a witnessed value,
not an existing cell (`cond_swap.rs:21-26`).

The contract root uses the same rule. `append_leaf_with` hashes a missing
right child as zero at every level up to the stored depth
(`registration.rs:905-917`). `poseidon_distro_crh` is
`ConstantLength<4>(tag_crh, Fp(layer), left, right)`
(`distro_poseidon.rs:232-242`). Host `note_root` (`note_membership.rs:222-231`)
and the circuit loop call that same function. A clear bit keeps the node
on the left.

### Under-constraint looked for

| Risk | Result |
| --- | --- |
| Non-boolean swap mixes left and right and forges a path | Closed by `swap·(1-swap) = 0` on every layer. |
| Swaps at different layers use inconsistent bits | Not a bug. Any 17 booleans are a legal index. Forging a leaf still requires the CRH preimage. |
| Empty slot opened as a leaf | The missing sibling is the field zero, and the parent is `CRH(layer, real, 0)`, not `CRH` of a hashed empty leaf. Opening the other child at that node requires `CRH(layer, 0, forged) = CRH(layer, real, 0)`. Under collision resistance that forces `forged = 0` and `real = 0`. A real note leaf is a Poseidon image, not zero. Higher empty subtrees require a preimage of zero under the remaining CRHs. |
| Depth confusion with vk_id 2 (16 levels) | This key always runs 17 iterations (`NOTE_DEPTH`, `note_membership.rs:40`). A 16-level root is a different field element. |
| Second preimage across leaf and internal node | Leaf is `L=3` with tag `terp-hs-note-leaf-v2`. Internal is `L=4` with tag `terp-hs-distro-crh-v1` and an explicit layer. Both the length (capacity) and the tag differ. |

### Lean

`Zcash.Circuits.CondSwap.SwapSpec` in
https://github.com/zcash/ironwood/blob/main/Zcash/Circuits/Utilities/CondSwap.lean
is this gate, including the boolean clause, and the file's `soundness`
proof is filled. Ironwood's comment cites `halo2_gadgets-0.5.0`
`cond_swap.rs:256-284`, the same gate text as zakura's `cond_swap.rs:256-284`.

There is no 17-step Merkle composition in Ironwood. Orchard's Merkle chip
is Sinsemilla. Do not instantiate it.

### Gap

A lemma `merkleFold_sound` quantifying over 17 `SwapSpec`s and 17
`ConstantLength 4` hashes, plus the statement that the contract's
`append_leaf_with` root equals `merkleFold` on the stored leaves with zero
siblings in the unoccupied positions. The second half is a contract
function, batch 6. It is ordinary induction on the frontier, not a new
cryptographic argument.

## 3. Nullifier derivation

### Argument

```153:153:tools/headstash/circuits/halo2/src/note_membership.rs
        let nullifier = poseidon(&mut layouter, &config, "nullifier", [nf_tag, secret, ctx.clone()])?;
```

The tag cell is the pinned constant `personalization_to_fp("terp-hs-nf-v1")`
(`note_membership.rs:47`, `note_membership.rs:152`). The secret cell is the
same cell that was absorbed into `key_com` (`secret.clone()` at line 151,
then moved into the nullifier). The ctx cell is the same cell later
constrained to instance row 0 (`ctx.clone()` at line 153, and `ctx` in the
`public` array at line 167).

So one satisfying assignment gives a single secret that opens both the
leaf (via `key_com`) and the nullifier, and the nullifier is bound to the
public ctx. The nullifier does not depend on the root, the position, or
the amount. The contract comment states the intended consequence: one
spend per note across every anchor (`direct.rs:11-13`).

Uniqueness across notes is not a gate. It is injectivity of
`secret ↦ Poseidon_L2(tag_keycom, secret)` together with the contract
rejecting a repeated `key_com` inside a list and in the global `KEYS` map
(`notes.rs:305-315`, `registration.rs:584-587`). Two notes that somehow
shared a secret would share a nullifier and only one could be paid. That
is a liveness failure for the holder, not a second payment.

The contract stores the nullifier bytes the proof was verified against,
not a nullifier it recomputes (`direct.rs:220-225`, `direct.rs:242`,
`headstash.rs:289-299`). Recomputation would be redundant if the relation
holds, and it is a useful independent check only after the tag string is
agreed. Today the denominations spec and the circuit disagree on that
string. See finding F2.

### Under-constraint looked for

| Risk | Result |
| --- | --- |
| Free nullifier tag, unlimited spends of one secret | Closed on this key. Open on `NoteCircuitImpl<false>` and on vk_id 2. Finding F3. |
| Nullifier not bound to the spent leaf | Bound through the shared `secret` cell. A different secret is a different `key_com` and does not open the leaf. |
| Same secret, two denominations, two payouts | Prevented by unique `key_com` at registration, assuming Poseidon is collision-resistant. Not prevented inside the circuit. |
| Cross-contract replay | `ctx` is inside the hash. `direct_ctx` mixes chain id and contract address (`direct.rs:49-61`). A second funded contract with the same leaves and a different ctx has a different nullifier, so the first spend does not block the second. That is deployment policy. The circuit cannot see other contracts. |
| Confusion with `claim_auth::claim_nullifier` | That function is a different hash: `ConstantLength<6>` under tag `terp-hs-nullifier-v1` of `(epk_x, epk_y, nd, v, fdi)` (`claim_auth.rs:27`, `claim_auth.rs:367-383`). It is the K=19 distribution-leaf nullifier. It is not called by `NoteMembershipCircuit`. |

### Lean

`poseidonConstLen 3 ![tagNf, secret, ctx]` once component 1 is a lemma.
The "same cell" fact is a copy constraint, which Clean already tracks as
equality-enabled columns (`note_membership.rs:119-121` enables equality on
all six advice columns).

### Gap

The composition lemma has to name the shared `secret` and shared `ctx`
cells. A proof that only says "there exist some secret1 and secret2" is
the wrong statement: it would not tie the opened leaf to the published
nullifier. The illustrative statement in `02-scoping.md` uses one `secret`.

## 4. Range and boolean checks

### Argument

The only boolean constraint in `NoteMembershipCircuit` is the swap flag in
component 2. There is no lookup, no running-sum decomposition, and no
range check on:

- `secret` (`note_membership.rs:141`),
- `amount` (`note_membership.rs:143`, `Base::from` of a `u64` on the honest path only),
- siblings (`note_membership.rs:159`),
- `dest_lo` / `dest_hi` (`note_membership.rs:144-145`).

That is sufficient for the relation in `02-scoping.md`. Each of those
values is a field element. The hashes are defined on field elements.
Range is needed only where a field element is later read as an integer
with a narrower modulus (a secp scalar, a bit, a limb). The swap bit is
the one place that happens.

`amount` is a `u64` on the wire because the contract builds the instance
limb (`direct.rs:79-83`) and the verifier checks equality with the field
element inside the leaf. Pallas order is about `2^254`, so every `u64` has
a unique 32-byte encoding. A witness whose amount cell is not that `u64`
fails verification against the limb the contract sends. An in-circuit
`amount < 2^64` check would not change the set of proofs the contract
accepts.

`prove_note` refuses a non-denomination before proving
(`note_membership.rs:296-298`). That is a prover convenience. The circuit
accepts any field element that was actually inserted as a leaf. Registration
only inserts `DENOMS[class]` (`notes.rs:163-168`, `notes.rs:383-394`).
Claim rejects an amount outside `DENOMS_UTERP` and outside the campaign
table (`direct.rs:208-214`). Both checks sit outside the relation. Finding
F4.

### Under-constraint looked for

| Risk | Result |
| --- | --- |
| Missing boolean on the Merkle bit | Present, as `swap·(1-swap)=0`. |
| Missing range on amount lets the prover add the field modulus and double-pay | The instance encoding is the canonical 32-byte representative. `p` does not fit in 32 bytes as a second encoding of a small `u64`. |
| `canonical32` admits a non-canonical root or nullifier | For Pallas, `p = 2^254 + c` with `c ≈ 2^125`. Any 32-byte string with byte 31 ≤ `0x3f` is `< 2^254 < p`, hence canonical. The check (`direct.rs:97-101`) does not let two byte strings decode to one field element. It does reject the negligible set of field elements `≥ 2^254` (byte 31 = `0x40`). Finding F5, completeness, not soundness. |
| vk_id 2 free constants, which are a kind of missing range | Fixed on this key only. |

### Lean

Ironwood `LookupRangeCheck` and `DecomposeRunningSum` are the wrong tools
for this circuit. The boolean lemma is the one inside `SwapSpec`. A
separate `amount ∈ DENOMS` lemma belongs to the contract model, as a
finite check on a `u64`, and should not be stuffed into the circuit
statement.

### Gap

None inside the circuit, once `SwapSpec` is instantiated. The gap is
documentary: a reader of the K=19 chip will expect range checks everywhere,
and this circuit is sound without them. The plan should say that explicitly
so batch 8's CRT work is not back-ported into vk_id 3.

## 5. Public-input binding

### Argument

One instance column, equality enabled (`note_membership.rs:109-110`).
Six cells, in order, rows 0..5 (`note_membership.rs:167-170`):

```text
0 ctx
1 root          -- the advice `node` after the loop
2 nullifier
3 dest_lo
4 dest_hi
5 amount
```

`constrain_instance` is a copy constraint between the advice cell and
`instance` at that row. There is no seventh row. `NOTE_INSTANCES` is 6
and the footer records that count (`note_membership.rs:44`,
`note_membership.rs:361-366`).

The host encoding used by tests and by the contract is the concatenation
of the six `to_repr()` values, 192 bytes, little-endian
(`note_membership.rs:476-479`, `direct.rs:7-14`, `direct.rs:85-95`).
`pack_destination` in the circuit crate (`no_ecdsa.rs:277-292`) and in the
contract (`direct.rs:64-77`) copy the same slices: at most 31 bytes into
`lo`, the remainder into `hi`, both buffers zero-initialised, reject past
62 bytes. The circuit crate then `from_repr`s them. For a bech32 address
the bytes are ASCII, `lo[31]` and `hi[31]` stay zero, and both integers
are `< 2^248 < p`, so `from_repr` succeeds and the bytes round-trip.

`claim_note` passes `info.sender` as the destination string
(`direct.rs:240`). A proof built for a different address fails the copy
constraint. A copied proof pays the same sender or fails. That is the
front-running property in the security-review addendum N4; the mechanism
is this instance binding plus the contract not trusting the user's limbs.

`note_canonical_body` (`note_membership.rs:234-248`) is a debug string.
`claim_note` does not read it. It is not part of the relation. Finding F7.

### Under-constraint looked for

| Risk | Result |
| --- | --- |
| Instance vector longer or shorter than the constrained rows | The footer says 6. The contract sends 192 bytes. The host parser that turns those bytes into six field elements was not in this worktree. Batch 6 states the assumption: the verifier feeds exactly these six canonical elements and rejects any other length. See questions.md. |
| Prover publishes a root other than the folded node | Row 1 is the `node` cell, not a second advice. |
| ctx in the nullifier differs from ctx in row 0 | Same `AssignedCell`. |
| amount in the leaf differs from row 5 | Same `AssignedCell` (`amount.clone()` into the leaf at line 155, `amount` in `public` at line 167). |
| User-supplied instance bytes | `ClaimNote` does not take them. `instances()` rebuilds from campaign state and `info.sender`. A future entry point that forwarded the user's buffer would bypass destination and amount policy. Finding F4. |

### Lean

Clean's instance-column copy is the whole lemma: the six extracted field
elements equal the six advice values named in the relation. Ironwood's
Action circuit has its own public-input layout (`Zcash/Circuits/Action/`);
the layout does not transfer, the copy-constraint lemma does.

### Gap

A one-page fixture: the 192-byte vector from `note_host_proof`
(`note_membership.rs:467-491`) next to the six `Fp` values, checked in Lean
by reflexivity on the bytes. That fixture is also the regression test for
the contract's `instances()`. Generating it is a K=11 prove. Not run here.

## 6. Destination binding

### Argument

Destination is not committed in the leaf. The leaf is
`Poseidon(tag_leaf, key_com, amount)` (`note_membership.rs:1-4`,
`note_membership.rs:155`). Anyone who knows `secret` can prove the note
against any `dest_lo, dest_hi`. The circuit then forces those limbs to
equal the public rows. The contract forces the public rows to equal
`pack_destination(info.sender)`.

The registration signature binds a `reward_addr`
(`notes.rs:218-225`, checked at `notes.rs:125-133` and `notes.rs:318`).
That address is handed to the optional registration hook
(`notes.rs:409-422`). It is not an input to `claim_note` and it is not a
cell in the circuit. The spend capability is knowledge of `secret`, not
possession of the key that signed the note list.

This is the binding the product asked for: the transaction sender is paid,
and a relayer who changes the sender invalidates the proof. It is not a
binding from the eligible EVM or Solana key to the payout address. The
eligible key authorises insertion of `key_com` into the tree. After that,
the secret spends the note.

### Under-constraint looked for

| Risk | Result |
| --- | --- |
| Circuit accepts malformed destination limbs | Yes. Any two field elements work. The contract never asks the circuit to parse them. Sound for `claim_note`. Unsound for a verifier that decodes rows 3–4 as an address and trusts the string. Finding F4. |
| Two addresses pack to the same limbs | `pack_destination` is injective on strings of length ≤ 62: it is a zero-padded byte copy with a fixed split at 31 (`direct.rs:64-77`). Bech32 payees used here are shorter than 62. |
| Packed limbs differ between contract and prover | The two functions are the same copy. The circuit side additionally rejects a non-canonical encoding (`no_ecdsa.rs:289-290`), which the zero-padding prevents for these strings. |
| Payout to `reward_addr` instead of sender | Not what the code does. An FV statement that says so is false. Do not prove it. |
| Destination substituted inside the Merkle leaf | Impossible: the leaf hash does not take the destination cells. |

### Lean

No existing block. The lemma is not cryptographic. It is:

```lean
-- Contract side, after NoteRelation.
def PaysSender (pub : NotePublic) (sender : ByteArray) : Prop :=
  pub.destLo = packLo sender ∧ pub.destHi = packHi sender
  ∧ sender.size ≤ 62
```

composed with "the bytes passed to `proof_instance_verify` are
`instances(campaign.ctx, root, nf, sender, amount)`". That second sentence
is a statement about `direct.rs:85-95` and `direct.rs:224`, which a Lean
proof can carry only after those functions are modelled. Batch 6.

### Gap

The circuit theorem stops at "rows 3 and 4 equal the witness limbs". The
sentence "those limbs are the sender" is a separate theorem about the
contract. Shipping only the first sentence in a blog post would overclaim.
`04-fv-plan.md` keeps them in different batches on purpose.

## Composition

The hand proof of the top statement, which stage 4 of the skill would turn
into `have`s:

1. From the six instance copies, the public `ctx`, `root`, `nullifier`, and
   `amount` are the advice cells that entered the hashes (component 5).
2. From `SwapSpec` at each layer, the bits are boolean and the children
   are a permutation of `(node, sibling)` (component 2).
3. From `ConstantLength` soundness at `L = 4`, each parent equals the spec
   Poseidon of `(tag_crh, layer, left, right)` with the pinned tag and the
   pinned layer (components 1 and 2).
4. By induction on the 17 layers, `root` is `merkleFold leaf bits siblings`.
5. From `ConstantLength` at `L = 2` and `L = 3`, `key_com` and `nullifier`
   and `leaf` match the spec hashes on one shared `secret` (components 1
   and 3).
6. Collision resistance is not proved. It is the assumption that makes
   "the opened leaf is one that was appended" and "one secret, one
   nullifier" computational rather than information-theoretic. State it as
   an explicit hypothesis, the way Ironwood states a Sinsemilla break, and
   do not discharge it with `sorry`.

Steps 1–5 are the circuit. The contract obligations (denomination set,
anchor set, nullifier set, sender packing, `direct_ctx`) are applied to
the extracted `NotePublic` afterwards. They are true of `claim_note` as
reviewed; they are not gates.
