# ROUND1-META-REVIEW

| Field | Value |
|-------|--------|
| **Agent** | REVIEW (meta) |
| **Date** | 2026-07-20 |
| **Inputs** | `ROUND1-HARNESS.md`, `ROUND1-SWAP.md`, `ROUND1-BRIDGE.md`, `E2E-HARNESS-PLAN.md`, SPECs A–D / SEAM / CLARITY / DEMO-PATH, pure fixtures + `private_bridge.rs` scaffold |
| **Code changes this review** | None (doc only) |
| **Follow-up decisions** | 2026-07-20 — see § Decisions accepted |

---

## Decisions accepted (2026-07-20)

SSOT: [`CLARITY-cw-headstash-router-and-asset-registry.md`](../CLARITY-cw-headstash-router-and-asset-registry.md)

| Topic | Decision | Closes |
|-------|----------|--------|
| **Mint / router home** | **`cw-headstash` is the mint** — unified on-chain router for eligibility claims, **bridge mints**, and future note-entry/swap-related mints | D8, BRIDGE Q6, HARNESS suite home |
| **Separate `cw-bridge-mint`** | **Not** the product default | D8 |
| **Asset registry** | **Internal and/or external**; expand with **cross-chain wiring**; never mint balances | D3 path |
| **Test suites** | Extend Headstash / PrivateBridge around **one** headstash code_id | HARNESS |

Round-2 agents must treat these as frozen unless program revisits.

---

## Executive take (GO / CONDITIONAL / REWORK)

### **CONDITIONAL GO** for Round 2

Round 1 did the right *kind* of work: **pure structural freezes**, inventory honesty, and no fake joint e2e. H-1 (burn ≠ spent), oracle-never-mints, and note/nullifier domain tags are **aligned in spirit** across the three agents and with SPECs.

**Do not REWORK** the round-1 artifacts. **Do not** greenlight “e2e complete,” CosmWasm bridge-mint production, or Halo2 swap circuits until the **shared freeze questions** below are answered and **L0 compose glue** exists.

| Track | Round-1 posture | Gate to call green |
|-------|-----------------|-------------------|
| L0 pure seams | Expanded (counts drift vs inventory; see below) | `cargo test` per fixture crate + one **cross-crate** compose test |
| L1 suite | Stub only (`todo!` methods) | Methods re-export L0 + claim fixture loader |
| L2 Tacit anvil | Inventory only | Thin `just` wrap; no Terp mint claim |
| L3 dual container | Plan only (hashmerchant topology) | Smoke (E2E-19) before mint path |
| L4 LC | BRIDGE `ReflectionSnapshot` rich; HARNESS `LcMockConfig` thin | Unify mock types; never Tier-0 |
| H1 claim MockProver | Still last-mile red (DEMO-PATH) | Mock ZK for L1 CI; real prove nightly |

**Bottom line:** Program-ready for **sequenced Round 2 implementation**, not for marketing Tier-0 bridge or joint anvil+terpd+LC mint.

---

## Cross-agent consistency findings

### Aligned (preserve)

| Concern | Status |
|---------|--------|
| **H-1 burn vs spent** | BRIDGE enforces in legacy T3 + hinge `spent_only`; HARNESS E2E-03; SPEC B/C; CLARITY. **Do not dilute.** |
| **Oracle never mints** | SWAP hard-rejects `oracle_mint_note` / reserve bump; HARNESS E2E-13; BRIDGE A12 reserved; SPEC D/B. |
| **Nullifier domains** | SEAM `0x01` claim / `0x02` bridge burn / `0x03` pool spend not on egress; SWAP NE-4 rejects ingress lineage as pool ν. |
| **Distro Poseidon ≠ pool tree T** | SWAP + CLARITY + DEMO-PATH + SEAM provenance_anchor roles consistent. |
| **Shared anonymity set narrative** | Claim + bridge mint + swap outs → one product tree `T` (or dual+link non-goal). All three agents assume this. |
| **Mock LC honesty** | HARNESS E2E-20 / C9; BRIDGE “label mock”; suite docs. |
| **Hashmerchant ≠ mint** | HARNESS Q12 + risks; correct. |
| **Crypto deferred** | SWAP no K=18; BRIDGE no SP1 guest; HARNESS mock ZK OK — correct prioritization. |

