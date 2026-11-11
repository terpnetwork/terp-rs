# vk_id 3 claim circuit — optimization review

Documents only. No circuit, contract, or config edits. Branch `wip/hs-circuit-opt-review`.

The production circuit is `NoteMembershipCircuit` (`NoteCircuitImpl<true>`) in `tools/headstash/circuits/halo2/src/note_membership.rs`. It is Halo2 on Terp's zakura-halo fork (`~/abstract/terp-core/crates/zakura-common`, IPA over Pasta, Pallas base field). Configured constants: `NOTE_DEPTH = 17`, `NOTE_K = 11`, `NOTE_INSTANCES = 6`, `NOTE_VK_ID = 3`, proof length `NOTE_PROOF_BYTES = 2752` (`note_membership.rs:40-44`, `:639`).

It proves three things, and nothing else:

1. The leaf `Poseidon(terp-hs-note-leaf-v2, key_com, amount)` sits at a private index in a depth-17 Poseidon Merkle tree (`note_membership.rs:151-165`).
2. `nullifier = Poseidon(terp-hs-nf-v1, secret, ctx)` (`note_membership.rs:152-153`).
3. The destination limbs the prover used are the public instance limbs (`note_membership.rs:167-169`). The contract, not the circuit, packs `info.sender` into those limbs.

Registration signatures (EIP-191 ECDSA or Solana ed25519) are checked in `contracts/headstash/cw-headstash/src/notes.rs` (`verify_signer`, around `:427`). They never enter this circuit.

## Cost model in one paragraph

The scarce resources are the Halo2 domain `2^K` and the number of committed columns. Filled rows matter only when they cross a power of two. This circuit uses **1,598** rows. Usable rows at K=11 are **2,042** (domain 2,048 minus 6 unusable rows). K=10 has 1,018 usable rows, and 1,598 does not fit. `note_k_is_minimal` records that (`note_membership.rs:560-562`). Row cuts that stay above 1,018 do not change proof size, `proof_instance_verify` gas, or the MSM/FFT domain the browser prover pays for.

## Files

| File | Contents |
|---|---|
| [00-methodology.md](00-methodology.md) | rot256 `circuit-optimization` method, what was applied, what does not apply |
| [01-inventory.md](01-inventory.md) | Columns, gates, degree, rows, with `path:line` |
| [02-accounting.md](02-accounting.md) | How the regions sum to 1,598, and the K=11 headroom |
| [03-opportunities.md](03-opportunities.md) | Each candidate: row/column/degree/K delta, soundness, verify gas, prove time |
| [04-recommendations.md](04-recommendations.md) | Priority table |
| [05-do-not-change.md](05-do-not-change.md) | Invariants, including the pinned tags and the public-input ABI |
| [06-k15-comparison.md](06-k15-comparison.md) | vk3 against the K=15 secp256k1 four-design table |

Zakura sources cited as `zakura-common/crates/...` live in the sibling checkout `~/abstract/terp-core/crates/zakura-common`, not inside this worktree.
