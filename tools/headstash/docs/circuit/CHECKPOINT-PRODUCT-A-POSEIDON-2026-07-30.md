# Checkpoint: Product A Poseidon note + suite + keys path

**Date:** 2026-07-30  
**Branch:** `zk-mvp` (local checkpoint — not published)  
**Scope:** private distro + claim (Product A); SEAM shield remains Product B

## Landed

| Area | Status |
|------|--------|
| Poseidon private note `cmx` + lift nullifier | pure + gadget + synthesize |
| Poseidon-v1 distro leaf/path | suite + circuit |
| Suite tooling depth-32 `circuit_anchor` | sound vs shallow root |
| `suite_backed_claim_pair` MockProver | green |
| Offline keygen for zkvm upload | `build_headstash_keys_to` / `just demo-keys` |

## Commands

```bash
cd crates/headstash

# Fast Product A regression (no K=18 full prove)
just demo-product-a

# Suite unit tests
cargo test -p zk-headstash --lib suite::

# Offline store-circuit blob (long keygen; no network)
just demo-keys
# → artifacts/headstash_vk.bin (or HEADSTASH_VK_PATH)

# Optional upload (requires MNEMONIC; not for this checkpoint)
# MNEMONIC='…' cargo run -p zk-test-press --bin cc_headstash --features interface -- --upload
```

## Do not

- Publish / push without review  
- Commit mnemonics or key material  
- Mix shallow suite roots into on-chain genesis_root  
- Claim mainnet production  

## Next

1. Run `just demo-keys` once and verify footer / store-circuit on lab chain.  
2. Contract multi-test claim with mock ZK then real prove.  
3. FE: public mint → shield SEAM (Product B).

---

# Follow-up: strip unused Sinsemilla CS (2026-08-14)

**Session:** complete Poseidon note-commit layout (residual from `019fb059`).

## Landed

| Area | Status |
|------|--------|
| `NoteCommitChip` / dual Sinsemilla / MerkleChip / CommitIvk / LeafHash **not configured** on Product A `Circuit` | synthesize already Poseidon-only; configure matches |
| Range table | `load_kbit_range_table` loads `[0, 2^10)` for ECC / secp256k1 lookups |
| Helpers | `gadget.rs` no longer constructs unused Sinsemilla chips |

## Still residual (not on Product A prove path)

- `circuit/src/circuit/note_commit.rs`, `commit_ivk.rs`, `headstash_merkle_tree.rs` remain in-tree for legacy unit tests
- Off-circuit `sinsemilla` crate / Orchard ZIP vectors
- `commit_ivk` (IVK) still Sinsemilla if re-enabled later
- Existing `artifacts/headstash_vk.bin` is **stale** — VK/CS changed; re-run `just demo-keys`

## Tests run (host, no `interface`)

```
cargo test -p zk-headstash --lib --no-default-features \
  --features "circuit,std,host-crypto,multicore" -- \
  note_poseidon distro_poseidon note::commitment
# 27 passed
```

