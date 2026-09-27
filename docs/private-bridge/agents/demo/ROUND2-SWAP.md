# ROUND2-SWAP

| Field | Value |
|-------|-------|
| **Agent** | SWAP |
| **Date** | 2026-07-20 |
| **Status** | SEAM → SwapAction pure compose + registry view (no Halo2) |
| **SSOT** | `SPEC-private-dex-seams.md`, `SEAM-NOTE-OUT.md`, `CLARITY-cw-headstash-router-and-asset-registry.md`, `fixtures/private_dex_seams` |
| **Prior** | [`ROUND1-SWAP.md`](./ROUND1-SWAP.md), [`ROUND1-META-REVIEW.md`](./ROUND1-META-REVIEW.md) |

---

## Mission checklist

| # | Item | Result |
|---|------|--------|
| 1 | Pure helper: build/validate **SwapActionV0** from SEAM-shaped note sketches | **Done** — `build_swap_action_from_seam_notes` |
| 2 | Accept registry-resolved 32-byte asset ids (`AssetMap` / `AssetRegistryView`) | **Done** — demo HUB/B/C kept as test helpers only |
| 3 | Tests: happy SEAM→swap; wrong asset; missing rcm (not dex-consumable) | **Done** (+ unregistered asset + custom registry id) |
| 4 | No `circuit/src/swap` Halo2 Circuit | **Honored** — pure fixture only |
| 5 | Coordinate with COMPOSE if present | **No COMPOSE crate** — mirrored SEAM sketch types in D fixture |

---

## Frozen decisions consumed

From `CLARITY-cw-headstash-router-and-asset-registry.md` + meta review:

- Notes minted later via **cw-headstash** router; pure layer accepts **registry-resolved 32-byte `asset_id`s**
- Registry **never mints** balances
- No oracle mint; no full Halo2 this round (H1 claim still red)
- Prefer pure `private_dex_seams` over Headstash circuit thrash

---

## Code landed

**Path:** `/Users/returniflost/abstract/terp-core/docs/plans/spectrum/fixtures/private_dex_seams/src/lib.rs`

### Asset registry view

| Surface | Role |
|---------|------|
| `AssetRegistryView` | Trait: `is_registered(&[u8;32])` |
| `AssetMap` | In-memory register / optional labels; `require`, `resolve_label` |
| `AssetMap::demo_hub_b_c()` | Fixture seed only — not product SSOT |
| `asset_id_hub/b/c()` | **Deprecated as sole path**; retained as labeled test helpers |

### SEAM → SwapAction compose

| Surface | Role |
|---------|------|
| `SeamNoteSketch` | D-side SEAM-NOTE-OUT spend subset (mirror; no path dep on `seam_note_out`) |
| `is_dex_consumable_sketch` | §4.3 spirit: `rcm_flag=1`, non-zero rcm/asset/cm, encoding set |
| `spend_opening_from_seam_sketch` | Sketch → `SpendNoteOpening` (uses sketch `cm_public` as-is) |
| `seam_sketch_abstract_leaf` | Build consumable abstract-leaf sketch |
| `SwapFromSeamParams` | pool legs, reserves, min_out, root, out/change openings |
| `build_swap_action_from_seam_notes` | Compose + `validate_swap_action` |

### Error extensions

- `SwapActionError::ErrNotDexConsumable` — missing rcm / non-consumable note  
- `SwapActionError::ErrUnregisteredAsset` — asset not in registry view  

### Explicit non-goals this round

- `crates/headstash/circuit/src/swap/` Halo2 Circuit / MockProver  
- CosmWasm host apply path  
- Path dependency on `seam_note_out` (mirrored widths; COMPOSE can own conversion later)  
- Oracle mint path (still hard-reject)

---

## Tests added

| Test | Property |
|------|----------|
| `seam_to_swap_action_structural_happy` | SEAM sketch (registry B) → SwapAction → validate + apply; pool ν ≠ lineage |
| `seam_to_swap_action_rejects_wrong_asset` | Note asset C vs params B→hub → `ErrWrongAsset` |
| `seam_to_swap_action_rejects_missing_rcm` | `rcm_flag=0` → `ErrNotDexConsumable` |
| `seam_to_swap_action_rejects_unregistered_asset` | B not registered → `ErrUnregisteredAsset` |
| `asset_map_demo_labels_are_test_helpers_not_sole_path` | Arbitrary 32-byte registry ids (not HUB/B/C bytes) compose cleanly |

**Prior green suites retained:** SPEC §6.1–6.6 + ROUND1 SwapAction structural fixtures.

### Run

```bash
cd /Users/returniflost/abstract/terp-core/docs/plans/spectrum/fixtures/private_dex_seams && cargo test
```

Expected: **23** `#[test]` (18 prior ROUND1 + 5 new ROUND2). Agent environment lacked an interactive shell; parent should confirm:

```text
cd docs/plans/spectrum/fixtures/private_dex_seams && cargo test
```

---

## Compose / COMPOSE coordination

| Concern | ROUND2 posture |
|---------|----------------|
| `SeamNoteOutV0` ↔ `SeamNoteSketch` | Field-role mirror; COMPOSE should add thin conversion when cross-crate L0 lands |
| Bridge `NoteOutSketch` still lacks rcm | BRIDGE r2; SWAP rejects non-consumable until rcm attached |
| Shared `AssetRegistryView` | Same trait spirit as CLARITY target; bridge `AssetRegistry` is HashSet of mapped ids — compatible idea, not yet one crate |
| C7 structural path | Happy SEAM→swap pure path exists; full bridge_mint → SEAM → swap still needs BRIDGE rcm + COMPOSE glue |

---

## Clarity / meta answers (SWAP scope)

| Q | ROUND2 stance |
|---|---------------|
| M3 asset_id | Accept **only** registered 32-byte ids; no third scheme |
| M10 DEX timeline | Stay L0 pure this round |
| M15 public Δ | Still public `delta_r_in/out` |
| M16 fees in reserves | Unchanged |
| M17 require_oracle default false | Unchanged (params optional) |
| M19 owner_binding | Structural only; no Orchard spend-auth |
| M20 change | Supported when `change_rcm` + residual |

---

## Risks / follow-ups

| Risk | Mitigation / next |
|------|-------------------|
| Dual sketch types vs `seam_note_out` | COMPOSE conversion helper; avoid third language |
| Bridge notes without rcm | `ErrNotDexConsumable` until BRIDGE emits openings |
| Enum `AssetId` vs `AssetId32` dual APIs | Labeled: u64-nf `apply_swap` legacy; SwapAction path is product-facing |
| Circuit still deferred | After H1 claim layout stable |

---

## Document history

| Date | Change |
|------|--------|
| 2026-07-20 | ROUND2: AssetMap + SEAM sketch → SwapActionV0 pure compose + tests |
