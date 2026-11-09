# Origin

- Repo: https://github.com/AtlasDA0/app-contracts
- Commit: `4b921920092004f4a79724e3c0db3f7993b64be7`
- Paths: `packages/randomness/src/lib.rs`, `contracts/raffles/src/randomness/mod.rs`, `contracts/randomness_verifier/`

## What changed

- Removed the Drand signature payload (`DrandRandomness`) and the Nois proxy fields.
- `RandomnessSource::Seed` is the testable source: a provider submits 32 bytes, and the contract hashes them with the raffle id.
- `RandomnessSource::Callback` keeps the upstream split where a second contract attests entropy. The reply attributes are `raffle_id`, `round`, `randomness`, and `owner`. No Drand or Nois verifier is included.
- Winner selection is `select_tickets` (sha256, without replacement). Upstream called into `nois::ints_in_range`.
