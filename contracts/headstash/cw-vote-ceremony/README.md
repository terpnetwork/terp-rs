# cw-vote-ceremony

DAO **ceremony module** contract — nullifier **system of record** for vote-sdk-on-Terp.

**Design:** [`docs/research/DAO-CEREMONY-NULLIFIER-AUTH.md`](../../../../docs/research/DAO-CEREMONY-NULLIFIER-AUTH.md)

## Owns

| State | Key | Notes |
|-------|-----|--------|
| Ceremonies | `ceremony` / session_id | status, domain, registration window |
| Registrations | `reg` / (session, addr) | participant workflow entry |
| Spent nullifiers | `spent` / (domain, session, nf) | **SSOT** — V2 gate only raw-queries |

## Does not own

- Smart-account ante hooks (V2 `cw-nullifier-gate`)
- ZKP verify (V4 Path A)
- Full depth-24 tree (V5 anchor registry is separate)

## DAO registration (operator)

1. Instantiate this contract (set `admin` to DAO execute address or module account).
2. Register the contract address as a **DAO member / dao-dao module** (or proposal-callable target).
3. Pass proposal → `ExecuteMsg::StartCeremony { session_id, domain, … }`.
4. Participants `Register { session_id }`.
5. Voter smart accounts add thin nullifier-gate authenticator with  
   `params.ceremony_module = <this bech32>` (V2).
6. On successful vote tx, gate `ConfirmExecution` → chain sudo `SudoMsg::MarkSpent`.

### Raw query (ante)

Prefer storage raw read of Map namespace **`spent`** over smart query. See `src/raw_keys.rs`.

```bash
cargo test
```

## Sudo

```json
{"mark_spent":{"domain":"terp.vote.v1","session_id":"round-1","nullifiers":["BASE64..."]}}
```

Only intended from CosmwasmAuthenticator post-handler / module privilege — not public execute.
