# 05 — Soundness findings

Reviewed from source on 2026-10-31. Nothing in this list was patched.
The production key is `NoteMembershipCircuit` (`PIN = true`). No critical
defect was found in that key. The one high finding is the predecessor key,
which new campaigns already refuse.

Severity is about claim integrity: can a proof pay the wrong amount, pay
twice, or pay someone who does not know the secret.

| ID | Sev | Applies to | Title |
| --- | --- | --- | --- |
| F1 | high | vk_id 2, not vk_id 3 | Tags and layer indices are unconstrained advice |
| F2 | low | spec vs vk_id 3 | Denominations spec names the wrong nullifier tag |
| F3 | low | same module as vk_id 3 | Unpinned circuit type is still constructable |
| F4 | medium | vk_id 3 boundary | Destination and denomination are not gates |
| F5 | info | contract parser | `canonical32` rejects a negligible slice of field elements |
| F6 | info | vk_id 3 | Nullifier ignores amount, position, and root |
| F7 | info | vk_id 3 | `note_canonical_body` is not a binding |
| F8 | info | K=19 lab, not this key | Foreign-field chip was unsound; replacement not re-reviewed |
| F9 | info | registration | Eligible-key signature is outside the circuit |

## F1 — high — vk_id 2 tags are free advice

`MembershipCircuit::synthesize` loads every Poseidon tag with `load`, which
is `assign_advice` and nothing else.

```175:175:tools/headstash/circuits/halo2/src/no_ecdsa.rs
    let tag = load(layouter, config.advice[5], label, Value::known(tag))?;
```

The same pattern is in `hash3` (`no_ecdsa.rs:184-198`) and in the Merkle
loop (`no_ecdsa.rs:217-228`). A prover may substitute another nullifier tag
and obtain another nullifier for the same secret. The security review
states the consequence: unlimited claims of one allocation
(`tools/headstash/docs/security-review-2026-10-26.md:128-135`).

vk_id 3 closes this. `fixed::<true>` calls `constrain_constant`
(`note_membership.rs:198-199`) on the key-commitment tag, the nullifier
tag, the leaf tag, the CRH tag, and each layer index
(`note_membership.rs:150-163`). `pinned_tags_reject_a_forged_nullifier`
(`note_membership.rs:566-585`) distinguishes the two.

New campaigns cannot select vk_id 2:

```120:124:contracts/headstash/cw-headstash/src/epoch.rs
    if existing_campaign.is_none() && campaign.vk_id == crate::notes::RETIRED_VK_ID {
        // N1: vk_id 2 stays verifiable for the campaign that already uses it, but no
        // new campaign may be created under it. New membership campaigns use vk_id 3.
        return Err(StdError::msg("vk id 2 is retired for new campaigns"));
    }
```

An already-stored vk_id 2 campaign is still verifiable. Suggested fix,
not applied: reject `proof_instance_verify` for `RETIRED_VK_ID` on any
path that moves funds, and migrate or sweep that campaign under the vk_id
3 key. Do not "fix" it by reusing the vk_id 2 verifying key with pinned
constants; pinning changes the key.

## F2 — low — spec nullifier string is not the circuit tag

The circuit constant is `terp-hs-nf-v1`:

```46:48:tools/headstash/circuits/halo2/src/note_membership.rs
pub const TAG_KEYCOM: &str = "terp-hs-keycom-v1";
pub const TAG_NULLIFIER: &str = "terp-hs-nf-v1";
pub const TAG_NOTE_LEAF: &str = "terp-hs-note-leaf-v2";
```

`tools/headstash/docs/headstash-denominations-spec.md:66` writes
`Poseidon("terp-hs-nullifier-v1", secret, ctx)`. That other string is the
tag of a different hash, `ConstantLength<6>` over an ephemeral key and a
distribution leaf (`claim_auth.rs:27`, `claim_auth.rs:367-383`).

`REGISTRATION-TREE.md:24` has the vk_id 2 string right (`terp-hs-nf-v1`)
and is stale on depth and on the leaf tag (it still says depth 16 and
`terp-hs-claim-leaf-v1`).

The contract does not recompute the nullifier. It verifies the proof and
stores the public limb (`direct.rs:220-225`, `direct.rs:242`). A client
that follows the denominations spec produces a proof the contract rejects,
or a second system that deduplicates with the spec formula will not see
spends. Suggested fix: change the spec sentence to
`Poseidon("terp-hs-nf-v1", secret, ctx)` with `ConstantLength<3>`, and
point `terp-hs-nullifier-v1` at `claim_auth::claim_nullifier` only.

## F3 — low — `NoteCircuitImpl<false>` sits next to the production type

```68:83:tools/headstash/circuits/halo2/src/note_membership.rs
/// `PIN = true` is the production circuit. `PIN = false` reproduces the vk_id 2
/// shape (free tag cells) and exists only so tests can demonstrate finding N1.
pub struct NoteCircuitImpl<const PIN: bool> {
```

`note_circuit_blob` keygens `NoteMembershipCircuit`, the `true`
instantiation (`note_membership.rs:266` via `note_vk`, and the type alias
at line 83). A caller who constructs `NoteCircuitImpl::<false>` and
registers that verifying key gets finding F1 back. Suggested fix: gate the
`false` impl and `pinned_tags_reject_a_forged_nullifier` behind `cfg(test)`.
The production blob path should not see the type.

## F4 — medium — payout policy is the contract, not the circuit

