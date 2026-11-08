# ROUND3-CONTRACT-E2E

| Field | Value |
|-------|--------|
| **Agent** | CONTRACT / BRIDGE e2e |
| **Round** | 3 |
| **Date** | 2026-07-20 |
| **Status** | multi-test e2e landed (`test_bridge_e2e.rs`) |
| **SSOT** | [`CLARITY-cw-headstash-router-and-asset-registry.md`](../CLARITY-cw-headstash-router-and-asset-registry.md), [`ROUND2-BRIDGE.md`](./ROUND2-BRIDGE.md) |
| **Code** | [`crates/headstash/contracts/cw-headstash/tests/test_bridge_e2e.rs`](../../../../crates/headstash/contracts/cw-headstash/tests/test_bridge_e2e.rs) |

> **Product:** `cw-headstash` is the mint/router. This round exercises the **real** `execute` / `query` path via `cw-multi-test` `ContractWrapper` (not pure `authorize_bridge_mint_pure` alone). Mock verify only; no SP1 / anvil.

---

## 1. What landed

### Integration binary

| Path | Role |
|------|------|
| `contracts/cw-headstash/tests/test_bridge_e2e.rs` | Multi-test e2e suite (standalone integration crate) |

### Deploy pattern

1. `App::default()` + store `ContractWrapper::new(execute, instantiate, query).with_reply(reply)`
2. Instantiate with **`TokenStrategy::ExistingFungible("ubridge")`** + prefund  
   - Avoids tokenfactory Stargate `CreateDenom` (not mocked in multi-test)
3. Deterministic BLS WAVS PoO (1 operator, fixed `Fr` scalar) so instantiate pairing check passes
4. Owner corridor msgs then public `BridgeMintNote`

### Why `mock_verify: true` is required

Integration tests compile the **library** without `cfg(test)`.  
`mock_verify_bridge_proof` only allows mock when `BridgeCfg.mock_verify || cfg!(test)`.  
Empty proofs are also rejected outside `cfg(test)` → e2e always sends a non-empty proof blob.

Unit tests in `bridge.rs` still rely on `cfg!(test)` for direct `execute_*` calls.

### Interface / schema

No code change required this round:

- `ExecuteMsg` / `QueryMsg` already include bridge variants (`SetBridgeCfg`, `BridgeMintNote`, `IsBridgeMinted`, …)
- `#[cfg_attr(feature = "interface", derive(cw_orch::ExecuteFns/QueryFns))]` already on those enums
- `src/interface.rs` uses `InstantiateMsg, ExecuteMsg, QueryMsg` — picks up new variants when `interface` is enabled

---

## 2. Test cases

| Test | Path exercised | Expect |
|------|----------------|--------|
| `e2e_bridge_mint_happy_path_is_minted` | owner setup → `BridgeMintNote` → `IsBridgeMinted` | ACCEPT; attrs `action=bridge_mint_note`, `dex_consumable=true`, `rcm_flag=1`; note_out decodes to DEX-consumable `NoteOutResult`; `IsBridgeMinted=true` |
| `e2e_bridge_double_mint_same_nu_reject` | second `BridgeMintNote` same ν | REJECT `already minted`; still minted once |
| `e2e_bridge_h1_spent_only_reject` | `spent_only=true`, `in_burn_set=false` | REJECT H-1 / burn set; `IsBridgeMinted=false` |
| `e2e_bridge_unregistered_asset_reject` | cfg+snapshot, **no** `RegisterAsset` | REJECT unregistered/unmapped |
| `e2e_bridge_update_reflection_then_mint` | `UpdateReflection` instead of full snapshot | tip query + happy mint |
| `e2e_bridge_not_configured_reject` | no cfg/snapshot | REJECT not configured |

Owner setup (happy) always:

1. `SetBridgeCfg { mock_verify: true, dest_domain, K=6, lag=64 }`
2. `SetReflectionSnapshot` (or `UpdateReflection` in one case)
3. `RegisterAsset` with mapped terp asset id (`terp_asset_id_from_tacit`)

Fixture constants match `bridge::tests::fixture` (same labels / heights) so pure unit + multi-test stay aligned.

---

## 3. How to run

From the headstash workspace:

```bash
cd crates/headstash

# Bridge multi-test e2e only
cargo test -p cw-headstash --test test_bridge_e2e

# All integration tests for the package
cargo test -p cw-headstash --tests

# Lib unit tests (includes bridge pure + mock-deps execute tests)
cargo test -p cw-headstash --lib bridge::

# Combined filter
cargo test -p cw-headstash bridge
```

Expected e2e binary: **6** tests, all green, no network / SP1 / anvil.

### Agent environment note

This agent session had **no interactive shell** to execute `cargo test` in-loop (same constraint as ROUND2-COMPOSE/SWAP). Parent should confirm green with the recipe above.

---

## 4. Gaps / non-goals (explicit)

| # | Gap | Notes |
|---|-----|-------|
| 1 | Real SP1 / LC IMT verify | still `mock_verify` + claim membership bools |
| 2 | Bank / note-tree write on mint | attrs only (`note_out` base64 JSON); no pool append |
| 3 | External asset registry **query** at mint | addr hook stored; mint resolves **internal** map only |
| 4 | Tokenfactory NewFungible multi-test | e2e uses ExistingFungible to skip CreateDenom Stargate |
| 5 | Factory (`cw-headstash-manifold`) deploy path | direct ContractWrapper instantiate (faster / deterministic) |
| 6 | `test-press` PrivateBridgeSuite L1 multi-test re-export | still L0 pure; suite can later call these msgs |
| 7 | Full 382-byte SEAM wire event | contract emits JSON `NoteOutResult` attr (ROUND2 posture) |
| 8 | Pre-existing integration tests (`test_headstash` etc.) | some still use empty WAVS mocks; **not** fixed this round |

---

## 5. Risks

| Risk | Mitigation |
|------|------------|
| Integration build drops `cfg!(test)` mock path | force `mock_verify: true` + non-empty proof in e2e |
| WAVS pairing fails under multi-test API | fixed BLS key + same MockApi hash-to-G2 as unit tests |
| Double-mint race | `BRIDGE_MINTED` + shared `NULLIFIERS` under `bridge:02:{hex}` |
| Registry as mint authority | registry resolve only; gates enforce burn-set / H-1 |

---

## 6. Parent-agent summary

**ROUND3 contract e2e:** multi-test suite `tests/test_bridge_e2e.rs` deploys real `cw-headstash` via `ContractWrapper`, owner-configures bridge corridor (`SetBridgeCfg` mock_verify, reflection, `RegisterAsset`), then exercises `BridgeMintNote` happy path + `IsBridgeMinted`, double-mint reject, H-1 spent-only reject, unregistered asset, `UpdateReflection` mint, and not-configured reject. Interface msgs already covered. No SP1/anvil. Parent: `cargo test -p cw-headstash --test test_bridge_e2e`.
