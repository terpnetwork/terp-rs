# ROUND3 — Tacit anvil → headstash fixture glue

| Field | Value |
|-------|--------|
| **Agent** | TACIT-GLUE / BRIDGE glue |
| **Round** | 3 |
| **Date** | 2026-07-20 |
| **Status** | golden mint packet + loader landed; optional anvil annotate |
| **SSOT** | Domain B [`SPEC-tacit-bridge-mapping.md`](../SPEC-tacit-bridge-mapping.md) §9, `cw-headstash` `BridgeMintClaimPublic` |
| **Code homes** | [`fixtures/bridge_mint_claim_happy.v1.json`](../fixtures/bridge_mint_claim_happy.v1.json), [`fixtures/emit_bridge_mint_fixture.sh`](../fixtures/emit_bridge_mint_fixture.sh), [`test-press/src/harness/bridge_mint_fixture.rs`](../../../../crates/headstash/test-press/src/harness/bridge_mint_fixture.rs) |

> **Scope freeze:** real script + **mock mint packet** (no SP1, no full ict-rs dual-container). Anvil roundtrip is **documented + optional**; default degrades to pre-baked golden.

---

## 1. What landed

### Golden fixture

| Path | Role |
|------|------|
| `docs/plans/spectrum/fixtures/bridge_mint_claim_happy.v1.json` | `bridge-mint-claim-public-v1` document: `snapshot` + `cfg` + `asset` + `claim` + `proof_hex` + `field_docs` |
| Schema | Compatible with `cw-headstash::bridge::BridgeMintClaimPublic` (hex digests instead of CosmWasm `Binary` base64) |

Labels match:

- `bridge_auth_seams::hinge_happy_fixture`
- `cw-headstash::bridge::happy_bridge_mint_world`
- ROUND2-BRIDGE unit fixture

### Emitter script

`docs/plans/spectrum/fixtures/emit_bridge_mint_fixture.sh`

| Flag | Behavior |
|------|----------|
| (default) | Print/copy golden; docs anvil how-to on stderr |
| `--regenerate` | Recompute SHA-256 label hashes (python3) → rewrite golden |
| `--try-anvil` | If `anvil`/`cast`/`forge`/`node` present, run `crates/tacit/tests/evm-confidential-anvil-roundtrip.sh` and attach `source.tacit_anvil` meta; on failure **degrade** to golden claim |
| `--out PATH` | Write elsewhere |

Thin wrap: `crates/headstash/scripts/emit_bridge_mint_fixture.sh`.

### How to run Tacit anvil (documented)

```bash
cd crates/tacit
# Requires: anvil, cast, forge, node + dapp deps
bash tests/evm-confidential-anvil-roundtrip.sh
```

This proves **EVM confidential mint** (module prover → `mint` tx → supply/noteStatus). It does **not** produce Bitcoin-reflection Domain B burn membership. The glue therefore maps **known hinge-happy shapes** into `BridgeMintClaimPublic` for Terp mock LC tests.

### Harness loader

`crates/headstash/test-press/src/harness/bridge_mint_fixture.rs`

| API | Role |
|-----|------|
| `load_bridge_mint_fixture(path)` | JSON → policy validate |
| `build_happy_bridge_mint_fixture()` | Synthetic same labels |
| `load_or_build_happy_bridge_mint_fixture()` | Golden if present, else synthetic |
| `authorize_from_bridge_mint_fixture` | L0 `authorize_bridge_mint_apply` |
| `assert_fixture_matches_hinge_happy` | Golden ≡ pure hinge public fields |

Policy checks: claim_id re-derive (A14), domain_binding re-derive (C §3.1), snapshot/claim root pins, asset map, H-1 flags (`in_burn_set`, not `spent_only`), `mock_verify=true`.

### Suite wiring

`PrivateBridgeSuite`:

- `load_bridge_mint_fixture` / `build_bridge_mint_fixture`
- `e2e_burn_to_mint_fixture(path)` loads golden/synthetic then L0 authorize (+ optional LC mock height gate)
- `e2e_bridge_mint_from_fixture(path)` — L1 CW: JSON → `BridgeL1World::from_fixture_doc` → `BridgeMintNote`

L1 Binary conversion: `BridgeL1World::from_fixture_doc` (same labels as `happy_bridge_mint_world`).

---

## 2. Recipes

```bash
# Spectrum-only emit golden
cd docs/plans/spectrum && just demo-bridge-fixture

# Headstash: emit + harness unit tests
cd crates/headstash && just demo-bridge-fixture

# Optional live anvil meta (degrades without tools)
cd crates/headstash && just demo-bridge-fixture-anvil

# Included in L0 aggregate
cd crates/headstash && just demo-e2e-l0   # cargo test harness:: covers bridge_mint_fixture
```

---

## 3. Domain B field map (fixture `field_docs`)

| Claim field | Domain B / C meaning |
|-------------|----------------------|
| `source_chain_tag` | Source chain tag |
| `tacit_asset_id` | Foreign asset id |
| `value_u64` | Opened burn value |
| `nullifier` | Burned note ν (once-per-ν) |
| `dest_commitment` | Burn-bound destCommitment |
| `claim_id` | Re-derived claim id |
| `source_pool_root` / `source_burn_root` | Pool + **bridge-burn** root pins |
| `source_height` | Inclusion height |
| `domain_binding` | C §3.1 bind digest |
| `in_burn_set` | Mock LC stand-in for burn IMT (H-1) |
| `spent_only` | Explicit H-1 reject tests |
| `rcm` | DEX consumability opening |
| snapshot `burn_root` | Mint authority root (≠ spent alone) |
| `cfg.mock_verify` | Round-2/3 stub — **not** Tier-0 |

---

## 4. Honesty / non-goals

| Item | Status |
|------|--------|
| Real SP1 bridge proofs | **Not** this round |
| Real Bitcoin reflection LC / IMT under burn root | Mock bools only |
| Full ict-rs Anvil+Terp dual container | **Not** this round (E2E-19 stub remains) |
| Anvil output = Domain B burn packet | **No** — EVM confidential path only; claim is hinge-happy golden |
| Oracle / registry mint balances | Still forbidden |

---

## 5. Verify

```bash
bash docs/plans/spectrum/fixtures/emit_bridge_mint_fixture.sh --regenerate
cd crates/headstash && cargo test -p zk-test-press --lib harness::bridge_mint_fixture::
cd crates/headstash && cargo test -p zk-test-press --lib suites::private_bridge::bridge_mint_fixture --features interface
# or: just demo-bridge-fixture
```

---

## 6. Parent-agent summary

Round-3 Tacit glue: **golden `bridge-mint-claim-public-v1` fixture** aligned with hinge + `BridgeMintClaimPublic`, emitter script documenting anvil roundtrip with **graceful degrade**, harness loader used by e2e/suite, `just demo-bridge-fixture` in spectrum + headstash. No SP1, no dual-container.
