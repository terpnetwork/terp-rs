# terp-vsck

**VSCK / Shielded Vote authenticator** — register a CosmWasm authenticator so a
DAO can require **private voting proofs** (vote-sdk circuit workflow) before
sensitive actions execute.

## Relation to vote-sdk

[vote-sdk](../../../vote-sdk) (Shielded Vote Chain) defines:

- Halo2 circuits: **delegation**, **vote_proof**, **share_reveal**
- EA key ceremony, note trees, nullifiers
- Coordinator policy for session operations

This contract is the **CosmWasm smart-account / voting-module gate** that:

1. Stores per-session `note_tree_root` (from vote chain or IBC bridge)
2. Verifies role-appropriate proofs via [`verify::VsckVerifier`](src/verify.rs)
3. Tracks nullifiers so ballots cannot be double-spent on the host chain

Cryptographic verify is **fail-closed**. Envelope + VK + role/circuit +
session-root prefix binding always run; `vote-sdk` Halo2 is not linked, so
authenticate returns `CircuitNotLinked`. Feature `vote-circuits` is the hook
for `shielded-vote-circuits` — it does not accept stub proofs.

## DAO integration

```
DAO core
  └─ voting module (or proposal execution)
       └─ smart-account authenticator = terp-vsck
            ├─ OpenSession(session_id, note_tree_root, proposal_ref)
            ├─ voters Authenticate(VsckAuthPayload { VoteProof, … })
            └─ talliers Authenticate(ShareReveal) for tally finalize msgs
```

Typical message auth matrix:

| Message intent | Role | Circuit |
|----------------|------|---------|
| Register voting note | Delegator | Delegation |
| Cast / change ballot commitment | Voter | VoteProof |
| Submit tally share | Tallier | ShareReveal |
| Open/close session | DAO/admin execute | (not sudo) |

## Why an authenticator (not only the vote chain)

- Host DAOs on Terp can **reuse** private ballots while settlement stays on VSCK
- Composite authenticators: `AllOf(passkey, vsck)` for “human + private vote”
- Progressive adoption: mock verifier in CI; production VKs from ceremony

## See also

- `terp-zkjwt` — OIDC/JWT identity proofs for profiles  
- `terp-authenticator-suite` — Mock deploy + sudo helpers  
- vote-sdk `circuits/src/lib.rs` — delegation / vote_proof / share_reveal APIs  
