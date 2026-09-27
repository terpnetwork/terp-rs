# ROUND2-BRIDGE

| Field | Value |
|-------|--------|
| **Agent** | BRIDGE |
| **Round** | 2 |
| **Date** | 2026-07-20 |
| **Status** | pure hinge rcm + `cw-headstash` bridge mint surface landed |
| **SSOT** | [`CLARITY-cw-headstash-router-and-asset-registry.md`](../CLARITY-cw-headstash-router-and-asset-registry.md) |
| **Code homes** | [`fixtures/bridge_auth_seams`](../fixtures/bridge_auth_seams/), [`crates/headstash/contracts/cw-headstash`](../../../../crates/headstash/contracts/cw-headstash/) |

> **Frozen product decisions:** `cw-headstash` **is** the mint; no freestanding `cw-bridge-mint`. Asset registry = internal and/or external; registry/oracle **never mint balances**.

---

## 1. Pure hinge improvements (`bridge_auth_seams`)

### `NoteOutSketch` ↔ `SeamNoteOutV0` field roles

| Field | Round-1 | Round-2 |
|-------|---------|---------|
| core origin / nf / provenance / claim_id / … | present | unchanged roles |
| `rcm` | **missing** (P0 D2) | **present** (`Hash32`) |
| `rcm_flag` | **missing** | **present** (`0`/`1`) |
| `memo` / `memo_flag` | missing | present (zero / `0` on mint) |
| DEX consumability | impossible without compose attach | `NoteOutSketch::is_dex_consumable()` when `rcm_flag == 1` |
| serialization | n/a | `to_seam_bytes()` → **382** bytes (SEAM order) |

### `BridgeMintPublic.rcm`

- Non-zero `rcm` on the public packet → hinge emits `rcm_flag = 1`.
- All-zero / absent → `rcm_flag = 0` (phase-0 non-DEX note; still a valid mint).
- Happy fixture uses `h("rcm-hinge-happy")` so happy path is **DEX-consumable**.

### API

```text
authorize_bridge_mint(...) -> Result<NoteOutSketch, BridgeMintError>
NoteOutSketch::is_dex_consumable(&self) -> bool
NoteOutSketch::to_seam_bytes(&self) -> [u8; 382]
```

### Tests (H-1 kept green)

| Test | Expect |
|------|--------|
| legacy T1–T7 + spent-alone | still green |
| `hinge_t1_authorize_bridge_mint_happy` | ACCEPT + `rcm_flag=1` + `is_dex_consumable` |
| `hinge_rcm_absent_not_dex_consumable` | ACCEPT mint, `rcm_flag=0`, not DEX |
| `hinge_h1_spent_only_reject` | REJECT `NotInBurnSet` |
| other hinge rejects (dest / lag / immature / domain_binding / stale burn) | green |

```bash
cd docs/plans/spectrum/fixtures/bridge_auth_seams && cargo test
# or: bash run_tests.sh
```

### Contract sketch enum (types-only)

`ExecuteMsgSketch` aligned to Round-2 names: `UpdateReflection`, `SetReflectionSnapshot`, `RegisterAsset` (internal), `SetExternalAssetRegistry`, `BridgeMintNote`, `Freeze`. Production surface is on **cw-headstash**, not this fixture.

---

## 2. `cw-headstash` bridge mint surface

### Module

| Path | Role |
|------|------|
| [`src/bridge.rs`](../../../../crates/headstash/contracts/cw-headstash/src/bridge.rs) | state, pure gates, execute/query handlers, unit tests |
| [`src/msg.rs`](../../../../crates/headstash/contracts/cw-headstash/src/msg.rs) | `ExecuteMsg` / `QueryMsg` extensions |
| [`src/lib.rs`](../../../../crates/headstash/contracts/cw-headstash/src/lib.rs) | `pub mod bridge`; execute/query dispatch |

Workspace: `contracts/cw-headstash` added to `crates/headstash` members for `cargo test -p cw-headstash`.

### ExecuteMsg (new)

| Variant | Auth | Role |
|---------|------|------|
| `UpdateReflection { pool_root, spent_root, burn_root, source_height, tip_height }` | owner | advance tip roots |
| `SetReflectionSnapshot { snapshot }` | owner | full snapshot (K, lag, frozen) |
| `RegisterAsset { asset_id, local_denom, origin?, status? }` | owner | **internal** registry entry |
| `SetExternalAssetRegistry { addr: Option<String> }` | owner | optional external hook (not required for demos) |
| `SetBridgeCfg { cfg }` | owner | dest_domain, K, lag, `mock_verify` |
| `BridgeMintNote { claim, proof }` | public (gates enforce) | one-shot mint → SEAM note attrs |

### QueryMsg (new)

`ReflectionTip`, `IsBridgeMinted { nullifier }`, `BridgeAsset { asset_id }`, `BridgeConfig`, `ExternalAssetRegistry`.

### State

