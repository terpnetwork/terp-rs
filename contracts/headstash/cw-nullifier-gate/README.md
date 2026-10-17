# cw-nullifier-gate

Thin **CosmwasmAuthenticator** nullifier gate for vote-sdk-on-Terp.

**Design:** [`docs/research/DAO-CEREMONY-NULLIFIER-AUTH.md`](../../../../docs/research/DAO-CEREMONY-NULLIFIER-AUTH.md)

| Hook | Behavior |
|------|----------|
| Authenticate | Query ceremony module `IsSpent` (prefer **raw** spent-map layout on ante) — reject if spent |
| Track | **No-op** (never burn on failed execute) |
| ConfirmExecution | Emit MarkSpent payload toward ceremony module |

**Does not own** spent nullifiers (SSOT = `cw-vote-ceremony`).

## Params

```json
{
  "ceremony_module": "terp1…",
  "domain": "terp.vote.v1",
  "session_id": "optional-fixed",
  "require_session": true,
  "max_nullifiers_per_tx": 8,
  "prefer_raw_query": true
}
```

## Gas note

Raw storage read of ceremony Map namespace `spent` is preferred over smart query on the ante hot path. Scaffold unit tests use smart-query mock; production should wire host raw_query with `cw-vote-ceremony` `raw_keys` encoding.

## ConfirmExecution / MarkSpent

Ceremony module accepts `SudoMsg::MarkSpent`. Gate emits a Wasm execute carrying the MarkSpent payload for integration visibility; app/keeper wiring should privilege-elevate to ceremony **sudo** (OD-mark-spent).

```bash
cargo test
```
