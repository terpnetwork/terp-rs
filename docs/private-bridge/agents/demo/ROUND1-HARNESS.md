# ROUND1-HARNESS

> **Supersession (2026-07-20):** Deploy **one** mint wasm — `cw-headstash` as product router (claim + bridge mint + future note entry). Registry internal/external. See `CLARITY-cw-headstash-router-and-asset-registry.md`.

**Agent:** HARNESS · **Date:** 2026-07-20 · **Scope:** curation + thin scaffold only  
**SSOT plan:** [`../E2E-HARNESS-PLAN.md`](../E2E-HARNESS-PLAN.md)

---

## Inventory table

| Path | Kind | What it proves | Layer | Gaps vs joint private-bridge e2e |
|------|------|----------------|-------|----------------------------------|
| `docs/plans/spectrum/fixtures/bridge_auth_seams` | pure Rust (9 tests) | H-1 burn≠spend, conservation, double mint, tip/conf, domain, asset map (T1–T7 + extras) | L0 | No CosmWasm, no Anvil, no LC |
| `docs/plans/spectrum/fixtures/private_dex_seams` | pure Rust (12 tests) | swap_happy, min_out, oracle stale/slippage, nullifier, wrong asset, oracle cannot mint | L0 | No CW DEX contract |
| `docs/plans/spectrum/fixtures/seam_note_out` | pure Rust (6 tests) | `SeamNoteOutV0` structural encode/schema | L0 | Not wired to claim/mint msgs |
| `crates/headstash/test-press/src/suites/headstash.rs` | cw-orch suite | `HeadstashSuite`: headstash + manifold + circuit; `Deploy` store/deploy | L1 | No bridge mint; H1 prove red |
| `crates/headstash/test-press/src/suites/no_rick.rs` | cw-orch suite | Simpler circuit suite pattern | L1 | Not on bridge path |
| `crates/headstash/test-press/src/suite.rs` | composed suite | `TestPressSuite` no_rick + headstash | L1 | Partial `Deploy` (`todo!`) |
| `crates/headstash/test-press/src/suites/private_bridge.rs` | **scaffold (new)** | Types + method stubs + LC mock gate helper | L1→L4 | Methods `todo!` until R2 |
| `crates/headstash/justfile` `demo-policy` / `demo-h1` / `demo-keys` | recipes | V0 policy, V1 MockProver, keygen | L1 | H1 last-mile red |
| `crates/headstash/docs/circuit/DEMO-PATH.md` | doc | Claim fixture schema + demo spine | — | Fixture export “next” |
| `cw-headstash` lib / multi-test | contract tests | distro roots, process_headstash policy | L1 | Mock ZK until prove green |
| `crates/tacit/tests/evm-confidential-anvil-roundtrip.sh` | shell e2e | Anvil deploy + prover mint + RPC | L2 | EVM-only; no Terp |
| `crates/tacit/contracts/test/*.t.sol` | forge | Confidential pool/factory/farm/poseidon | L2 | EVM-only |
| `crates/tacit/tests/*signet*.mjs`, `bridge-*.mjs` | node e2e | Signet/testnet confidential + bridge surfaces | L2 | Not Terp mint packet |
| `crates/ict-rs/ict-rs` `AnvilChain`, `TestEthChain` | container | Anvil in Docker, accounts, eth send | L2/L3 | No ConfidentialPool recipe |
| `crates/ict-rs/ict-rs/tests/anvil_tests.rs` | integration | Anvil start/funds (`ethereum` feature) | L2 | Unit-level |
| `crates/ict-rs/examples/hashmerchant.rs` | Docker e2e | **Anvil + Terp + sidecar** VE/root path | L3 | Oracle/VE **not** conservation mint — best dual-container **template** |
| `crates/ict-rs/examples/headstash.rs` | Docker e2e | Terp zk image + wasm + claim lifecycle sketch | L3 | Keys/wasm paths; prove unfinished |
| `crates/ict-rs/examples/ibc_transfer*.rs`, polytone, loyalty_* | Docker e2e | IBC dual-chain, dual-path claims, gas tables | L3 | Cosmos↔Cosmos or loyalty — not Tacit bridge |
| `crates/ict-rs/ict-rs-cw-orch` | glue | `daemon_builder_from_chain` → cw-orch Daemon | L3 | CosmWasm only; no Anvil handle |
| `crates/ict-rs/ict-rs/src/spec.rs` builtins | config | gaia, osmosis, **terp**, juno, akash, **anvil** | L3 | Ready names; bridge compose missing |
| Domain SPECs A–D + FLOW + matrix | docs | Seam freeze, reject codes, C0–C9 cases | — | Objects not all landed |
| Crosslink LC / 08-wasm / Eth LC / TM ICS07 | monorepo LCs | Per Domain C inventory | L4 real | Not private-bridge e2e wired |