Rows 3, 4, and 5 are copied to the instance and, for the amount, into the
leaf. They are not otherwise constrained (`note_membership.rs:144-145`,
`note_membership.rs:167-170`).

- Any two field elements are an acceptable destination. `pack_destination`
  runs in `direct.rs:64-77` and `no_ecdsa.rs:277-292`, not in a gate.
- Any field element is an acceptable amount. Membership in `DENOMS_UTERP`
  is `direct.rs:208-210` and, on the prover only, `note_membership.rs:296-298`.
- `claim_note` rebuilds the 192-byte buffer from `campaign.ctx`, the
  anchor, the nullifier, `info.sender`, and the amount
  (`direct.rs:85-95`, `direct.rs:224`, `direct.rs:240`). As written, the
  sender cannot be swapped without invalidating the proof, and a
  non-denomination amount never reaches `proof_instance_verify`.

The medium rating is the boundary. `NoteRelation` does not say "pays the
sender" or "amount is a denomination". A later verifier that passes the
user's instance bytes to `proof_instance_verify`, or a campaign whose
registration path appends a non-denomination leaf, would be accepted by
this key. Suggested fix: keep the rebuild in `instances()` as the only
construction of the buffer; add a test that `ClaimNote`'s message type has
no instance field; leave the denomination check where it is. Do not add a
12-way lookup to the circuit unless the contract check is being removed.
An FV write-up that stops at the circuit will overclaim if it omits this
paragraph.

## F5 — info — `canonical32` drops elements at or above `2^254`

```97:101:contracts/headstash/cw-headstash/src/direct.rs
fn canonical32(b: &[u8], what: &str) -> StdResult<()> {
    if b.len() != 32 || b[31] > 0x3f {
        return Err(StdError::msg(what.to_string()));
    }
    Ok(())
}
```

Pallas base is `2^254 + c` with `c` about `2^125`. Every 32-byte string
with byte 31 at most `0x3f` is strictly less than the prime, so the check
does not admit a second encoding. Field elements in `[2^254, p)` have byte
31 equal to `0x40` and are rejected. A uniform Poseidon output lands there
with probability about `2^-129`. An anchor or nullifier in that set cannot
be claimed. Suggested fix: accept the canonical representative by
`from_repr` instead of the top-byte test. Not a soundness hole.

## F6 — info — nullifier domain is `(secret, ctx)` only

`note_membership.rs:153` hashes the pinned tag, the secret, and ctx. It
does not hash the amount, the position, or the root. Combined with the
global `key_com` set (`notes.rs:305-315`, `registration.rs:584-587`), one
secret can be inserted once and spent once per ctx. Two funded contracts
that both contain that leaf and have different `direct_ctx` values
(`direct.rs:51-61`) will both pay. Suggested fix: none in the circuit.
Treat "one live campaign per note list" as a deployment invariant, and
state it next to `NoteRelation` so a proof engineer does not add the root
into the nullifier and break anchor-independent spends (`direct.rs:11-13`).

## F7 — info — the canonical body string is unused at claim time

`note_canonical_body` (`note_membership.rs:234-248`) formats
`headstash-note-claim-v2:...`. `claim_note` never reads it. The binding is
the six instance cells. Suggested fix: delete the string from any doc that
calls it a signed message, or stop returning it from `prove_note`. Not a
missing constraint.

## F8 — info — K=19 numbers are not a soundness baseline

`wip/k19-lab:tools/headstash/docs/k19-lab-benchmark.md` section 0 says
`FpChip::add` and `FpChip::sub` range-check an output and do not constrain
it to `a ± b`, and that a wrong secret key was accepted by `MockProver`.
The same file's later addendum describes a CRT replacement at commit
`02764a5`. That circuit is not on this branch. This pass did not re-read
the replacement. vk_id 3 does not call that chip. Suggested fix: keep
batch 8 of `04-fv-plan.md` unscheduled until the replacement is the frozen
source. Do not cite the K=19 prove times as evidence the gadget is sound.

## F9 — info — the eligible key is not a circuit witness

`register_notes` verifies EIP-191 or ed25519 over the list hash
(`notes.rs:220-233`, `notes.rs:427-450`) and then appends
`note_registration_leaf(key_com, DENOMS[class])` (`notes.rs:390-394`).
The circuit proves knowledge of a secret whose key commitment is inside
some leaf of an anchored root. It does not prove that the eligible key
signed that leaf. Suggested fix: none. The threat this leaves is a bug in
`register_notes`, which is batch 6 of the plan, not a gate to add.

## What was checked and did not become a finding

- Swap flag boolean. `cond_swap.rs:273-281` together with
  `utilities.rs:169-171` and `utilities.rs:236-239` is `swap * (1 - swap) = 0`.
- Shared `secret` cell between key commitment and nullifier, and shared
  `amount` cell between the leaf and instance row 5
  (`note_membership.rs:151-167`).
- Merkle empty sibling is the field zero on both sides
  (`registration.rs:905-908`, `note_membership.rs:222-231`).
- Padding absorb uses `assign_fixed` plus `constrain_equal`
  (`zakura-common/.../poseidon/pow5.rs:334-354`), which is the repaired
  form of the halo2 padding bug, not the `MockProver`-only form.
- Leaf insertion for vk_id 3 goes through `register_notes` only.
  `insert_registration` is rejected by `legacy_only`
  (`registration.rs:488-493`).
- Verifying-key pin: `notes.rs:147-148` and `direct.rs:125-128` both
  require the blob hash `34d11437…5398`.