### Divergences / drift risks (fix before parallel code thrash)

| # | Issue | Agents | Severity |
|---|--------|--------|----------|
| D1 | **Two parallel bridge mint type surfaces** | BRIDGE `NoteOutSketch` / `BridgeMintPublic` in `bridge_auth_seams` vs SEAM `from_bridge_mint` / its own `BridgeMintPublic` in `seam_note_out` — no conversion helper, no shared crate | **P0 compose** |
| D2 | **`NoteOutSketch` incomplete for DEX** | Sketch has no `rcm` / `rcm_flag`; `is_dex_consumable` **requires** `rcm_flag == 1`. Happy hinge mint **cannot** feed SWAP spend without attaching openings | **P0 compose C7** |
| D3 | **`asset_id` triple language** | (a) Headstash `NoteDenom`/blake3→Fp on claim instance; (b) `terp_asset_id_from_tacit` SHA-256 map for bridge; (c) SWAP demo `HUB`/`B`/`C` first-byte fixtures. No registry fixture SSOT for claim→swap multi-asset | **P0 freeze** |
| D4 | **claimId hash** | SPEC B: Tacit `keccak(destChain ‖ destCommitment ‖ ν ‖ assetId)`; BRIDGE r1: SHA-256 fixture domain with **extra value_be**. Wire-compat deferred to r2 but harness golden vectors will bifurcate if not labeled | **P1** |
| D5 | **LC mock richness** | BRIDGE `ReflectionSnapshot` (pool/spent/burn, K, **max_lc_lag residual**); HARNESS `LcMockConfig` (tip, conf, frozen only — **no lag residual, no root pins**) | **P1** |
| D6 | **Inventory test counts stale** | HARNESS/E2E plan: bridge **9**, dex **12**, seam **6**. Source `#[test]`: bridge **17**, dex **18**, seam **6**. Agents will under-run CI if they trust the inventory table | **P1 docs** |
| D7 | **cm_encoding product path** | BRIDGE hinge emits `ABSTRACT_LEAF_V0` (0x03); claim path `ORCHARD_CMX` (0x01); Tacit production `TACIT_KECCAK_LEAF` (0x02). Shared-`T` honesty already documented — still easy to overclaim unlinkability in demos | **P1 honesty** |
| D8 | **Contract home / suite home** | BRIDGE Q6 (`cw-bridge-mint` vs Headstash ext); HARNESS Q10 (`PrivateBridgeSuite` in test-press). Unanswered → duplicate scaffolds | **P1 process** |
| D9 | **Reject code freeze** | Matrix placeholders vs BRIDGE enum (`NotInBurnSet`, …) vs harness `E_*` strings — three dialects | **P2** until contracts |
| D10 | **Legacy dual APIs in dex crate** | u64-nullifier `apply_swap` + 32-byte `SwapActionV0` both green; fine if labeled, confusing if L1 re-exports the wrong one | **P2** |

### Plan deviations (justified vs problematic)

| Deviation | Verdict |
|-----------|---------|
| BRIDGE expanded `bridge_auth_seams` instead of new crate | **Beneficial** — avoids opcode fork |
| SWAP stayed in pure fixtures; no `circuit/src/swap/` | **Beneficial** — avoids H1 layout thrash |
| HARNESS only stub suite + plan (no Docker compose) | **Correct** for round 1 |
| DEMO-PATH “SEAM-NOTE-OUT only after claim boring” vs SEAM already frozen | **OK if read as claim-demo priority**, not as “delete SEAM” — claim fixture export is still next for A |
| HARNESS L0–L4 catalog E2E-01..20 | **Realistic** as progression map; **over-scoped** if treated as round-2 deliverable set |

