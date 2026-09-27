# ROUND1-SWAP

| Field | Value |
|-------|-------|
| **Agent** | SWAP |
| **Date** | 2026-07-20 |
| **Status** | Design freeze + pure structural stubs (no Halo2 K=18) |
| **SSOT** | `SPEC-private-dex-seams.md`, `SEAM-NOTE-OUT.md`, `fixtures/private_dex_seams` |

---

## Design summary (public/private I/O)

### v1 scope: single-leg private swap

One action pays `asset_in` into a public virtual pool and receives `asset_out` on the constant-product fee-aware curve. Multi-hop = sequential independent actions (no route opcode).

### Public inputs (`SwapActionPublic`)

| Field | Role |
|-------|------|
| `pool_id` | Pair key (star-hub demo: hub ↔ asset_i) |
| `asset_in`, `asset_out` | 32-byte Terp asset ids; must equal oriented pool legs |
| `root` | Allowed commitment-tree root for membership (shared anonymity set `T`) |
| `nullifiers[]` | **Pool-spend** ν (one per spent note) — not claim/burn lineage |
| `cm_out[]` | New leaves: `note_out`, optional `note_change` |
| `delta_r_in`, `delta_r_out` | Public reserve Δ; host recomputes curve from pre-state R |
| `min_out` | Slippage floor (cleartext); reject if curve Δ_out < min_out |
| `gamma`, `gamma_den` | Fee params bound to pool (e.g. 997/1000) |
| `r_in_before`, `r_out_before` | Pre-trade public reserves (statement binds host load) |
| `oracle_mid?`, `oracle_params?`, `now_height` | Optional bounds only — **never** mint authority |

**Honest privacy posture (v1):** pair, reserve deltas (approx size), nullifiers, and new cms are public; who traded and note lineage stay private under a future proof.

### Private witnesses (`SwapActionWitness`)

| Field | Role |
|-------|------|
| `notes_in[]` | Openings: `asset_id`, `value`, `cm_public`, `owner_binding`, `rcm`, `ingress_nullifier_lineage`, path stub |
| `note_out` | Output of `asset_out` with `value = Δ_out` (exact equality in v1) |
| `note_change?` | Residual `asset_in` if sum(notes_in) > Δ_in |
| `delta_in` | Amount paid into the curve (must equal public `delta_r_in`) |

Merkle auth paths remain stubs in pure round-1; circuit round-2 binds them under `root` via Headstash tree gadgets.

### Conservation (normative, oracle-free)

1. `sum v(notes_in) = Δ_in + v(change)`
2. `v(note_out) = Δ_out` (exact; no silent mint)
3. `Δ_out = floor(R_out · γ · Δ_in / (R_in · γ_den + γ · Δ_in))` on **pre-state** public reserves
4. `Δ_out ≥ min_out`
5. Post-state: `R_in' = R_in + Δ_in`, `R_out' = R_out − Δ_out` with `R_out' > 0`

Oracle may only gate acceptance (freshness / slippage band). Any API that credits notes or bumps reserves from mid alone → `ErrOracleDisabledMint`.

---

## Relation to pure seams + Headstash

### Pure seams (`fixtures/private_dex_seams`)

| Surface | Reuse |
|---------|--------|
| `quote_exact_in` / `apply_reserves` | Curve + reserve math (SPEC §2.2) |
| `apply_swap` + `NoteIn` | Earlier u64-nullifier host harness (SPEC §6.1–6.6) — still green |
| `check_oracle_bound` / mint rejects | SPEC §3 bounds + hard no-mint |
| **New** `SwapActionV0` | SEAM-NOTE-OUT widths: 32-byte asset_id / cm / pool ν; full structural action |

### Headstash (reuse later for circuit; not round-1 prove)

| Component | Swap use |
|-----------|----------|
| Note commit / `cmx` (`note_commit`, Sinsemilla) | Output + spent note openings (when `cm_encoding = ORCHARD_CMX`) |
| Nullifier derive (`Nullifier::derive`) | Replace harness `synthetic_pool_spend_nf` for production spends |
| Merkle path / `Anchor` | Membership under shared tree `T` |
| Spend-auth / keys | Owner authorization for spends |
| Distro Poseidon tree | **Separate** — eligibility only; not pool `T` (CLARITY / A §1.6) |

