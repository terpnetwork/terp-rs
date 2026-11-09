# Agent brief — DEMO corridor E2E automation

**Track:** E2E  
**SPEC:** `docs/plans/spectrum/DEMO-CASHAPP-ZEC-CORRIDOR.md` §5–6  
**Primary:** `crates/headstash/test-press`, spectrum fixtures

## Goal

Automate the **successful deposit → bridge mint → oracle-bound swap** workflow for the CashApp→ZEC corridor, with **Simulated** asset backend by default and APIs open for **LC live** later.

## Deliverables

1. Harness module e.g. `harness/cashapp_zec_corridor.rs` (or under `private_bridge` suite methods).
2. Workflow W0–W7 from SPEC:
   - Create `DepositIntentV0` (use corridor crate types when present; minimal local struct if racing)
   - Fresh deposit addr
   - **Sim deposit** success
   - Bridge mint (L0 pure and/or L1 Mock `e2e_bridge_mint_happy` with intent-aligned owner_binding)
   - Optional note persist
   - Swap with oracle bound from intent
   - Assert dest + min_out
3. `CorridorAssetBackend::Simulated` path green in CI (no Docker).
4. Stub or trait hook for `LightClient { MockAttestation | Live }` — compile + `todo`/ignore live tests OK.
5. Reject tests: I2 mid too low, I1 wrong dest (at least one each).
6. Commands documented in SPEC or `NOTE-PERSIST` / E2E plan pointer.

## Constraints

- Prefer `PrivateBridgeSuite` + compose/private_dex fixtures.
- Mock ZK OK.
- Do not claim joint anvil+LC green.
- Oracle bound_only — never mint from oracle.

## Done when

```bash
cargo test -p zk-test-press --lib cashapp_zec --features 'interface,l0-seams'
# or fixture-level equivalent all green
```

Happy deposit path + at least two reject cases.

## Out of scope

- dao-dao UI (UI track).
- Full ICS-08 / real ZEC LC (document hook only).