---

## Blind spots (ranked P0/P1/P2)

### P0 — must address before claiming any compose / e2e

1. **No cross-crate L0 path**  
   `authorize_bridge_mint` → full `SeamNoteOutV0` → `is_dex_consumable` → `SwapActionV0` is **not** tested end-to-end. Three green islands ≠ C7 / C1.2 structural compose.

2. **rcm gap on bridge egress**  
   DEX consumability hard-requires openings; bridge sketch omits them. Either:
   - hinge emits `rcm` when `rcm_flag=1` for phase-1 tests, or  
   - document phase-0 bridge notes as **non-DEX** until openings attached, and add an explicit attach helper in compose tests.

3. **asset_id SSOT for multi-asset demo**  
   Without a single registry fixture (claim denom → 32-byte Terp id; tacit id → same width), swap fixtures will not consume real claim/bridge notes.

4. **Demo order conflict (product narrative)**  
   - DEMO-PATH / H1: **claim policy → prove** is the Headstash spine.  
   - BRIDGE: **burn → mint** is conservation spine.  
   - SWAP: **note in T → swap hub edge**.  
   Parallel stories are fine **only if** harness IDs keep them separate (E2E-08 vs E2E-01 vs E2E-10) and standups do not demand one “full stack” green this sprint.

### P1 — should fix in Round 2 planning / thin glue

5. **Who posts LC / reorg / lag**  
   BRIDGE Q3–Q4, Q9 surface the right questions; HARNESS `lc_mock_allows_mint` does not model residual lag or reorg freeze. Policy freeze needed before L4 demos.

6. **Membership still boolean**  
   `in_burn_set` / `in_pool_root` stubs — E2E-15 (wrong root) cannot leave “bool flip” fidelity until pure IMT fixtures land (BRIDGE r2 item).

7. **CW bridge-mint object missing**  
   HARNESS correctly keeps E2E-01 on pure until msg exists; risk is agents inventing ad-hoc ExecuteMsg outside Domain B sketch.

8. **Spend auth adequacy**  
   `owner_binding` vs full Orchard spend-auth (SWAP Q7) — phase-1 structural vs production security story must stay labeled.

9. **Public Δ privacy honesty**  
   SWAP freezes public reserve deltas; good. Ensure HARNESS / demo scripts do not claim “private trade size.”

10. **Stale inventory in E2E plan**  
    Update test counts and map new hinge / SwapAction tests to E2E-xx / C\* so CI recipes are trustworthy.

11. **No `just e2e-l0` aggregator**  
    Planned but missing; PR CI budget (HARNESS Q13) unresolved.

### P2 — nice to have / later

12. Dual hash domains for claimId (document translation table only).  
13. cBTC corridor vs bridge mint surface (BRIDGE Q10).  
14. Exit path priority unshield vs bridge_burn (HARNESS Q14).  
15. Fee protocol skim destination (SWAP Q3 — already default “in reserves”).  
16. Optional 4th agent sprawl (see sequencing).

---

## Merged clarity questions (must-answer for round 2, ranked)

Human/product answers. Deduped from HARNESS (H), SWAP (S), BRIDGE (B). **Top block blocks useful Round 2 code.**

### Must-answer (block parallel implementation)

