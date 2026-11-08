# ROUND1-BRIDGE

> **Supersession (2026-07-20):** Contract home is **`cw-headstash`** (unified mint/router), not a freestanding `cw-bridge-mint`. Asset registry = **internal and/or external** + cross-chain wiring. See `CLARITY-cw-headstash-router-and-asset-registry.md`.

| Field | Value |
|-------|--------|
| **Agent** | BRIDGE |
| **Round** | 1 |
| **Date** | 2026-07-20 |
| **Status** | types + pure gates + tests + contract sketch landed |
| **SSOT** | Domain B [`SPEC-tacit-bridge-mapping.md`](../SPEC-tacit-bridge-mapping.md), Domain C [`SPEC-lc-hinge-private-bridge.md`](../SPEC-lc-hinge-private-bridge.md), [`SEAM-NOTE-OUT.md`](../SEAM-NOTE-OUT.md) |
| **Code home** | [`docs/plans/spectrum/fixtures/bridge_auth_seams`](../fixtures/bridge_auth_seams/) (expanded; not a parallel opcode crate) |

> Round 1 = **deterministic mapping hinge**, not full SP1 guest or mainnet LC.  
> Burn set is mint authority (**H-1**). Oracle never mints. ZEC egress LC+TZE is **non-blocking** (document only).

---

## Mint-critical public statement

### Class A reflection tip (snapshot)

Aligns with Tacit `BitcoinRelayPublicValues` mint-critical subset + Terp tip policy:

```text
ReflectionSnapshot {
  pool_root,        // bitcoinPoolRoot — burned note membership (A7); NOT mint authority alone
  spent_root,       // bitcoinSpentRoot — cross-lane / spend gates only; NEVER mint alone (H-1)
  burn_root,        // bitcoinBurnRoot — **mint authority** (A5 / H-1); never zero
  source_height,    // confirmed height H
  tip_height,       // LC tip (≥ H + K for maturity)
  confirmations_k,  // REFLECTION_CONFIRMATIONS spirit (default 6)
  max_lc_lag,       // lag residual window after K (default 64)
  frozen,           // misbehaviour / gov freeze
}
```

### Mint packet (`BridgeMintPublic`)

Domain B §9 + Domain C §3.1 class A fields:

```text
BridgeMintPublic {
  source_chain_tag,     // e.g. "bitcoin-mainnet" | "eip155:1"
  tacit_asset_id,       // 32-byte foreign id
  value_u64,            // opened; == v_burn (A9)
  nullifier,            // ν of burned note
  dest_commitment,      // burn-bound dest (A6)
  dest_domain,          // Terp dest chain/domain hash
  claim_id,             // re-derived (A14)
  source_pool_root,     // pin vs snapshot.pool_root
  source_burn_root,     // pin vs snapshot.burn_root (A17)
  source_height,        // pin vs snapshot.source_height (r1 exact)
  domain_binding,       // C §3.1 re-derive
  unit_scale,
  pool_domain,          // Terp note-set instance (A16)
  cm_public,            // dest leaf fingerprint for NoteOutSketch
}
```

### ClaimId / domain_binding re-derive (fixture-stable SHA-256)

```text
claim_id = H("terp-bridge-claim-v1" ‖ dest_domain ‖ dest_commitment ‖ ν ‖ tacit_asset_id ‖ value_be)

domain_binding = H(
  "terp-private-bridge-v1" ‖
  src_chain_id ‖ dst_chain_id ‖ lc_client_id ‖
  tacit_asset_id ‖ ν ‖ height_be ‖ burn_root
)
```

> Production may use keccak where Tacit on-chain does; Round 1 freezes **SHA-256 fixture-stable** re-derives so pure tests stay offline. Wire-compat translation is Round 2.

### Accept / reject gates (aligned with bridge_auth_seams T1–T7 + hinge)

| Gate | Property | Reject |
|------|----------|--------|
| tip / freeze | A1/A3 | `TipNotOk` |
| zero burn root | A17 / H-1 | `ZeroBurnRoot` |
| conf depth K | A2 / T4 | `ImmatureConfirmation` |
| lag residual | LC-STALE | `LagExceeded` |
| burn root pin | A17 / T5 | `StaleBurnRoot` |
| pool root pin | A7 currency | `StalePoolRoot` |
| **burn-set membership** | **A5 / H-1 / T3** | **`NotInBurnSet`** |
| pool membership stub | A7 | `NotInPoolRoot` |
| once-per-ν | A13 / T2 | `AlreadyMinted` |
| asset map | A15 / T7 | `UnmappedAsset` |
| dest domain | A16 / T7 | `DomainMismatch` |
| destCommitment | A6 / T9 | `DestCommitmentMismatch` |
| conservation | A9 / T5 | `ValueMismatch` |
| claim_id | A14 / T8 | `ClaimIdMismatch` |
| domain_binding | C §3.1 | `DomainBindingMismatch` |

