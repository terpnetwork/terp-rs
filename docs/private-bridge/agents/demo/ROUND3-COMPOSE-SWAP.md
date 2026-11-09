# ROUND3-COMPOSE-SWAP

| Field | Value |
|-------|--------|
| **Agent** | COMPOSE + SWAP product path |
| **Date** | 2026-07-20 |
| **Status** | Landed — product pure E2E + single re-export surface |
| **Code home** | [`docs/plans/spectrum/fixtures/compose_seams`](../fixtures/compose_seams/) |
| **Peer** | `private_dex_seams` (`build_swap_action_from_seam_notes`, `apply_swap_action`) |
| **No Halo2** | yes |

---

## Mission completed

1. **`compose_seams` is the single re-export path** for C1–C4 product narrative:
   - Bridge hinge aliases (`hinge_*`)
   - Full `SeamNoteOutV0` + DEX consumability helpers
   - `SwapActionV0` / `SwapFromSeamParams` / `apply_swap_action` / `validate_swap_action` / pool state
2. **Documented end-to-end pure scenario** `product_path_burn_to_swap_sketch`:
   - register assets (tacit + hub) on `AssetRegistryView`
   - `authorize_bridge_mint_to_seam_note_out_apply` → DEX-consumable SEAM note (+rcm)
   - pool setup + `sketch_swap_action_from_seam_notes` → `SwapActionV0`
   - `apply_swap_action` → assert **reserves** + **pool nullifiers** (≠ bridge ν)
3. **Thin harness re-export**: `zk-test-press` `harness::compose_l0` → same product path
4. **Docs**: DEMO-PATH V0c + E2E-HARNESS-PLAN inventory/command + `just demo-e2e-l0` map line

---

## API

| Surface | Role |
|---------|------|
| `run_product_path_burn_to_swap_sketch() -> Result<ProductPathOutcome, ComposeError>` | Callable product pure E2E |
| `ProductPathOutcome` | notes_in, bridge/pool νs, Δ, R before/after |
| `authorize_bridge_mint_to_seam_note_out(_apply)` | C1 path (unchanged) |
| `sketch_swap_action_from_seam_notes` | C4 path (unchanged) |
| Re-exports | hinge + seam + dex apply/validate/pool |

### Pipeline

```text
AssetRegistryView.register(tacit + hub)
        │
        ▼
authorize_bridge_mint_to_seam_note_out_apply  → SeamNoteOutV0 (rcm_flag=1)
        │
        ▼
sketch_swap_action_from_seam_notes            → SwapActionV0
        │
        ▼
apply_swap_action(pool, state, action, false) → R_in' / R_out' + pool ν set
```

Invariants checked:

- Bridge ν marked in `MintedSet`
- Pool-spend ν ≠ bridge ingress lineage / mint ν (NE-4)
- `R_in' = R_in + Δ_in`, `R_out' = R_out − Δ_out`
- Host nullifier set contains action νs; tree_leaves = cm_out len

---

## Tests

| Id | Name | Property |
|----|------|----------|
| C1–C4 | existing | mint→SEAM, H-1, registry, two-note swap sketch |
| **P1** | `product_path_burn_to_swap_sketch` | full apply + reserves + nullifiers |
| harness | `harness::compose_l0::product_path_burn_to_swap_sketch` | thin re-export of P1 |

### Run (product pure e2e)

```bash
cd /Users/returniflost/abstract/terp-core/docs/plans/spectrum/fixtures/compose_seams \
  && cargo test product_path_burn_to_swap_sketch

cd /Users/returniflost/abstract/terp-core/docs/plans/spectrum/fixtures/private_dex_seams \
  && cargo test

# spectrum justfile shortcut
cd /Users/returniflost/abstract/terp-core/docs/plans/spectrum && just product-pure-e2e

# aggregator
cd /Users/returniflost/abstract/terp-core/crates/headstash && just demo-e2e-l0

# harness re-export only
cd /Users/returniflost/abstract/terp-core/crates/headstash \
  && cargo test -p zk-test-press --lib harness::compose_l0
```

**Verify note:** agent environment may lack interactive shell; parent should confirm green:

```bash
cd docs/plans/spectrum/fixtures/compose_seams && cargo test
cd docs/plans/spectrum/fixtures/private_dex_seams && cargo test
```

Expected compose: **8** tests (C1–C4 + registry + zero-rcm + hinge-rcm + product_path).

---

## Files

| File | Action |
|------|--------|
| `fixtures/compose_seams/src/lib.rs` | product path API + re-exports + test |
| `crates/headstash/test-press/src/harness/compose_l0.rs` | **created** thin re-export |
| `crates/headstash/test-press/src/harness/mod.rs` | wire `compose_l0` |
| `crates/headstash/test-press/Cargo.toml` | path dep `compose_seams` |
| `crates/headstash/justfile` | demo-e2e-l0 map line for product pure e2e |
| `crates/headstash/docs/circuit/DEMO-PATH.md` | V0c command block |
| `docs/plans/spectrum/E2E-HARNESS-PLAN.md` | inventory + method + command |
| `docs/plans/spectrum/agents/ROUND3-COMPOSE-SWAP.md` | this report |

---

## Headstash integration points (notes for parent)

| Integration | Status |
|-------------|--------|
| Bridge mint hinge (pure) | consumed via compose |
| SEAM-NOTE-OUT + rcm (DEX consumable) | forced on product path |
| `build_swap_action_from_seam_notes` (ROUND2-SWAP) | delegated |
| `apply_swap_action` reserves + ν | product path only (C4 still sketch-only) |
| `cw-headstash` on-chain mint | out of scope this round (pure only) |
| Halo2 SwapAction circuit | **not** touched |

---

## Parent summary

Round-3 COMPOSE+SWAP product path: `compose_seams` is the L0 re-export surface for authorize→SEAM(+rcm)→SwapAction. New **`product_path_burn_to_swap_sketch`** runs register → bridge mint note → `SwapActionV0` → `apply_swap_action` and asserts reserves + nullifiers. Thin harness re-export under `zk-test-press::harness::compose_l0`. Documented as DEMO-PATH **V0c** / product pure e2e. No Halo2.