| # | Question | Source | Why blocking |
|---|----------|--------|--------------|
| **M1** | **Ingress path for first joint green:** (a) Anvil `OP_BRIDGE_BURN` → mock hinge → CW mint, (b) Headstash claim-only L1 then Anvil later, (c) both with separate case IDs? *Rec: (b) PR CI + (a) weekly L3.* | H1 | Sets suite methods + Docker budget |
| **M2** | **CosmWasm mint object home:** new `cw-bridge-mint` vs Headstash/manifold extension vs `x/`? Name owner crate. | H2, B6 | Stops dual scaffolds |
| **M3** | **`asset_id` SSOT:** Is Headstash `NoteDenom` (32B field form) the same object as SEAM `asset_id` for claim→swap, or mandatory registry map before multi-asset merge? Who owns registry fixture (JSON / genesis / instantiate)? | S1, H6, B2 | Unblocks C7 + swap fixtures |
| **M4** | **Single privacy tree T** for claim + bridge + swap in phase-1 demo (eligibility multi-root only for distro)? Confirm. | S2, B7 | Leaf append + root window policy |
| **M5** | **Mock ZK on PR CI for E2E-08** (real H1 prove nightly / `demo-h1` only)? yes/no | H3 | CI honesty |
| **M6** | **Mock LC never Tier-0** even if E2E-01 green on mock? yes/no | H4, B risks | Trust labeling |
| **M7** | **claimId wire:** SHA-256 Terp fixture domain through r2 vs byte-for-byte Tacit keccak? If both: golden vector translation table required. | B5 | Golden fixtures |
| **M8** | **Confirmations + lag:** Pin pilot K and max residual lag (e.g. K=6, MAX=64) for Anvil local vs production; residual-after-K vs absolute tip−H? | H5, B3 | Unify `LcMockConfig` ↔ `ReflectionSnapshot` |
| **M9** | **Who posts `UpdateReflection`?** Permissionless / allowlist / gov? | B4 | Contract sketch + L3 relayer |
| **M10** | **DEX on Terp timeline:** E2E-10..12 stay L0-only for N sprints, or multi-test AMM stub required before L3? | H7, S plan | Scope SWAP r2 |

### Should-answer soon (shape demos, not block pure glue)

| # | Question | Source |
|---|----------|--------|
| M11 | Suite home: keep `PrivateBridgeSuite` in `zk-test-press`? | H10 |
| M12 | Tacit deploy in Docker: host forge vs sidecar foundry vs predeploy volume? | H9 |
| M13 | ict-rs image: `local-zk` only vs non-zk smoke? | H8 |
| M14 | Hashmerchant never cited as conservation mint proof? (expect **yes**) | H12 |
| M15 | Public Δ_in/out for v1 demo? (SWAP rec: **yes**) | S5 |
| M16 | Fees remain in pool reserves only; protocol fee 0? | S3 |
| M17 | `require_oracle = false` default for demo? | S4 |
| M18 | Hub asset: shielded TERP vs cBTC-class label for star topology? | S6 |
| M19 | owner_binding sufficient for phase-1 spend checks vs full Orchard spend-auth? | S7 |
| M20 | Change notes allowed vs exact-input only for v1 circuit size? | S8 |
| M21 | Pilot `source_chain_tag` + `dest_domain` encoding (mainnet vs local) | B1 |
| M22 | Mock proof bytes minimum for CW pilot | B8 |
| M23 | Historical height mints within lag window? | B9 |
| M24 | cBTC lock: same contract surface vs separate msg? | B10 |
| M25 | ZEC egress non-blocking for r2? (expect **yes**) | B11 |
| M26 | Reject codes: freeze matrix `E_*` for harness vs wait for contract enums? | H11 |
| M27 | CI wall-clock: L0 only on PR vs L0+L1 mock? | H13 |
| M28 | Exit first: unshield vs bridge_burn for E2E-17? | H14 |

---

## Recommended round-2 plan (per agent + shared)

### Shared first (0.5–1 day, human + integrator)

1. Answer **M1–M10** (record decisions in this file or a short `ROUND2-DECISIONS.md`).  
2. Refresh **E2E-HARNESS-PLAN** inventory counts (17 / 18 / 6 as of 2026-07-20 source scan) and map new hinge / SwapAction tests → E2E-xx.  
3. Add **`just e2e-l0`** (or workspace recipe) running three fixture crates.