**Summary:** L0 pure seams are green. L1 headstash suite exists but claim prove is red. L2 Tacit anvil is green in isolation. L3 dual Anvil+Terp exists only for **hashmerchant VE**, not bridge mint. L4 hinge is SPEC + unit LCs; **mock first** for compose.

---

## Proposed architecture (L0–L4)

```text
L4  LC hinge — mock attestation first (tip, K conf, freeze); real Crosslink / reflection PVs later
L3  ict-rs — Anvil container + terpd (local-zk) + optional sidecar; cw-orch via ict-rs-cw-orch
L2  Anvil-only Tacit — wrap `evm-confidential-anvil-roundtrip.sh` / forge / cast
L1  cw-orch Mock / multi-test — HeadstashSuite + PrivateBridgeSuite; mock ZK OK
L0  Pure seams — bridge_auth_seams, private_dex_seams, seam_note_out (always-on CI)
```

**Glue rules**

| Concern | Choice |
|---------|--------|
| Containers | ict-rs (`ChainSpec` / `AnvilChain` / `CosmosChain`) |
| CosmWasm scripting | cw-orch suites (`Deploy`, `ZkCwEnv`) |
| Tacit EVM | foundry anvil + existing Tacit scripts |
| Dual-container template | Clone `examples/hashmerchant.rs` topology; **replace** VE mint authority with Domain B mint packet + L4 mock |
| ZK | Mock verify on L1 until H1 green; real prove nightly later |
| Opcodes | Only Domain B map; no invention |

Full writeup: [`E2E-HARNESS-PLAN.md`](../E2E-HARNESS-PLAN.md).

---

## Suite sketch (types, methods)

**Module:** `crates/headstash/test-press/src/suites/private_bridge.rs` (stub landed)

```text
PrivateBridgeSuite<Chain: ZkCwEnv>
  headstash: HeadstashSuite<Chain>
  tacit_rpc: Option<String>
  fixtures_dir: Option<PathBuf>
  lc_mock: Option<LcMockConfig { tip_height, confirmations_required, frozen }>

PrivateBridgeDeployData { headstash, fixtures_dir, tacit_rpc, lc_mock }

Methods:
  new / with_tacit_rpc / with_fixtures_dir / with_lc_mock
  Deploy::store_on | deploy_on  → delegates HeadstashSuite
  load_claim_fixture_and_process   // E2E-08  TODO
  e2e_burn_to_mint_fixture         // E2E-01  TODO
  assert_double_mint_reject        // E2E-02  TODO
  assert_ordinary_spend_not_burn   // E2E-03  TODO
  apply_swap_fixture               // E2E-10  TODO → L0 dex seams
  assert_oracle_cannot_mint        // E2E-13  TODO
  lc_mock_allows_mint(height, now) // L4 helper (implemented)
```

**ict-rs (not created round 1):** future `examples/private_bridge_compose.rs` with features `docker` + `ethereum`.

**Fixture schemas:** DEMO-PATH claim JSON; Domain B §9 mint packet; frozen `SeamNoteOutV0`.

---

## Test case catalog (E2E-xx)

