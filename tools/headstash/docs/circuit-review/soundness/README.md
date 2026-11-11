# vk_id 3 claim circuit — soundness and FV scope

Documents only. The circuit, the contract, and the verifying key were not
modified. Branch `wip/hs-circuit-soundness-review`.

The production circuit is `NoteMembershipCircuit` in
`tools/headstash/circuits/halo2/src/note_membership.rs`: Halo 2, K = 11,
six public inputs, depth-17 Poseidon Merkle membership, Poseidon
nullifier, destination limbs copied to the instance. The k19 lab measured
1,598 used rows. Registration signatures stay in `cw-headstash`. The
contract rebuilds the instance and calls `proof_instance_verify`.

## Read in this order

| File | Contents |
| --- | --- |
| [00-methodology.md](00-methodology.md) | rot256 Lean skill as applied here: claim, decomposition, no Lake build on this host. |
| [01-prior-art.md](01-prior-art.md) | Ironwood, dregg, verified-zkEVM / clean / ArkLib / zkLean. What can be imported. |
| [02-scoping.md](02-scoping.md) | The relation, in `synthesize` order, and the contract limbs around it. |
| [03-components.md](03-components.md) | Poseidon, swap, nullifier, booleans, public inputs, destination. Risks and Lean gaps. |
| [04-fv-plan.md](04-fv-plan.md) | Batches 0–8, dependencies, person-week estimates. |
| [05-findings.md](05-findings.md) | F1–F9. No critical issue in the pinned vk_id 3 key. |

## Result in one paragraph

vk_id 3 pins every Poseidon tag and every Merkle layer index, so the
vk_id 2 nullifier-tag bug does not apply to this key. The swap flag is a
real boolean constraint. The four hashes are `P128Pow5T3` /
`ConstantLength` at lengths 2, 3, and 4, which Ironwood already models.
What the circuit does not prove, and the contract currently does, is that
the amount is a denomination and that the destination limbs are the
transaction sender. A Lean development should import Ironwood's Poseidon
and `CondSwap` after a diff against `halo2_gadgets-0.5.0`, prove the
17-layer composition, and keep the contract obligations in a separate
theorem. The Halo 2 verifier and the K=19 secp chip are out of that
critical path.