**Explicit non-authority:** ordinary spent-set membership alone → **REJECT** (H-1). Oracle / VE mid → **never** in mint path (A12).

### Mock LC vs real reflection path

| Mode | What Round 1 does | What production needs |
|------|-------------------|------------------------|
| **Mock LC** | `ReflectionSnapshot` fields set by fixture; `in_burn_set` / `in_pool_root` booleans stand in for IMT membership | Real IMT paths under attested roots |
| **Real reflection** | Documented only | SP1 `BitcoinRelayPublicValues` / `attestBitcoinStateProven` or Terp CosmWasm LC client storing same roots; `BITCOIN_RELAY_VKEY`; monotonic height |
| **Round 1** | Pure verify gates + `NoteOutSketch` | No guest, no headers, no mainnet K |

---

## Pure API + tests landed

### Home

Expanded existing pure crate (preferred over a second crate):

| Path | Role |
|------|------|
| [`../fixtures/bridge_auth_seams/src/lib.rs`](../fixtures/bridge_auth_seams/src/lib.rs) | Legacy T1–T7 gates **plus** hinge types + `authorize_bridge_mint` |
| [`../fixtures/bridge_auth_seams/Cargo.toml`](../fixtures/bridge_auth_seams/Cargo.toml) | `sha2` only; no SP1/halo2 |

### API surface

```text
authorize_bridge_mint(snapshot, claim, minted, registry) -> Result<NoteOutSketch, BridgeMintError>
authorize_bridge_mint_apply(...)  // mark ν after accept

ReflectionSnapshot, BridgeMintPublic, BridgeMintClaim, NoteOutSketch
derive_claim_id_with_dest, derive_domain_binding, terp_asset_id_from_tacit
InstantiateMsgSketch / ExecuteMsgSketch / QueryMsgSketch / LcIngressCertificateSketch
```

### `NoteOutSketch` → SEAM-NOTE-OUT

| Field | Value on accept |
|-------|-----------------|
| `origin` | `0x02` `ORIGIN_BRIDGE_MINT` |
| `nullifier_domain` | `0x02` `NF_BRIDGE_BURN` |
| `owner_binding` | `dest_commitment` |
| `provenance_anchor` | `source_burn_root` (H-1 currency) |
| `asset_id` | mapped Terp id (`terp-tacit-asset-v1` …) |
| `cm_encoding` | `0x03` abstract harness leaf (no keccak in pure crate) |

Full 382-byte `SeamNoteOutV0` remains in [`seam_note_out`](../fixtures/seam_note_out/); Round 1 emits a structural sketch with the same field roles.

### Tests

**Legacy (kept green):** 9 tests — T1 happy, T2 double mint, T3 ordinary spend ≠ burn, T4 immature, T5 value mismatch, T6 unmapped asset, T7 domain mismatch, domain_bind separation, spent_set alone never authorizes.

**Hinge (new):**

| Test | Expect |
|------|--------|
| `hinge_t1_authorize_bridge_mint_happy` | ACCEPT → `NoteOutSketch` |
| `hinge_dest_commitment_bind_reject` | REJECT A6 |
| `hinge_lag_policy_stub_reject` | REJECT lag |
| `hinge_immature_confirmation_reject` | REJECT A2 |
| `hinge_domain_binding_reject` | REJECT domain_binding |
| `hinge_h1_spent_only_reject` | REJECT H-1 |
| `hinge_stale_burn_root_reject` | REJECT A17 |
| `contract_surface_sketch_constructs` | types compile / construct |

Verify:

```bash
cd docs/plans/spectrum/fixtures/bridge_auth_seams && cargo test
```

---

## Contract surface sketch

Types-only CosmWasm sketch in the same crate (not a deployable contract):

```text
InstantiateMsgSketch {
  dest_domain, confirmations_k, max_lc_lag, lc_client_id, admin?
}

ExecuteMsgSketch {
  UpdateReflection { pool_root, spent_root, burn_root, source_height, tip_height }
  RegisterAsset { source_chain_tag, tacit_asset_id, unit_scale, conservation_class }
  BridgeMintNote { public: BridgeMintPublic, proof: bytes }   // opaque r1
  Freeze {}
}

QueryMsgSketch { ReflectionTip, IsMinted, AssetMapped, Config }

LcIngressCertificateSketch { class, client_id, height, pool/spent/burn roots, domain_binding, proof_kind, value }
```

**Future names:** `cw-bridge-mint` **or** Headstash extension — team choose in clarity Qs.  
**Not in Round 1:** storage layout, wasm entry points, real proof verify, Headstash claim path (separate; bridge mint shares SEAM-NOTE-OUT only).

---

## Map to Tacit surfaces (S1–S12)