| ID | Scenario | Matrix | Min → Target | Primary seed |
|----|----------|--------|--------------|--------------|
| E2E-01 | burn → mature tip → bridge_mint note | C0.1 | L0 → L3+L4 | `t1_happy_path_mint` |
| E2E-02 | double mint same ν reject | C2.2 | L0 → L1/L3 | `t2_double_mint_reject` |
| E2E-03 | ordinary spend not burn (H-1) | — | L0 → L1/L3 | `t3_ordinary_spend_not_burn` |
| E2E-04 | immature conf / tip lag | C3.1 | L0 → L4 | `t4_immature_confirmation` |
| E2E-05 | value mismatch | conservation | L0 → L1 | `t5_value_mismatch` |
| E2E-06 | unmapped asset | C4.3 | L0 → L1 | `t6_unmapped_asset` |
| E2E-07 | domain mismatch | C4.1 | L0 → L1 | `t7_domain_mismatch` |
| E2E-08 | headstash claim policy | C1.1 / C2.1 | L1 → L3 | `demo-policy` / Part T |
| E2E-09 | note schema consumable | C7.* | L0 → L1 | `seam_note_out` |
| E2E-10 | swap happy | C1.2 | L0 → L1* | `swap_happy` |
| E2E-11 | min_out fail | C5.4 | L0 → L1* | `min_out_fail` |
| E2E-12 | oracle stale/missing | C5.1–2 | L0 → L1* | `oracle_stale` |
| E2E-13 | oracle cannot mint | C6.* | L0 → L1/L3 | pure + matrix |
| E2E-14 | LC frozen | C3.2 | L4 mock → real | Domain C |
| E2E-15 | wrong inclusion root | C3.3 | L0/L4 → L4 | membership |
| E2E-16 | lag then tip advances | C3.4 | L4 mock → L3+L4 | stateful hinge |
| E2E-17 | unshield / bridge_burn exit | C0.2–3 | L1* → L3 | exit msgs TBD |
| E2E-18 | Tacit anvil confidential RT | — | L2 | shell script |
| E2E-19 | dual container smoke | — | L3 | hashmerchant topology |
| E2E-20 | trust-tier honesty | C9 | doc → L3 meta | labeling |

\* Until CosmWasm private DEX / exit contracts exist, L1* re-exports pure seams.

**Progression:** matrix §13 — one full ingress path + nullifiers + oracle non-authority + shared schema.

---

## Files created/changed

| File | Action |
|------|--------|
| `docs/plans/spectrum/E2E-HARNESS-PLAN.md` | **created** — full L0–L4 plan, catalog, tooling |
| `docs/plans/spectrum/agents/ROUND1-HARNESS.md` | **created** — this report |
| `crates/headstash/test-press/src/suites/private_bridge.rs` | **created** — suite stub + LC mock helper |
| `crates/headstash/test-press/src/suites/mod.rs` | **changed** — `pub mod private_bridge` under `interface` |

**Not created (intentionally):** ict-rs `private_bridge_compose` example, Docker compose files, real LC client, ConfidentialPool ict deploy recipe.

---

## Clarity questions for team (numbered, must-answer for round 2)

1. **Ingress surface for L3 demo mint:** Is the first joint green path **(a)** Tacit `ConfidentialPool` `OP_BRIDGE_BURN` on Anvil → mock hinge → CosmWasm bridge-mint, **(b)** Headstash claim-only (no Anvil) then bolt Anvil later, or **(c)** both in parallel with separate case IDs? *Recommendation: (b) for L1 CI + (a) for L3 weekly.*

2. **CosmWasm mint object:** Does a `cw-bridge-mint` (or headstash extension msg) already have an owner crate/name, or is L1 mint limited to pure `bridge_auth_seams` + fixture JSON until Domain B contract lands?

3. **Mock ZK policy:** Confirm mock verify is allowed for E2E-08 on PR CI, with real H1 prove gated nightly / `demo-h1` only — yes/no?

4. **L4 mock trust labeling:** Accept mock hinge as **demo/degraded** only (matrix C9), never Tier-0, even if E2E-01 goes green on mock — yes/no?

5. **Confirmation constant:** Pin numeric `confirmations_required` for Anvil local (e.g. 1 or 3) vs production Tacit `REFLECTION_CONFIRMATIONS` — separate fixtures or one env override?

6. **Asset registry bootstrap:** Who owns the L1/L3 `asset_id` registry fixture (Domain B map) — harness JSON, genesis, or contract instantiate?

7. **Private DEX on Terp timeline:** Is E2E-10/11/12 allowed to stay L0-only for multiple sprints, or is a multi-test AMM stub required before L3 compose?

