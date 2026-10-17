# Raffle randomness

Atlas `app-contracts` drew winners from a Nois proxy and later from a Drand
verifier (`contracts/randomness_verifier`). Those crates are not in this
workspace, and the draw does not call them.

`nftfi-raffle` stores a `RandomnessSource` set at instantiate:

| Source | Who may draw | What the contract stores |
| --- | --- | --- |
| `Seed { provider }` | `provider`, or any account when `provider` is omitted | the 32-byte seed, round `0`, and the sender |
| `Callback { verifier }` | any account calls `RequestDraw` | 32 bytes returned by `verifier` |

`Callback` sends `VerifierExecuteMsg::Provide { raffle_id, owner }` to
`verifier`. The reply must be a success whose wasm attributes include:

- `raffle_id` — decimal raffle id
- `round` — decimal round number
- `randomness` — base64 of exactly 32 bytes
- `owner` — bech32 account credited as the provider

The winner index is `sha256(seed || raffle_id_be || salt_be)` reduced modulo
the remaining ticket pool (`nftfi_randomness::select_tickets`). The same seed
always picks the same tickets. A raffle with zero sold tickets, or fewer than
`min_ticket_number`, is claimed without a draw: prizes return to the owner and
ticket funds are refunded.

`Seed { provider: None }` is for tests and local chains. A production instantiate
should set a provider, or use `Callback` with a verifier you operate.
