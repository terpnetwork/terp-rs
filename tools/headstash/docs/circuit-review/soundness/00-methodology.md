# 00 — Methodology

This pass scopes soundness and a Lean formal-verification (FV) plan for the
v1 production claim circuit, vk_id 3. It does not prove anything in Lean and
it does not change the circuit.

## What was followed

The proving workflow is the rot256 Lean skill
([`SKILL.md`](https://github.com/rot256/skills/blob/master/lean/SKILL.md),
raw `https://raw.githubusercontent.com/rot256/skills/master/lean/SKILL.md`)
plus the two files it links:

- [`references/setup.md`](https://github.com/rot256/skills/blob/master/lean/references/setup.md)
  — elan toolchain, `lake exe cache get` before any `lake build`, never
  compile Mathlib from source, `lake build --wfail` is the completeness signal.
- [`references/tactics.md`](https://github.com/rot256/skills/blob/master/lean/references/tactics.md)
  — `exact?` / `apply?` / `rw?` / `simp?`, Loogle, LeanSearch, and the
  small-`have` style.

Two rules in that skill dominate the plan in `04-fv-plan.md`:

1. **Plan before proving.** Write the mathematical claim, decompose it into
   lemmas, then prove. Do not open a tactic file first.
2. **A green `lake build` is not a finished proof.** `sorry`, `admit`, and
   extra axioms fail the bar. The skill's check is `lake build --wfail`.
   Theorem statements are immutable once written down: if a statement is
   wrong, stop and fix the statement in review, do not weaken it to close
   the goal.

This host is a mainnet node. The skill's cache rule and the brief's build
rule agree: no `lake build`, no Mathlib fetch, no `cargo build` / `cargo test`,
no proving at K ≥ 20. Row count and timings below are cited from an existing
measurement, not re-run here.

## How that workflow was applied here

The "prove" stage is deferred. This directory is stages 1–3 of the skill.

### Stage 1 — human-readable claim

The claim is knowledge soundness of one relation, not zero-knowledge and not
the Halo 2 verifier. Restated so a later Lean statement can be checked
against it:

> A vk_id 3 proof, verified against the pinned verifying key, convinces a
> verifier of the six public field elements
> `(ctx, root, nullifier, dest_lo, dest_hi, amount)` only if the prover knows
> a secret and a depth-17 authentication path such that
>
> - `key_com = Poseidon_{L=2}(tag_keycom, secret)`,
> - `nullifier = Poseidon_{L=3}(tag_nf, secret, ctx)`,
> - `leaf = Poseidon_{L=3}(tag_leaf, key_com, amount)`,
> - folding `leaf` with the path under `Poseidon_{L=4}(tag_crh, layer, left, right)`
>   and a boolean swap bit per layer yields `root`,
> - `dest_lo` and `dest_hi` are the public elements and are otherwise
>   unconstrained,
>
> with every tag and every layer index fixed in the circuit.

What this claim deliberately does **not** say, because the circuit does not
prove it:

- `amount` is one of the twelve denominations.
- `(dest_lo, dest_hi)` is the packed bytes of a bech32 address, or of the
  address that signed registration.
- `ctx` equals `direct_ctx(chain_id, contract)`.
- `root` is an anchor the contract has accepted.
- `nullifier` has not been spent.
- The leaf was inserted by an ECDSA or ed25519 registration.

Those are contract obligations. They are batch 6 in the plan, not gates.

### Stage 2 — decomposition

The skill asks for one obligation per non-trivial step. The circuit splits
into the six components the brief names. Each component in `03-components.md`
lists the lemma that a later `.lean` file would have to prove, the Ironwood
or other lemma it should call, and the gap. The composition order is in
`02-scoping.md`.

### Stage 3 — todo list

`04-fv-plan.md` is that list, batched so independent proofs can land without
sharing a working tree. No `sorry` is introduced in this pass because no
Lean project is created.

## Source discipline

Every claim about the circuit cites `path:line` in this worktree, or an
absolute path for `zakura-common`, which lives outside the worktree at
`/Users/returniflost/abstract/terp-core/crates/zakura-common`.

The production circuit is `NoteMembershipCircuit`, the `PIN = true`
instantiation of `NoteCircuitImpl`:

```83:83:tools/headstash/circuits/halo2/src/note_membership.rs
pub type NoteMembershipCircuit = NoteCircuitImpl<true>;
```

`PIN = false` is the vk_id 2 shape and is not the deployed key. See finding
F3.

## Out of scope for the proofs

- Zero-knowledge of the Zakura prover. That is a separate Ironwood/Zakura
  argument (https://zakura.com/engineering/ironwood-zero-knowledge/) and is
  not required for claim integrity.
- The Halo 2 IPA verifier, except as a named assumption. Ironwood already
  has a verifier knowledge-soundness stack under `Zcash/Snark/`. Re-proving
  it is not on the vk_id 3 path unless a diff shows the zakura verifier
  diverged. See `01-prior-art.md`.
- The K=19 ECDSA and raw-key circuits on `wip/k19-lab`. They are context
  for what not to formalize first. The lab's own note is that the old
  foreign-field chip was unsound
  (`wip/k19-lab:tools/headstash/docs/k19-lab-benchmark.md`, section 0).
