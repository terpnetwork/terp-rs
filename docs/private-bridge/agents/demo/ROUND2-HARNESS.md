# ROUND2-HARNESS

| Field | Value |
|-------|--------|
| **Agent** | HARNESS |
| **Round** | 2 |
| **Date** | 2026-07-20 |
| **Scope** | L0/L1 suite wiring — pure re-exports, claim fixture, `just demo-e2e-l0`, inventory |
| **Frozen** | One mint: **`cw-headstash`** only; suite extends Headstash/PrivateBridge ([`CLARITY-cw-headstash-router-and-asset-registry.md`](../CLARITY-cw-headstash-router-and-asset-registry.md)) |
| **SSOT plan** | [`../E2E-HARNESS-PLAN.md`](../E2E-HARNESS-PLAN.md) |

---

## Delivered

### 1. L0 pure paths + suite methods

| Surface | Path | Notes |
|---------|------|-------|
| Public hinge world | `docs/plans/spectrum/fixtures/bridge_auth_seams` → `hinge_happy_fixture()` | Shared with crate unit tests |
| Harness re-exports | `crates/headstash/test-press/src/harness/bridge_l0.rs` | No Docker / no Halo2 |
| Claim fixture A2 | `…/harness/claim_fixture.rs` | Load JSON + synthetic builder; 168-byte instance contract |
| Suite | `…/suites/private_bridge.rs` | Methods call L0; still one `cw-headstash` deploy |

**Implementable now (L0):**

| Method | E2E | Body |
|--------|-----|------|
| `assert_policy_bridge_mint_happy` | E2E-01 | `authorize_bridge_mint_apply` happy |
| `assert_h1_spent_only_reject` (+ `assert_ordinary_spend_not_burn`) | E2E-03 | spent_only → `NotInBurnSet` |
| `assert_double_mint_reject` | E2E-02 | second mint → `AlreadyMinted` |
| `compose_bridge_mint_to_seam` | E2E-09 | 382-byte SEAM bytes + optional `seam_note_out` decode (`l0-seams`) |
| `load_claim_fixture` / `build_claim_fixture` | E2E-08 | DEMO-PATH A2 policy only |
| `load_claim_fixture_and_process` | E2E-08 | validates policy; **no** CW submit yet (mock ZK hook TBD) |
| `e2e_burn_to_mint_fixture` | E2E-01 | LC mock gate + L0 happy (no CW mint msg) |

**Still stub / external crate:**

- `apply_swap_fixture` → points at `private_dex_seams` via `demo-e2e-l0`
- `assert_oracle_cannot_mint` → same (E2E-13 lives in dex pure tests)

### 2. `just` targets

| Recipe | Where | Runs |
|--------|-------|------|
| `demo-e2e-l0` / `e2e-l0` | `crates/headstash/justfile` | 4 fixture crates (incl. `compose_seams`) + `zk-test-press` harness + suite |
| `e2e-l0-pure` | `docs/plans/spectrum/justfile` | 4 fixture crates only |
| `e2e-l3-smoke-stub` | spectrum justfile | **Documented stub only** — no dual-container implement |

### 3. L1 mock ZK posture (documented)

| Track | Command | PR CI? | Label |
|-------|---------|--------|-------|
| L0 pure | `just demo-e2e-l0` | yes | structural |
| Claim policy fixture | `ClaimFixture` + mock_proof_hex | yes when wired | **mock ZK** — not Tier-0 |
| Real H1 prove | `just demo-h1` | **no** (nightly/manual) | circuit track |

Also updated: `DEMO-PATH.md` (V0b + mock vs prove table), `E2E-HARNESS-PLAN.md` §6 freeze.

### 4. Inventory counts (source `#[test]` scan 2026-07-20 r2)

| Crate | Round-1 claim | Meta D6 | **Round-2 scan** |
|-------|--------------:|--------:|-----------------:|
| `bridge_auth_seams` | 9 | 17 | **18** |
| `private_dex_seams` | 12 | 18 | **23** (SWAP r2 SEAM compose added) |
| `seam_note_out` | 6 | 6 | **6** |
| `compose_seams` | — | — | **~7** (COMPOSE r2 C1–C4) |

