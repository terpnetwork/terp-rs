# voting-circuits on the zakura Halo2 fork

Source: `voting-circuits` **0.10.0** from crates.io (git sha1
`68d69c6d460426177f10ecf27a8181dfdb34ee67`), the exact version valargroup/vote-sdk
pins (`voting-circuits = "=0.10.0"`, vote-sdk main `5f23ec18`). License MIT OR
Apache-2.0 (files beside this one).

Why: upstream builds on crates.io `halo2_proofs` 0.3 / `orchard` 0.15, which cannot
serialize a `ConstraintSystem` or `VerifyingKey`. On the zakura fork (`halo2_proofs`
5.0.0-rc.3, `zakura-orchard` with `circuit-composition`) the keys serialize into the
chain's stored-circuit format (`params || CS || VK || footer`).

| Circuit | K | Public inputs | Ported |
|---|---|---|---|
| `vote_proof` (ZKP #2, cast) | 11 | 11 | yes |
| `share_reveal` (ZKP #3) | 10 | 9 | yes |
| `delegation` (ZKP #1) | 12 | 14 | no (needs Orchard note internals the fork keeps private; not used on chain here) |

## Changes (all mechanical, marked `zakura port` in the code)

- Imports: `orchard::circuit::{gadget, commit_ivk, note_commit}` and `orchard::constants`
  map to `orchard::circuit::composition::*`; SpendAuthG tables come from
  `composition::{SPEND_AUTH_U, SPEND_AUTH_Z, spend_auth_generator}`.
- Builder key material (`ask`, `nk`, `rivk`, `g_d`, `pk_d`) is read through the
  `composition::witness` adapters instead of crate-private accessors.
- rand 0.10 (`rand::rng()` for `OsRng`); prover bound `Circuit: Sync`, `Config: Send`.
- `src/delegation/` removed; two helpers only it used are `#[allow(dead_code)]`.
- Added `vote_proof::anchor` (not upstream): VAN leaf, vote-commitment-tree root/paths
  and `rsk_v` for hosts that pin a proposal anchor before voters prove.

## Faithfulness

The upstream `vote_proof::prove::tests::vk_fingerprint_unchanged` and
`share_reveal` fingerprint tests pass unchanged on the fork: the verifying key is
byte-identical to upstream's. `cargo test -p voting-circuits-zakura --release`
(plus `-- --ignored` for the proving tests) runs the upstream suite.