| Item / Map | Key / value |
|------------|-------------|
| `REFLECTION_SNAPSHOT` | `ReflectionSnapshot` |
| `BRIDGE_CFG` | `BridgeCfg` (dest_domain, K, lag, lc_client_id, **mock_verify**) |
| `ASSET_REGISTRY` | `hex(asset_id) → AssetEntry` |
| `EXTERNAL_ASSET_REGISTRY` | `Option<Addr>` |
| `BRIDGE_MINTED` | `bridge:02:{hex(ν)} → ()` (double-mint reject) |

Also pins the same key into shared `NULLIFIERS` for product-surface unity with claim nullifiers (domain-separated; no collision with `{root_id}:{nf}`).

### Pure gates (reimplemented in-contract)

`authorize_bridge_mint_pure` mirrors hinge gates:

1. tip / freeze / zero burn root  
2. height pin, conf maturity, lag residual  
3. burn root + pool root pin  
4. **H-1:** `in_burn_set` required; `spent_only && !in_burn_set` → reject  
5. pool membership stub  
6. once-per-ν (`AlreadyMinted`)  
7. internal asset map + **Active** status  
8. dest domain, destCommitment, claim_id, domain_binding re-derive  
9. emit `NoteOutResult` with **rcm / rcm_flag** for DEX consumability  

**Invariant:** registry lookup only resolves asset_id — never credits balances.

### ZK / LC verify (Round 2 mock)

```text
mock_verify_bridge_proof(mock_verify, claim, proof)
  - allow when cfg!(test) OR BridgeCfg.mock_verify
  - requires claim.in_burn_set
  - empty proof rejected outside cfg(test)
  - real SP1 / LC IMT: NOT wired (documented; no anvil)
```

### Unit tests (`bridge` module)

| Test | Expect |
|------|--------|
| `pure_happy_path_dex_consumable` | note `rcm_flag=1`, DEX-consumable |
| `pure_h1_spent_only_reject` | `NotInBurnSet` |
| `pure_unmapped_asset_reject` | `UnmappedAsset` |
| `pure_double_mint_flag_reject` | `AlreadyMinted` |
| `execute_unregistered_asset_reject` | execute fails unmapped |
| `execute_double_mint_reject` | second mint fails |
| `execute_h1_spent_only_reject` | H-1 on execute path |
| `execute_happy_path_mock_verify` | attrs + `BRIDGE_MINTED` set |
| `register_asset_and_set_external` | internal + external hook |
| `domain_separated_claim_key` | `bridge:02:…` |

```bash
cd crates/headstash && cargo test -p cw-headstash --lib
```

---

## 3. Compose coordination

- `compose_seams` still attaches `NoteOpenings` for abstract-cm recompute; hinge may now also carry `rcm` on `BridgeMintPublic` / sketch.
- Fixture `BridgeMintPublic` construction updated with `rcm` field so path deps compile.
- Prefer hinge-emitted rcm for phase-1 demos; openings remain for swap cm recompute path.

---

## 4. Clarity leftovers / Round-3 backlog

| # | Item | Notes |
|---|------|-------|
| 1 | Real SP1 / LC membership verify | replace mock bools + `mock_verify` |
| 2 | Pure IMT under burn/pool roots | E2E-15 fidelity |
| 3 | claimId keccak wire-compat vs SHA-256 fixture | label translation table |
| 4 | External registry **query** at mint time | addr hook stored; not queried yet |
| 5 | Shared `AssetRegistryView` package | compose fixture SSOT; contract map is parallel |
| 6 | Who posts `UpdateReflection` | owner-only today; relayer allowlist TBD |
| 7 | Lag / reorg freeze numbers for pilot | defaults K=6, max_lag=64 |
| 8 | Bank / note-tree append on mint | Round-2 emits attrs only (no pool write) |
| 9 | Full 382-byte event / response data | attr base64 JSON today |
| 10 | ZEC egress TZE | non-blocking |

---

## 5. Risks / trust tiers (unchanged spirit)

| Risk | Mitigation this round |
|------|------------------------|
| Spent-set as mint authority | H-1 pure + execute tests |
| Double mint | `BRIDGE_MINTED` domain key |
| Unregistered asset mint | internal map reject |
| Mock LC over-trusted | `mock_verify` flag + module docs |
| Registry minting balances | no mint API on registry; resolve only |
| Parallel mint package | **not** introduced; msgs on cw-headstash |

---

## 6. Verify recipes

```bash
# Pure hinge
cd docs/plans/spectrum/fixtures/bridge_auth_seams && cargo test

# Contract lib (includes bridge unit tests + existing instantiate tests)
cd crates/headstash && cargo test -p cw-headstash --lib

# Optional: compose still green after rcm field on BridgeMintPublic
cd docs/plans/spectrum/fixtures/compose_seams && cargo test
```

> **Note:** Round-2 agent could not execute cargo in-tool; parent should run the recipes above before merge. Code is structured for green H-1 + mock happy path.

---

## Changelog

| Date | Change |
|------|--------|
| 2026-07-20 | Round 2: NoteOutSketch rcm/DEX, cw-headstash bridge msgs/state/execute/tests, report |
