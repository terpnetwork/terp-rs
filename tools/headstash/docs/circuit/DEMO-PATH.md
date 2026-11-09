# Headstash stable demo path

Goal: one **repeatable local demo** for Poseidon-v1 inclusion + claim policy (+ prove when green).

Related: `POSEIDON-DISTRO-SURFACE.md`, `ADR-POSEIDON-DISTRO-TREE`, `SPEC-airdrop-orchard-delta` §13.

---

## Demo versions

| Version | Command / tests | What it proves |
|---------|-----------------|----------------|
| **V0 – policy + fixtures** | `just demo-policy` / distro + suite unit tests | Tree root, domain, anchor match, nullifier scopes |
| **V0b – L0 e2e pure** | `just demo-e2e-l0` | Bridge/DEX/SEAM pure seams + claim fixture schema (no prove) |
| **V0c – product pure e2e** | see below | Register → bridge mint note → SwapActionV0 apply → reserves + ν |
| **V1 – circuit construct** | `h1_*` / suite-backed MockProver (**green** Product A) | Full claim witnesses satisfy constraints |
| **V1b – store-circuit keys** | `just demo-keys` / `cc_headstash` offline | VK blob for zkvm upload (`params‖cs‖vk‖footer`) |
| **V2 – prove + contract** | `cc_headstash --upload` + multi-test/daemon | Real proof bytes + `ProcessHeadstash` |

### Product pure e2e (V0c — no Halo2)

Cross-crate L0 product narrative lives in `compose_seams` (single re-export path for C1–C4 + apply):

```bash
# Preferred — one named scenario
cd docs/plans/spectrum/fixtures/compose_seams && cargo test product_path_burn_to_swap_sketch

# Full compose suite (C1–C4 + product path)
cd docs/plans/spectrum/fixtures/compose_seams && cargo test

# Aggregator (all L0 fixtures + harness re-export)
cd crates/headstash && just demo-e2e-l0

# Thin harness re-export of the same product path
cd crates/headstash && cargo test -p zk-test-press --lib harness::compose_l0
```

### L1 mock ZK vs real prove

| Path | Use | Label |
|------|-----|-------|
| Policy + **mock** proof bytes | PR CI / multi-test wiring (`ClaimFixture.mock_proof_hex`) | **mock ZK** — not Tier-0 |
| Real H1 MockProver / prove | `just demo-h1` / nightly only while composite is red | circuit track |

Do **not** block V0 / V0b demos on V1 green.

---

## Spine (do not fork)

```text
holders JSON
  → Poseidon-v1 leaf/CRH tree (suite)
  → depth-32 path root = genesis_root / claim anchor
  → PartialClaimNote + note (matching esk leaf)
  → Circuit + Instance (168 bytes)
  → [MockProver | Proof::create]
  → cw-headstash: register root (poseidon-v1) → process_headstash
```

**Publish the depth-32 path root**, not the shallow suite-only root  
(`MerkleAuthPath::to_circuit_path_and_root_poseidon_v1`).

---

## Claim fixture schema (A2)

Stable JSON for suite, contract tests, and UIs:

```json
{
  "distro_hash_domain": "poseidon-v1",
  "root": "<0x… 32-byte LE pallas base>",
  "root_id": 0,
  "leaf_index": 3,
  "depth": 32,
  "path": ["0x…", "… 32 siblings …"],
  "partial_note": {
    "esk_hex": "…",
    "token": "uterp",
    "value": 1000000,
    "fdi": 3,
    "recipient": "0x…32 bytes…"
  },
  "instance": {
    "anchor": "0x…",
    "nd": "0x…",
    "v": 1000000,
    "recp": "0x…",
    "nf": "0x…",
    "cmx": "0x…"
  },
  "instance_bytes_len": 168,
  "circuit": { "k": 18, "public_inputs": 6 }
}
```

Producer: `suite_backed_claim_pair` + export helper (next).  
Consumer: contract tests, demo scripts, optional prover.

---

## Tooling map

| Tool | Role |
|------|------|
| `zk-headstash` suite / `distro_poseidon` | Tree + claim pair |
| `orchard_delta_part_t` | H1–H6 regression |
| `build_headstash_keys_to` / `just demo-keys` / `cc_headstash` | Offline store-circuit blob for zkvm (no mnemonic) |
| `cc_headstash --upload` | Optional daemon upload (`MNEMONIC` required; never commit) |
| `cw-headstash` + `distro` | Roots + `ProcessHeadstash` |
| `manifold` | Multi-drop factory story |
| `gen_headstash_notes` | Human fixture generator |
| `just demo-policy` | Fast green demo without prove |

---

## Ordered work after H1 green

1. Export claim fixture from suite (schema above).  
2. `just demo-policy` (already sketched).  
3. Check in or CI-build `vk_combined` / footer.  
4. Multi-test claim with mock ZK verify, then real prove nightly.  
5. Manifold second root demo.  
6. SEAM-NOTE-OUT only after claim is boring.

## Do not block demo on

- Full PQ note-commit migration  
- RedPallas batch validator  
- MerkleChip config strip (layout / rekey risk)  
- Private DEX / IBC north-star features  

---

## Known residual (H1 still red as of 2026-07-20)

Composite circuit MockProver fails on ECC `normalize` + Fixed column permutations even after:

- Poseidon distro path  
- note-commit bool_check fix  
- nk from esk+rho  
- NoteCommitment packing aligned to gadget  
- derive-cm-first (no free cm_old equality)

Isolated `note_commit` unit test is green. Next isolation step: **minimal Circuit with only note_commit+nullifier (no 32-layer Poseidon)** to separate layout interaction from gadget bugs.
