# Smart-account authenticators

CosmWasm authenticators for Terp `x/smart-account`, with shared types in-tree:

| Crate | Role |
|-------|------|
| `crates/terp-auth` | `AuthSudoMsg`, request types, `TerpAccountTrait` |
| `crates/terp-account` | `traits::default::TerpAccountTrait` → `TerpAccountTrait` |
| `terp-authenticator-suite` | **moved** → `tests/suite/terp-authenticator-suite` (cw-orch Deploy + Mock) |
| `terp-zkjwt` | ZK proof of JWT / OIDC identity |
| `terp-vsck` | VSCK / vote-sdk private voting module auth |
| `terp-zkposiedon` | Poseidon membership scaffold |

## zk-jwt (web2-fluent identity)

Prove possession of a valid issuer JWT **without publishing the token**.

Useful extensions for existing websites:

- **SSO recovery** — Google/GitHub login recovers a smart account  
- **Email-domain gates** — `@company.com` for org DAOs  
- **Profile linking** — OAuth session → `RegisterClaim` → chain profile  
- **KYC reuse** — issuer attestation JWT, privacy-preserving  
- **Session continuity** — browser keeps OIDC; extension builds ZK for txs  

Proof API: `terp_zkjwt::verify::ZkJwtVerifier` (default envelope stub; plug circuits).

## VSCK (DAO private voting)

`terp-vsck` lets a DAO register an authenticator that requires vote-sdk proofs:

| Role | Circuit | vote-sdk module |
|------|---------|-----------------|
| Delegator | Delegation | voting power / note |
| Voter | VoteProof | private ballot |
| Tallier | ShareReveal | ceremony open |

Sessions bind `note_tree_root` from the Shielded Vote workflow; nullifiers prevent double ballots on the host chain.

## Tests

```bash
cargo test -p terp-authenticator-suite
```
