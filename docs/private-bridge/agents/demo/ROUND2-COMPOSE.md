# ROUND2-COMPOSE

| Field | Value |
|-------|--------|
| **Agent** | COMPOSE (L0 pure glue) |
| **Date** | 2026-07-20 |
| **Status** | Landed — pure cross-crate fixture |
| **Code home** | [`docs/plans/spectrum/fixtures/compose_seams`](../fixtures/compose_seams/) |
| **SSOT** | `CLARITY-cw-headstash-router-and-asset-registry.md`, ROUND1-META-REVIEW P0s |
| **Product freeze** | `cw-headstash` is mint/router; asset registry never mints balances |

---

## Mission completed

Cross-crate pure L0 compose that other agents / harness can re-export:

1. `authorize_bridge_mint` (bridge hinge) → full **SEAM-NOTE-OUT** with **rcm present** (DEX consumable)
2. Optional structural path into **SwapActionV0** (no prove)
3. Shared **AssetRegistryView** stub (tacit_id / denom / 32-byte asset_id)

No halo2, no cosmwasm.

---

## API surface

Crate: `compose_seams` (path deps on `bridge_auth_seams`, `seam_note_out`, `private_dex_seams`).

### Asset registry (compose SSOT)

| Type / fn | Role |
|-----------|------|
| `AssetRegistryView` **struct** | Rich in-memory map: tacit → asset_id, denom → asset_id, active set + status |
| `AssetRecord` | `asset_id`, optional `tacit_id` / `denom`, `origin`, `status` |
| `register` | Mapping only — **never credits balances** |
| `resolve_tacit` / `resolve_denom` | Active-only resolve |
| `to_bridge_registry` | Project → hinge `AssetRegistry` |
| `to_dex_asset_map` | Project → SWAP `AssetMap` |
| `impl private_dex_seams::AssetRegistryView` | `is_registered` ↔ active Terp id |

> Naming note: SWAP defines `AssetRegistryView` as a **trait**; COMPOSE uses the same name for a **rich struct** that implements the trait. Re-export alias: `DexAssetRegistryViewTrait`.

### Bridge → SEAM-NOTE-OUT

| Type / fn | Role |
|-----------|------|
| `NoteOpenings { rcm, recompute_abstract_cm }` | Attach / override openings |
| `sketch_to_seam_note_out` | `NoteOutSketch` + openings → full `SeamNoteOutV0` (`rcm_flag=1`) |
| `sketch_to_seam_note_out_from_hinge` | Prefer sketch-carried rcm (ROUND2-BRIDGE) + recompute abstract cm |
| `authorize_bridge_mint_to_seam_note_out` | Registry check → hinge → full note → `is_dex_consumable` |
| `authorize_bridge_mint_to_seam_note_out_apply` | Same + `minted.mark(ν)` |
| `ComposeError` | `Bridge` / `Seam` / `UnregisteredAsset` / `MissingOpenings` / `SwapSketch` / `Swap` |

**Invariant:** Compose never bypasses hinge gates. Spent-only still maps to `Bridge(NotInBurnSet)`.

When `recompute_abstract_cm = true` (default via `NoteOpenings::with_rcm`):

- `cm_encoding = ABSTRACT_LEAF_V0 (0x03)`
- `cm_public = abstract_leaf_cm(asset, value, owner, rcm)` so SwapAction openings match

ROUND2-BRIDGE already emits `rcm`/`rcm_flag` on `NoteOutSketch`; compose still normalizes to full `SeamNoteOutV0` and may recompute abstract leaf for DEX.

### Optional SwapActionV0 sketch

| Type / fn | Role |
|-----------|------|
| `seam_note_out_to_sketch` | `SeamNoteOutV0` → SWAP `SeamNoteSketch` |
| `sketch_swap_action_from_seam_notes` | notes + `SwapFromSeamParams` + registry → `SwapActionV0` |
| Delegates to | `private_dex_seams::build_swap_action_from_seam_notes` (ROUND2-SWAP) |

Does **not** prove; structural `validate_swap_action` runs inside the SWAP helper.

### Re-exports

- Hinge: `hinge_authorize_bridge_mint`, `hinge_authorize_bridge_mint_apply`, claim/snapshot aliases
- Seam: `seam_is_dex_consumable`, `to_dex_spend_inputs`, origin/nf constants
- Dex: `validate_swap_action`, `SwapActionError`, `SwapFromSeamParams`, `AssetMap`, sketch aliases

---

## Pipeline (Track B → C)