### Shared L0 glue (highest leverage — optional **4th agent: COMPOSE**, or HARNESS+BRIDGE+SWAP jointly)

| Task | Owner bias |
|------|------------|
| `NoteOutSketch` → `SeamNoteOutV0` (+ optional rcm attach) | BRIDGE + SEAM |
| `from_headstash_instance` / `from_bridge_mint` → `spend_opening` → `validate_swap_action` pure test | SWAP + SEAM |
| Single **registry fixture JSON** (one hub asset + one bridge-mapped asset) | HARNESS owns file; B/S consume |
| Align `LcMockConfig` with lag residual + tip fields (or wrap `ReflectionSnapshot`) | HARNESS + BRIDGE |

### Per agent (ordered)

#### HARNESS

1. `just e2e-l0` + E2E-id print mapping.  
2. Implement **L0 re-export** tests in/near `private_bridge` (no Docker): E2E-01..07, 10..13 via pure crates.  
3. DEMO-PATH claim fixture **loader** → multi-test/mock process_headstash (E2E-08) — mock ZK per M5.  
4. Document L2 wrap `just tacit-anvil-rt` (E2E-18).  
5. L3 **smoke only** E2E-19 (Anvil+Terp up; clone hashmerchant, strip VE) — **after** M1/M8/M12.  
6. **Do not** implement full burn→mint in Docker until CW mint object exists (M2).

#### BRIDGE

