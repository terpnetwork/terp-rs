# ROUND3-HARNESS-E2E

| Field | Value |
|-------|--------|
| **Agent** | HARNESS |
| **Round** | 3 |
| **Date** | 2026-07-20 |
| **Scope** | L1 real suite methods on cw-orch `Mock` — deploy `cw-headstash`, execute bridge mint |
| **Frozen** | One mint: **`cw-headstash`** only; extend `PrivateBridgeSuite` / `HeadstashSuite` |
| **SSOT** | [`../E2E-HARNESS-PLAN.md`](../E2E-HARNESS-PLAN.md), [`ROUND2-HARNESS.md`](./ROUND2-HARNESS.md), [`ROUND2-BRIDGE.md`](./ROUND2-BRIDGE.md) |

---

## Delivered

### 1. Contract public fixtures (shared unit + suite)

| Surface | Path | Role |
|---------|------|------|
| `happy_bridge_mint_world()` | `contracts/cw-headstash/src/bridge.rs` | Snapshot + cfg + claim + asset (Binary CosmWasm types); `mock_verify=true` |
| `mock_bridge_proof_bytes()` | same | Non-empty proof blob for production mock path |
| `generate_test_wavs_proof(n)` | `contracts/cw-headstash/src/wavs.rs` | Real BLS12-381 PoP for multi-test / Mock instantiate |

Unit tests in `bridge` re-use `happy_bridge_mint_world` (no second label set).

### 2. L1 harness world builders

| Surface | Path | Role |
|---------|------|------|
| `BridgeL1World` | `test-press/src/harness/bridge_l1.rs` | Happy world + H-1 mutation + `from_fixture_doc` (Domain B JSON → Binary) |
| Domain B JSON fixture | `test-press/src/harness/bridge_mint_fixture.rs` | L0 load/build (sibling Round-3 work); suite converts via `BridgeL1World::from_fixture_doc` |

### 3. `PrivateBridgeSuite` L1 methods (real execute)

| Method | E2E | Behavior |
|--------|-----|----------|
| `upload_and_instantiate_mint` | setup | Upload **only** `cw-headstash` + instantiate ExistingFungible + WAVS (no manifold/circuit) |
| `configure_bridge` / `configure_bridge_happy` | setup | Owner: `SetBridgeCfg`, `SetReflectionSnapshot`, optional `RegisterAsset` |
| `execute_bridge_mint` | — | `ExecuteMsg::BridgeMintNote` |
| `query_is_bridge_minted` | — | `QueryMsg::IsBridgeMinted` |
| **`e2e_bridge_mint_happy`** | E2E-01 | Configure + mint + assert minted |
| **`e2e_bridge_double_mint_reject`** | E2E-02 | Second mint → already minted |
| **`e2e_unregistered_asset_reject`** | E2E-06 | No RegisterAsset → unmapped |
| **`e2e_h1_spent_only_reject`** | E2E-03 | spent_only → H-1 / burn set |
| `e2e_bridge_mint_from_fixture` | E2E-01 | Domain B JSON/synthetic → Binary world → CW mint |

Execute uses **raw** `ExecuteMsg` / `QueryMsg` on `HeadstashContract` (cw-orch). `ExecuteFns` remains available on the enum when `interface` is on.

### 4. Tests

| Location | What |
|----------|------|
| `suites/private_bridge.rs` `#[cfg(test)]` | L0 wrappers + **4 L1 Mock tests** (`l1_e2e_*`) |
| `harness/bridge_l1.rs` tests | Happy shapes + fixture→world convert |
| Sibling: `cw-headstash` `tests/test_bridge_e2e.rs` | Multi-test ContractWrapper path (CONTRACT Round-3) |

### 5. Just recipes

| Recipe | Where |
|--------|-------|
| `just demo-e2e-l1` / `e2e-l1` | `crates/headstash/justfile` — bridge unit + multi-test e2e + suite L1 Mock |
| `e2e-l1` | `docs/plans/spectrum/justfile` — delegates to headstash |

---

## Deploy path notes