```text
ReflectionSnapshot + BridgeMintClaim
        │
        ▼
AssetRegistryView (active resolve) ──reject──► UnregisteredAsset
        │
        ▼
authorize_bridge_mint (bridge_auth_seams) ──reject──► Bridge(…)  // H-1 intact
        │
        ▼
NoteOutSketch + NoteOpenings.rcm
        │
        ▼
SeamNoteOutV0 { rcm_flag=1, abstract cm } + is_dex_consumable
        │
        ▼ (optional)
sketch_swap_action_from_seam_notes → SwapActionV0
        │
        ▼
validate_swap_action (structural; no ZK)
```

---

## Tests

| Id | Name | Property |
|----|------|----------|
| **C1** | `c1_happy_bridge_mint_to_seam_note_out` | Happy hinge → full note, `rcm_flag=1`, openings, `is_dex_consumable`, 382-byte layout, provenance=burn root |
| **C2** | `c2_h1_spent_only_still_rejects` | `spent_only && !in_burn_set` → `Bridge(NotInBurnSet)`; compose does not bypass |
| **C3** | `c3_registry_asset_id_and_unregistered_reject` | Note `asset_id` = registry Terp id; empty/wrong registry → `UnregisteredAsset` |
| **C4** | `c4_sketch_swap_action_from_two_notes_and_pool` | Two mint notes → `SwapActionV0`; pool ν ≠ ingress lineage; `validate_swap_action` green |
| — | `registry_never_mints_only_resolves` | Registry has resolve API only |
| — | `zero_rcm_openings_reject` | Zero rcm → `MissingOpenings` |

### Run

```bash
cd /Users/returniflost/abstract/terp-core/docs/plans/spectrum/fixtures/compose_seams && cargo test
```

Expected: **7** unit tests (C1–C4 + registry invariant + zero-rcm + hinge-rcm-without-openings). Agent environment lacked an interactive shell; parent should confirm.

---

## Closes / maps to META P0

| META P0 | How addressed |
|---------|----------------|
| No cross-crate L0 path | This crate + C1/C4 |
| rcm gap on bridge egress | `NoteOpenings` + `sketch_to_seam_note_out` force `rcm_flag=1` |
| asset_id SSOT | `AssetRegistryView` used by bridge gate projection + note asset check |
| H-1 | C2 re-asserts spent-only reject through compose |

---

## Coordination with ROUND2-BRIDGE / ROUND2-SWAP

| Peer | What we consume |
|------|-----------------|
| BRIDGE | `authorize_bridge_mint`, `NoteOutSketch` with rcm/memo fields, `BridgeMintPublic.rcm` |
| SWAP | `SeamNoteSketch`, `build_swap_action_from_seam_notes`, `AssetRegistryView` trait, `AssetMap` |
| SEAM | full `SeamNoteOutV0`, `is_dex_consumable`, 382-byte layout |

COMPOSE owns **cross-crate path deps** + richer tacit/denom registry + conversion helpers; does not fork note languages.

## Remaining gaps

| Gap | Owner bias | Notes |
|-----|------------|-------|
| Claim path compose (`from_headstash_instance` → swap) | COMPOSE / SWAP | Only bridge→seam→swap landed; claim track is independent (Track A) |
| Shared registry fixture **JSON** file | HARNESS | In-memory only; no genesis/JSON SSOT file yet |
| Trait vs struct `AssetRegistryView` name collision | product freeze | Prefer one shared package later |
| claimId SHA-256 vs Tacit keccak | BRIDGE | Labeled fixture domain; wire-compat deferred |
| Real IMT membership | BRIDGE | Still bool `in_burn_set` / `in_pool_root` |
| CosmWasm `cw-headstash` BridgeMintNote | BRIDGE | Types sketch only; not this crate |
| Halo2 SwapAction circuit | SWAP | Structural only here |
| `just e2e-l0` aggregator | HARNESS | Should include `compose_seams` |
| Dual-note cm path when `recompute_abstract_cm=false` | — | Prefer recompute for swap; hinge cm may be abstract fingerprint |
| Oracle-cannot-mint compose surface | optional | Domain crates already reject; no extra C case in this crate |
| **cargo test execution** | parent | Agent shell unavailable; parent should run recipe below |

---

## File layout

```text
docs/plans/spectrum/fixtures/compose_seams/
  Cargo.toml          # path deps: bridge_auth_seams, seam_note_out, private_dex_seams, sha2
  src/lib.rs          # AssetRegistryView + compose APIs + C1–C4 tests
```

---

## Parent summary

- New pure crate `compose_seams` wires bridge hinge → full SEAM-NOTE-OUT (with rcm) → optional SwapActionV0.
- Shared `AssetRegistryView` is the multi-asset map SSOT for fixtures (resolve-only).
- Tests C1–C4 cover happy DEX-consumable mint, H-1, registry reject, and two-note swap sketch.
- Run: `cargo test` in `docs/plans/spectrum/fixtures/compose_seams`.