8. **ict-rs image pin:** Canonical image for Terp in compose e2e — `terpnetwork/terp-core:local-zk` only, or also non-zk for faster smoke (E2E-19)?

9. **Tacit deploy in Docker:** Prefer **(a)** host `forge script` against mapped Anvil port (reuse shell), **(b)** bake foundry into ict-rs sidecar, or **(c)** predeployed state volume?

10. **Suite home:** Keep `PrivateBridgeSuite` in `zk-test-press` (as scaffolded) vs new crate under `docs/plans/spectrum` or `ict-rs` examples only?

11. **Reject code freeze:** Are matrix placeholder codes (`E_NULLIFIER`, `E_LC_LAG`, `E_ORACLE_MINT`, …) frozen for harness assertions, or wait for contract error enums?

12. **Hashmerchant boundary:** Confirm hashmerchant Anvil+Terp e2e remains **oracle/VE only** and must never be cited as bridge-mint conservation proof — yes/no?

13. **CI budget:** Max wall-clock for default PR job (L0 only? L0+L1 mock? exclude Docker?) so harness recipes can be split (`just e2e-l0` / `e2e-l3-nightly`).

14. **Exit path priority:** Unshield (C0.2) vs bridge_burn cross_out (C0.3) first for E2E-17?

---

## Risks / blind spots

| Risk | Why it hurts | Mitigation |
|------|--------------|------------|
| H1 composite still red | Blocks real prove claim e2e | Mock ZK on L1; keep `demo-policy` green path |
| No CW bridge-mint yet | E2E-01 cannot leave L0/L1 pure | Suite methods call pure seams until msg exists |
| Confusing hashmerchant with bridge | False Tier-0 / mint authority | E2E-13 + Q12; docs call out VE ≠ mint |
| Mock LC labeled as production | Trust-tier lie | E2E-20 + C9; `LcMockConfig` docs |
| Dual Docker flaky | L3 CI noise | E2E-19 smoke only; full path nightly |
| Opcode drift | Parallel note languages | Domain B SSOT; no new ops in harness |
| Fixture schema drift | Suite vs DEMO-PATH vs SEAM-NOTE-OUT | Single loader; export from suite when green |
| `TestPressSuite` incomplete Deploy | Composition dead-ends | Prefer `PrivateBridgeSuite` over expanding incomplete todos |
| Anvil ConfidentialPool not in ict-rs | L3 burns missing | Host forge against mapped port (Q9a) first |
| Private swap only pure | Compose C1 incomplete | Honest L0 min layer until D contracts |

---

## Suggested round-2 tasks (ordered)

1. **Team answers** Q1–Q14; update this file + plan with decisions.  
2. **`just e2e-l0`** aggregator running three fixture crates + print E2E-id mapping.  
3. **Implement L0 re-export tests** inside `private_bridge` (or thin `#[cfg(test)]`) for E2E-01..07/10..13 without Docker.  
4. **Claim fixture loader** (DEMO-PATH A2) → multi-test/mock process_headstash (E2E-08).  
5. **Document L2 wrap** from spectrum plan → one `just tacit-anvil-rt` (E2E-18).  
6. **L3 smoke example** `private_bridge_compose` or script: Anvil + Terp start/stop only (E2E-19); no mint required.  
7. **L4 mock types** shared crate or fixture: lag/freeze vectors (E2E-04/14/16).  
8. **When mint msg lands:** wire `e2e_burn_to_mint_fixture` at L1 then L3.  
9. **Nightly H1** attach real proof to E2E-08 when green.  
10. **Checkpoint** using matrix §13 template into sprint notes.

---

## Parent-agent summary

Round-1 harness curation complete. **No joint e2e exists**; reuse **L0 pure seams (green)**, **HeadstashSuite / ict-rs dual-container (hashmerchant template)**, and **Tacit anvil shell**. Proposed **L0–L4** stack with **E2E-01..20** mapped to matrix C\* cases. Delivered **`E2E-HARNESS-PLAN.md`**, **`PrivateBridgeSuite` stub**, and **14 clarity questions** blocking round 2. Do not implement full ict-rs network until Q1/Q2/Q8/Q9 answered.