```text
Mock::new(sender)
  → set_balance(uterp)
  → PrivateBridgeSuite::new(chain)
  → upload_and_instantiate_mint(&[Coin])   // ExistingFungible + generate_test_wavs_proof(1)
  → e2e_bridge_mint_happy()                // SetBridgeCfg / RegisterAsset / Snapshot / BridgeMintNote
```

- **Why not full `HeadstashSuite::deploy_on`?** Circuit upload + manifold not required for bridge mint surface; CLARITY still satisfied (one `cw-headstash` code_id).
- **Why `mock_verify: true`?** Integration / library builds drop `cfg!(test)` on the contract crate; mock verify only opens when flag is set. Proof must be non-empty.
- **Why real WAVS?** Instantiate always runs BLS PoP; empty `poos` fails `verify()`.

---

## Files created / changed

| File | Action |
|------|--------|
| `contracts/cw-headstash/src/bridge.rs` | **changed** — public `happy_bridge_mint_world`, `mock_bridge_proof_bytes` |
| `contracts/cw-headstash/src/wavs.rs` | **changed** — public `generate_test_wavs_proof` |
| `contracts/cw-headstash/src/lib.rs` | **changed** — unit `valid_wavs_proof` delegates to generator |
| `test-press/src/harness/bridge_l1.rs` | **created** — `BridgeL1World` + fixture convert |
| `test-press/src/harness/mod.rs` | **changed** — export `bridge_l1` under `interface` |
| `test-press/src/suites/private_bridge.rs` | **changed** — L1 methods + Mock tests |
| `crates/headstash/justfile` | **changed** — `demo-e2e-l1` / `e2e-l1` |
| `docs/plans/spectrum/justfile` | **changed** — `e2e-l1` delegate |
| `docs/plans/spectrum/agents/ROUND3-HARNESS-E2E.md` | **created** — this report |

---

## Suggested parent verification

```bash
cd crates/headstash

# Contract unit + multi-test e2e
cargo test -p cw-headstash --lib bridge::
cargo test -p cw-headstash --test test_bridge_e2e

# Suite L1 Mock
cargo test -p zk-test-press --lib suites::private_bridge:: --features interface -- --nocapture
cargo test -p zk-test-press --lib harness::bridge_l1:: --features interface

# Aggregate
just demo-e2e-l1
```

---

## Cross-agent

| Agent | Relation |
|-------|----------|
| **CONTRACT e2e** (`ROUND3-CONTRACT-E2E.md`) | Multi-test `test_bridge_e2e.rs` — same cases via `App`/`ContractWrapper`; harness suite uses cw-orch `Mock` + interface |
| **FIXTURE** (`bridge_mint_fixture` / golden JSON) | Suite converts JSON → Binary via `BridgeL1World::from_fixture_doc` |
| **COMPOSE/SWAP** | Still L0 pure for product burn→swap; L1 mint is headstash-only |

---

## Explicit non-goals this round

- Full manifold + circuit `deploy_on` for bridge mint
- Real SP1 / LC IMT verify (mock only)
- ict-rs dual-container / Daemon L3
- CosmWasm private DEX swap execute

---

## Risks / follow-ups

| Item | Notes |
|------|-------|
| Session had no shell for in-loop `cargo test` | Parent must confirm green |
| `demo-e2e-l0` also runs full `private_bridge` module tests | Now includes L1 deploy cases when `interface` is on — intentional |
| HeadstashDeployData::local_default still has empty WAVS | Full suite deploy_on still needs fixed WAVS for production use |
| Mint emits attrs / note_out JSON only | No bank credit / note tree write (product surface Round-2 design) |

---

## Parent-agent summary

Round-3 HARNESS: **L1 Mock bridge mint is real**. `PrivateBridgeSuite` uploads/instantiates **`cw-headstash`**, runs owner `SetBridgeCfg` / `RegisterAsset` / `SetReflectionSnapshot`, then `BridgeMintNote` with mock verify. Methods: `e2e_bridge_mint_happy`, `e2e_bridge_double_mint_reject`, `e2e_unregistered_asset_reject`, `e2e_h1_spent_only_reject`, plus fixture-driven mint. Fixtures public on contract + `BridgeL1World` Binary converters. `just demo-e2e-l1` aggregates unit + multi-test + suite. One mint remains **`cw-headstash`**.