### New relative to claim circuit

- Public **Δ reserves** + host recompute of AMM formula
- **`min_out`** inequality
- Fee `(γ, γ_den)` binding
- Optional **oracle bound** public inputs (host-side in v1; circuit may omit and leave host check)
- Multi-asset **asset_id** binding to pool legs
- Change note + multi-input merge conservation

### SEAM-NOTE-OUT alignment

- Spend inputs map from `SeamNoteOutV0` per §4.2 (`asset_id`, `value`, `cm_public`, `owner_binding`, `rcm` if `rcm_flag=1`)
- `nullifier_lineage` is **ingress only**; pool-spend ν is derived (`pool-nf-v0` harness / Orchard derive later) — NE-4
- Shared anonymity set: swap outs append to same `T` as claim/bridge leaves (SPEC §4)

### Non-goals (v1)

- Multi-hop / `T_SWAP_ROUTE` atomic route
- Full LP add/remove / farm
- Sealed batch auction / `T_SWAP_BATCH`
- SP1 / Tacit guest in circuit
- Oracle as mint or reserve authority
- Dual-tree link proofs; production Orchard ⟷ Tacit leaf unification
- Full Halo2 Circuit + MockProver (blocked neither by design freeze nor H1 claim last-mile)

---

## Code landed (paths + tests)

| Path | What |
|------|------|
| `docs/plans/spectrum/fixtures/private_dex_seams/src/lib.rs` | Existing seam math + **SwapActionV0** types, validate/apply, synthetic pool-nf / abstract leaf |
| `docs/plans/spectrum/fixtures/private_dex_seams/Cargo.toml` | `sha2` for structural domain hashes |
| `docs/plans/spectrum/agents/ROUND1-SWAP.md` | This design freeze |

**Code placement decision:** expand pure `private_dex_seams` first (no `crates/headstash/circuit/src/swap/` yet). Circuit module lands in round-2 once note-commit layout is stable enough to share gadgets without fighting H1 claim MockProver debt.

### Tests

**Prior (still green):** SPEC §6.1–6.6 suite — `swap_happy`, `min_out_fail`, `oracle_stale`, optional stale allows AMM, oracle mint/reserve reject, inflated min_out, double_spend, wrong_asset, quote/slippage units.

**New structural:**

| Test | Intent |
|------|--------|
| `swap_action_structural_fixture_happy` | Full action: notes in, min_out, R before/after, pool ν ≠ lineage, cm_out, apply_swap_action |
| `swap_action_structural_fixture_with_change` | Conservation with change leaf |
| `swap_action_rejects_ingress_lineage_as_pool_nf` | NE-4 domain separation |
| `swap_action_rejects_min_out_above_curve` | ErrMinOut |
| `swap_action_rejects_wrong_asset_out_note` | ErrWrongAsset |
| `swap_action_double_spend_pool_nf` | ErrNullifierExists on second apply |

Run:

```bash
cd docs/plans/spectrum/fixtures/private_dex_seams && cargo test
```

---

## Open crypto choices

1. **Amount posture:** v1 freezes **public Δ_in/Δ_out** (transparent reserve math; simpler circuit). Hidden amounts + public net Δ is a later privacy upgrade, not required for demo seams.
2. **Leaf encoding in shared `T`:** production path prefers one `cm_encoding` (Orchard cmx) for all product leaves; harness may use `ABSTRACT_LEAF_V0` (0x03). Dual Orchard+Tacit leaves need a published link policy (non-goal v0).
3. **Nullifier algorithm:** harness `H("pool-nf-v0" ‖ cm ‖ rcm)` vs Headstash `DeriveNullifier(nk, rho, psi, cm)`. Round-2 should switch spend gadgets to Headstash derive when openings are Orchard-shaped.
4. **Fee destination:** fees stay in pool reserves (LP virtual inventory). Protocol fee skim (`fee_protocol_bps`) is optional and **0** for demo unless team names a recipient.
5. **Oracle in-circuit:** default **host-only** bound check; circuit proves conservation + curve + membership only. In-circuit mid range is optional later.
6. **Value width:** note `value` is u64 on SEAM-NOTE-OUT; reserves use u128 in pure seams — circuit must document truncation/range checks.
7. **Root window:** single allowed root stub vs lagged root window (`ErrBadRoot`) — host policy TBD.