1. Wire **full** `SeamNoteOutV0` emission path (rcm policy per M / P0#2).  
2. claimId / asset golden vectors with **explicit hash algo label** (M7).  
3. Pure **IMT membership fixture** replace bools for burn/pool (E2E-15 seed).  
4. CosmWasm **scaffold** from sketch **only after M2** — storage: tip, minted set, registry.  
5. One Tacit anvil **public-values → hinge accept** golden (not full SP1).  
6. Lag/reorg policy freeze numbers (M8–M9).  
7. **Not r2:** SP1 guest rehost, mainnet headers, ZEC TZE.

#### SWAP

1. Freeze answers M3/M4/M10/M15–M20.  
2. **Compose pure test:** SEAM note → SwapAction (C7 / C1.2 structural) — priority over circuit.  
3. Optional: `crates/headstash/circuit/src/swap/` **types-only** skeleton **after** claim layout stable; **no** full Circuit thrash while H1 red.  
4. Keep oracle host-only; no mint path.  
5. Host apply stub design (mirror `apply_swap_action`) only if M10 demands multi-test.

#### Optional 4th agent

| Option | When | Role |
|--------|------|------|
| **COMPOSE (recommended)** | If three agents stay domain-siloed | Owns cross-crate L0 glue + registry fixture + E2E-id mapping only — **no** new SPECs |
| **CLAIM / H1** | If product priority is demo-policy → prove | Isolates MockProver last-mile; must **not** edit bridge/swap seams |
| **Avoid** | — | Fourth agent inventing a fourth note language or parallel suite framework |

### Demo path order (recommended narrative)

```text
Track A (Headstash): holders → Poseidon distro → demo-policy → [mock] process_headstash → SeamNoteOut (claim)
Track B (Bridge):    burn fixture → ReflectionSnapshot → authorize_bridge_mint → SeamNoteOut (bridge)
Track C (Swap):      SeamNoteOut (A or B) + rcm → SwapActionV0 → public ΔR
Track D (L3 later):  Anvil burn PV → mock hinge → CW mint  (only after M1=a/c and M2)
```

Do **not** serialize Track A behind Track B; do **not** require Track D for Track C L0.

---

## Tests to add before claiming e2e

### Before any “compose green” language

| Test | Layer | Property |
|------|-------|----------|
| `compose_bridge_mint_to_seam_note_out` | L0 | Sketch/full note field parity: origin, nf domain, provenance=burn root, asset map |
| `compose_claim_or_bridge_to_swap_action` | L0 | `is_dex_consumable` → pool ν ≠ lineage → reserves + min_out |
| `compose_oracle_cannot_mint_via_swap_or_bridge` | L0 | E2E-13 / C6 single assertion surface |
| `compose_h1_spent_not_mint_and_not_swap_auth` | L0 | Spent-only never mints; ingress ν never pool-spend |

### Before L1 “suite green”

| Test | Notes |
|------|-------|
| Claim fixture load + mock verify accept/reject | E2E-08 |
| Double mint / double claim nullifier | E2E-02 + C2.1 |
| LC mock freeze / immature conf | E2E-04/14 using **shared** lag model |

### Before L3 “joint e2e”

| Test | Notes |
|------|-------|
| E2E-19 dual container smoke | Ports only |
| E2E-18 Tacit anvil RT still green in isolation | Regression |
| E2E-01 only when CW mint + packet fields real | Else keep L0 |

### Explicitly **not** sufficient for e2e claims

- Green pure crate counts alone  
- Hashmerchant Anvil+Terp VE path  
- Hinge happy path with `in_burn_set: true` bool without membership fixture (ok for L0 unit, not Tier-0)  
- SwapAction structural tests without SEAM emitter linkage  

### Fixture test inventory (source scan 2026-07-20)

| Crate | `#[test]` count | Prior inventory claim |
|-------|----------------:|----------------------|
| `bridge_auth_seams` | **17** | 9 |
| `private_dex_seams` | **18** | 12 |
| `seam_note_out` | **6** | 6 |

**Runtime:** This review did **not** re-execute `cargo test` (read-only meta; no code changes). Round-2 opener should run:

```bash
cd docs/plans/spectrum/fixtures/bridge_auth_seams && cargo test
cd docs/plans/spectrum/fixtures/private_dex_seams && cargo test
cd docs/plans/spectrum/fixtures/seam_note_out && cargo test
```

---

## What not to do yet

1. **Do not** claim joint anvil + terpd + LC private-bridge e2e.  
2. **Do not** implement full Halo2 swap Circuit / MockProver while H1 claim composite is red (types-only skeleton OK).  
3. **Do not** treat hashmerchant / oracle / VE as mint or reserve authority in any demo script.  
4. **Do not** label mock LC or mock ZK as Tier-0.  
5. **Do not** invent opcodes or a fourth note language outside SEAM-NOTE-OUT / Domain B map.  
6. **Do not** start dual CosmWasm packages for bridge mint before **M2**.  
7. **Do not** pull ZEC Crosslink / TZE / ZIP-222 into Round-2 critical path (B11 / CLARITY).  
8. **Do not** expand L3 Docker mint path before L0 compose glue + CW object decision.  
9. **Do not** merge Orchard cmx and Tacit keccak leaves into one tree without encoding policy (honest tags only).  
10. **Do not** use ingress claim/burn nullifier as pool-spend ν (already tested in SWAP — keep red-line).  
11. **Do not** block Headstash `demo-policy` on private DEX or bridge mint.  
12. **Do not** thrash `TestPressSuite` incomplete `Deploy` — extend `PrivateBridgeSuite` instead.

---

## What Round 1 did well

- **Honest inventory** of missing joint e2e and hashmerchant boundary.  
- **H-1 and oracle non-authority** repeatedly tested and documented.  
- **SEAM-NOTE-OUT** as structural SSOT already frozen with maps.  
- **Deferred crypto** (SP1, Halo2 swap, real LC) while expanding pure gates.  
- **Suite composition plan** (HeadstashSuite wrap) over new frameworks.  
- Clarity questions are generally the *right* questions — this meta review only **ranks and dedupes**.

---

## Document history

| Date | Change |
|------|--------|
| 2026-07-20 | Round-1 meta review: CONDITIONAL GO; consistency; P0–P2 blinds; merged Qs; r2 sequencing |