| # | Surface | Round-1 hinge posture |
|---|---------|------------------------|
| S1 | Confidential note model | `NoteOutSketch` fields; value + ν + destCommitment language |
| S2 | ConfidentialPool settle | Documented; not rehosted |
| S3 | `OP_BRIDGE_BURN` | Claim public values from burn; mock `in_burn_set` |
| S4 | `OP_BRIDGE_MINT` | `authorize_bridge_mint` = dest mint gate |
| S5 | Bitcoin reflection prover | `ReflectionSnapshot` public values only |
| S6 | Bridge-burn set ≠ spent | **H-1 enforced** in hinge + legacy tests |
| S7 | Finality / K | `confirmations_k` + lag stub |
| S8 | Asset registry / crossChainLink | `RegisterAsset` sketch + `AssetRegistry` |
| S9 | WRAP/UNWRAP | Out of mint path (same-chain) |
| S10 | TRANSFER / kernel | Conservation template only |
| S11 | AMM | Post-mint; Domain D |
| S12 | cBTC mint | Conservation class sibling; not exercised in r1 happy path |

S13–S16 (CDP, B4 dual-home, legacy tETH, envelopes): **non-primary**; not implemented in r1.

---

## Clarity questions for team (numbered)

1. **Chain ids:** Pin pilot `source_chain_tag` set (`bitcoin-mainnet` only vs + `eip155:1` / sepolia)? What is Terp dest chain id / `dest_domain` encoding for mainnet vs local?
2. **Asset registry:** Who may `RegisterAsset` — governance, admin, or immutable constructor list? First-write-wins without admin?
3. **Lag policy:** Is residual lag after K the right model, or absolute `tip - source_height ≤ MAX`? Pin numbers for pilot (K=6, MAX=64)?
4. **Who posts LC updates?** Permissionless relayer, allowlisted operators, or gov-gated `UpdateReflection`?
5. **ClaimId wire hash:** Keep SHA-256 fixture domain for Terp, or must match Tacit on-chain `keccak(destChain ‖ destCommitment ‖ ν ‖ assetId)` byte-for-byte in r2?
6. **Contract home:** New `cw-bridge-mint` package vs Headstash / manifold extension vs `x/` module?
7. **Note tree:** Bridge mint appends into same privacy set `T` as Headstash claims (SEAM-NOTE-OUT honesty) — confirm single tree vs dual + link proof for pilot.
8. **Mock proof bytes:** What minimum proof object does CosmWasm accept in pilot (empty under mock LC flag vs mandatory mock membership blob)?
9. **Historical height mints:** R1 pins exact `source_height == snapshot.source_height`; allow mint against older heights within lag window?
10. **cBTC lock corridor:** Same contract surface with `conservation_class = cbtc_lock`, or separate execute msg?
11. **ZEC egress:** Confirm non-blocking; no Round-2 dependency on ZIP-222 TZE for BTC reflection mint.

---

## Risks / trust tiers

| Risk | Tier / note | Mitigation in r1 |
|------|-------------|------------------|
| Spent-set mistaken for mint authority | Tier-0 **fail** if broken | H-1 tests + hinge `spent_only` reject |
| Stale burn root mint | A17 | Pin to snapshot burn root |
| Mock LC over-trusted in demos | Product honesty | Label mock; no Tier-0 claim without real reflection |
| SHA-256 vs keccak claimId drift | Wire incompat | Document; r2 translate |
| Oracle / hashmerchant mint attempt | Forbidden | Not in API; A12 reject path reserved |
| ZEC Crosslink confused with BTC burn | LC-DOM-04 | Class field on certificate sketch |
| Double mint | A13 | `MintedSet` / `IsMinted` |
| Registry squat | A15 | register-once sketch; unknown → reject |
| Bitcoin AMM worker reserves | Tier 1 | Never mint authority |
| cBTC peg backing | Tier 2 | Document only |

---

## Round-2 plan (ordered)

1. **Wire claimId / leaf encoding:** decide keccak vs SHA-256; golden vectors from Tacit `claimId` / owner-free leaf.
2. **Mock membership:** replace `in_burn_set` bool with pure IMT membership fixture under `burn_root` / `pool_root`.
3. **CosmWasm scaffold:** `cw-bridge-mint` (or chosen home) with `Instantiate` / `Execute` from sketch; storage for tip + minted set + registry.
4. **Compose with SEAM-NOTE-OUT:** emit full `SeamNoteOutV0` via `seam_note_out::from_bridge_mint` (or shared type).
5. **LC certificate adapter:** map `LcIngressCertificate` → `ReflectionSnapshot` + claim flags; reject class mismatch (Crosslink ≠ reflection).
6. **Tacit anvil golden:** one vector from `tests/evm-confidential-anvil-roundtrip.sh` / ConfidentialPool public values → hinge accept.
7. **Lag / reorg policy freeze:** numbers + freeze-on-deep-reorg behaviour.
8. **Not Round 2 blockers:** full SP1 guest rehost, mainnet LC headers, ZEC TZE egress, private DEX settle (Domain D parallel).

---

## Changelog

| Date | Change |
|------|--------|
| 2026-07-20 | Round 1: mint public statement, pure hinge API, tests, contract sketch, clarity Qs |