---

## Clarity questions for team (numbered)

1. **`asset_id` encoding SSOT:** Is Headstash `NoteDenom` (blake3→Fp) the same 32-byte object as SEAM-NOTE-OUT `asset_id` for claim→swap, or do we require a registry map (`terp_asset_id` style) before multi-asset merge?
2. **One tree vs multi-root:** Confirm Phase-1 demo uses a **single** privacy commitment tree `T` for claim + bridge + swap (multi-root only for **eligibility** distro anchors, not pool membership).
3. **Fee to whom:** Confirm v1: fees remain in `(R_A, R_B)` only; no separate fee note / treasury note until protocol fee is non-zero.
4. **Oracle optional default:** Confirm demo default `require_oracle = false` (pure AMM always available); oracle path is opt-in for bound demos only.
5. **Public Δ posture:** Confirm shipping public `delta_r_in/out` for v1 demo (simpler) vs hiding trade size behind commitments with only net Δ public.
6. **Hub asset identity:** Freeze demo hub as shielded TERP vs a tacit-mapped cBTC-class unit for star topology labeling.
7. **Spend auth on SEAM-NOTE-OUT:** Is `owner_binding` (32-byte Cosmos/RecpAddr) sufficient for phase-1 spend checks, or must phase-1 already require full Orchard spend-auth keys?
8. **Change policy:** Always allow change notes, or require exact-input notes for v1 to shrink the circuit (one less output)?

---

## Risks

| Risk | Mitigation |
|------|------------|
| Claim still transparent (A×D Issue 1) — no leaf in `T` | Phase table: swap design assumes phase ≥1 claim/bridge append; structural map ready |
| Orchard vs Tacit leaf crypto not equivalent | `cm_encoding` tags; harness abstract leaf; do not claim unlinkability across encodings |
| Ingress nullifier reused as pool ν | Structural tests + NE-4; validate rejects equal lineage |
| Oracle mid treated as mint | No successful mint path; pure tests hard-reject |
| Headstash claim MockProver last-mile | Swap pure freeze independent; circuit reuses gadgets only after claim layout stable |
| Public Δ leaks size | Document honesty table; optional later hidden-amount upgrade |
| u64 note value vs u128 reserves | Range-check in circuit; fixture amounts stay in u64 |

---

## Round-2 implementation plan (ordered)

1. **Freeze answers** to clarity Q1–Q8 (asset_id, tree, fee, oracle default, Δ posture, hub, owner, change).
2. **Wire SEAM-NOTE-OUT → SwapAction:** pure test: `from_headstash_instance` / bridge map → `is_dex_consumable` → `SpendNoteOpening` → `validate_swap_action` (compose C7 / C1.2 structural).
3. **Add `crates/headstash/circuit/src/swap/` module skeleton:** types only + pure helper re-exports / shared formula; **no** full Circuit yet.
4. **Circuit constraints (incremental):**
   - (a) asset_id + value conservation (in/out/change)
   - (b) note commit open for spends + outputs
   - (c) nullifier derive + public ν
   - (d) Merkle path to public root
   - (e) floor division gadget for curve Δ_out vs public `delta_r_out` and `min_out`
5. **Host apply path:** CosmWasm / module stub loads Pool, recomputes quote, verifies proof, inserts ν, appends cms, updates R (mirror pure `apply_swap_action`).
6. **MockProver** on a reduced-K or simplified layout action (may lag claim H1 green).
7. **Oracle host path** only (stale / slippage) in integration tests; keep circuit free of mint authority.
8. **Demo sequence:** claim-or-bridge stub leaf → swap hub edge → observe public ΔR + unlinkability narrative.

---

## Document history

| Date | Change |
|------|--------|
| 2026-07-20 | Round-1 freeze: I/O, pure SwapActionV0 stubs + structural fixture tests |