`E2E-HARNESS-PLAN.md` inventory tables updated.

### 5. Explicit non-goals this round

- Full ict-rs dual-container / E2E-19 green path  
- CW `process_headstash` mock-verify multi-test wiring (loader only)  
- Real H1 MockProver as harness gate  
- Parallel mint package  

---

## Files created / changed

| File | Action |
|------|--------|
| `docs/plans/spectrum/fixtures/bridge_auth_seams/src/lib.rs` | **changed** — `label_hash`, public `hinge_happy_fixture` |
| `crates/headstash/test-press/src/harness/{mod,bridge_l0,claim_fixture}.rs` | **created** |
| `crates/headstash/test-press/src/suites/private_bridge.rs` | **rewritten** — L0 methods + claim fixture + LC lag |
| `crates/headstash/test-press/src/lib.rs` | **changed** — `pub mod harness` |
| `crates/headstash/test-press/Cargo.toml` | **changed** — path deps `bridge_auth_seams`, optional `seam_note_out` (`l0-seams`) |
| `crates/headstash/justfile` | **changed** — `demo-e2e-l0` / `e2e-l0` |
| `docs/plans/spectrum/justfile` | **created** — pure + L3 stub |
| `docs/plans/spectrum/E2E-HARNESS-PLAN.md` | **changed** — inventory, methods status, mock ZK, tooling |
| `crates/headstash/docs/circuit/DEMO-PATH.md` | **changed** — V0b + mock ZK table |
| `docs/plans/spectrum/agents/ROUND2-HARNESS.md` | **created** — this report |

---

## Suggested parent verification

```bash
cd crates/headstash && just demo-e2e-l0
# or piecemeal:
cd docs/plans/spectrum/fixtures/bridge_auth_seams && cargo test
cd docs/plans/spectrum/fixtures/private_dex_seams && cargo test
cd docs/plans/spectrum/fixtures/seam_note_out && cargo test
cd docs/plans/spectrum/fixtures/compose_seams && cargo test
cd crates/headstash && cargo test -p zk-test-press --lib harness::
cd crates/headstash && cargo test -p zk-test-press --lib suites::private_bridge::
```

---

## Cross-agent (concurrent Round 2)

- **BRIDGE** landed `cw-headstash` `bridge` module + `BridgeMintNote` with mock verify (see `ROUND2-BRIDGE.md`). Harness suite methods remain **L0 pure** in this PR; next thin step is multi-test re-export of those contract unit cases via deployed `HeadstashSuite`.
- **SWAP** expanded `private_dex_seams` (inventory **23**); E2E-10..13 stay L0 via `demo-e2e-l0`.
- **COMPOSE** landed `fixtures/compose_seams` (mint→SEAM→swap); included in `demo-e2e-l0`.

## Risks / follow-ups

| Item | Notes |
|------|-------|
| Suite → CW BridgeMintNote multi-test | Contract surface green in `cw-headstash`; suite deploy + msg call not wired this round |
| Claim fixture not exported from suite_backed_claim_pair | Synthetic builder only; real export when H1 green |
| Dex inventory 23 | SWAP r2 SEAM compose — harness maps E2E-10..13 still L0 |
| Path deps from test-press → `docs/plans/...` | Intentional until fixtures promoted to workspace members |
| L3 dual container | Stub recipe only |

---

## Parent-agent summary

Round-2 HARNESS: **L0 suite wiring green path**. Public `hinge_happy_fixture` + `zk-test-press` harness re-exports implement `assert_policy_bridge_mint_happy`, `assert_h1_spent_only_reject`, `assert_double_mint_reject`, SEAM compose, and DEMO-PATH claim fixture load/build (**no prove**). `just demo-e2e-l0` aggregates pure crates (bridge/dex/seam/**compose**) + harness. Inventory **18 / 23 / 6 / ~7**. L1 mock ZK vs `demo-h1` documented. No full ict-rs dual-container (smoke stub only). One mint remains **`cw-headstash`**.
