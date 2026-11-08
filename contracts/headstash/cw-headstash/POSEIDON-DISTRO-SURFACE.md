# Poseidon-v1 distro surface (cw-headstash)

How UI, suite, and manifold pass **Poseidon** public-inclusion roots, register **additive** Headstash sets, and domain-separate claim nullifiers.

**Product router:** `cw-headstash` is the **on-chain mint/router** for eligibility claims and (planned) bridge mints / note-entry paths — not a separate mint contract. Asset maps may live **internal** to this contract and/or an **external** registry with cross-chain wiring. See `docs/plans/spectrum/CLARITY-cw-headstash-router-and-asset-registry.md`.

Related:

- `docs/plans/spectrum/ADR-POSEIDON-DISTRO-TREE.md`
- `docs/plans/spectrum/CLARITY-headstash-sets-and-bridge-models.md` §1
- Circuit helpers: `crates/headstash/circuit/src/distro_poseidon.rs` (when present)

---

## 1. Hash domain

| Tag | Enum | Use |
|-----|------|-----|
| `poseidon-v1` | `DistroHashDomain::PoseidonV1` | **Default** for all new Headstashes and additive roots |
| `sinsemilla-legacy` | `DistroHashDomain::SinsemillaLegacy` | Recovery of pre-Poseidon trees only — **not** for new drops |

Personalization (must match circuit + suite builders):

```text
POSEIDON_DISTRO_LEAF  = Poseidon( personalization="terp-hs-distro-leaf-v1",  fields… )
POSEIDON_DISTRO_CRH   = Poseidon( personalization="terp-hs-distro-crh-v1",   layer, left, right )
```

On-chain, roots are **32-byte** digests. Empty roots are rejected. Unknown domain tags are rejected at parse time.

---

## 2. Instantiation (UI / suite)

```json
{
  "genesis_root": "<base64 32-byte Poseidon-v1 root>",
  "distro_hash_domain": "poseidon_v1",
  "genesis_label": "drop-0",
  "token_strategy": { "...": "..." },
  "wavs": { "...": "..." }
}
```

Notes:

- Wire enum is `poseidon_v1` / `sinsemilla_legacy` (CosmWasm schema identifiers). ADR display tags are `poseidon-v1` / `sinsemilla-legacy` via `as_str()`.
- Omit `distro_hash_domain` → defaults to **Poseidon-v1**.
- Genesis is always registered as **`root_id = 0`**.
- Also stored on `HeadstashCfg.gr` / `GENESIS_TREE_ROOT` for backward compatibility.

### Suite / off-chain builder checklist

1. Build the eligibility Merkle tree with **Poseidon-v1** leaf + CRH (not Sinsemilla).
2. Export the 32-byte root.
3. Pass it as `genesis_root` with domain `poseidon-v1`.
4. Keep the same tree fixture for circuit instance `anchor` when proving claims.

---

## 3. Additive roots (multi-Headstash privacy surface)

Each new drop **adds** a root; it does not replace prior anonymity.

### On `cw-headstash` (source of truth)

```json
{
  "register_eligibility_root": {
    "root": "<base64 32-byte root>",
    "domain": "poseidon_v1",
    "label": "drop-1"
  }
}
```

- **Owner-only**.
- Assigns the next `root_id` (`1`, `2`, …).
- Additive policy: only **Poseidon-v1** for new registrations (Sinsemilla rejected).

### On manifold (index / composition)

```json
{
  "register_eligibility_root": {
    "headstash": "terp1…",
    "root": "<base64>",
    "domain": "poseidon_v1",
    "label": "drop-1",
    "forward_to_headstash": true,
    "root_id": 1
  }
}
```

- Manifold keeps `MANIFOLD_ROOTS: (headstash, root_id) → record` for listing active roots across drops.
- Prefer `forward_to_headstash: true` so the Headstash contract assigns/stores the root; pass `root_id` when known to index immediately.
- Queries: `EligibilityRoots { headstash }`, `EligibilityRoot { headstash, root_id }`.

---

## 4. Claims bind to `root_id`

```json
{
  "process_headstash": {
    "claims": [
      {
        "i": {
          "anchor": "<must match registered root for root_id>",
          "nd": "...",
          "v": 1000,
          "nf": "...",
          "recp": "...",
          "cmx": "..."
        },
        "p": "<proof bytes>",
        "rr": "<canonical recipient>",
        "root_id": 0
      }
    ]
  }
}
```

| Field | Role |
|-------|------|
| `root_id` | Which registered eligibility set (default `0` = genesis) |
| `i.anchor` | Public instance root — must equal the registered root bytes for that id (when non-empty) |
| `p` | Halo2 proof; circuit greening may lag this surface |

**Reject paths (no full ZK required for these checks):**

- empty root at register/instantiate  
- unknown `distro_hash_domain`  
- claim `root_id` not in `ELIGIBILITY_ROOTS`  
- double nullifier under the same `root_id`  

---

## 5. Nullifier domain separation

Nullifiers are stored under a **domain-separated key**:

```text
storage_key = "{root_id}:{nullifier_hex}"
```

Implications:

- Double-claim of the **same** leaf under the **same** root is rejected.
- The same nullifier encoding under **different** `root_id`s does **not** collide (set \(i\) vs set \(j\) are independent claim scopes — CLARITY §1).
- Future OR-membership circuits that hide which root was used still need on-chain one-shot keys; this map remains the double-spend ledger.

When querying `Nullifer { null }`, pass the **storage key** (`"0:ab…"`) or the raw historical key if any pre-domain entries exist.

---

## 6. Query surface

| Query | Returns |
|-------|---------|
| `DistroConfig {}` | default domain, tag string, `next_root_id`, genesis root |
| `EligibilityRoot { root_id }` | single entry |
| `EligibilityRoots { start_after, limit }` | paginated additive list |

---

## 7. Circuit lag

This surface is intentionally **ahead** of full Poseidon-v1 inclusion prove greening:

- State machine + msg/query are live.
- `process_headstash` still calls `Api::proof_instance_verify` when the zk feature is enabled; instance layout must eventually commit to the registered Poseidon root as `anchor`.
- Do **not** mix Sinsemilla-built roots with `distro_hash_domain = poseidon-v1`.

---

## 8. Minimal integration sketch

```text
suite: build Poseidon-v1 tree → root_bytes
  → InstantiateMsg { genesis_root: root_bytes, distro_hash_domain: PoseidonV1 }
  → (later) RegisterEligibilityRoot { root: root_bytes_1, label: "drop-1" }
  → prove claim with anchor = root for root_id
  → ProcessHeadstash { claims: [{ root_id, i.anchor = root, … }] }
```
