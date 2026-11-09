# `terp-dregg` — federation settlement authenticator

CosmWasm **x/smart-account** authenticator that gates txs on a **dregg federation light-client head** (claim-limited settlement wrap).

**Does not** store Discord IDs or merge dregg capabilities into Cosmos identity.

Design: `docs/agentics/DESIGN-IBC-WASM-V2-DREGG-FEDERATION-LC.md`  
Sibling LC: `contracts/light-clients/cw-ics08-wasm-dregg`

## Modes

| mode | Behavior |
|------|----------|
| `query_lc` | Query `lc_contract` for `ProvenHeight` / `IsProvenRoot` |
| `embedded_head` | Local min height pin only (lab); prefer `query_lc` for demos |

## Authenticate evidence (JSON in signature / params)

```json
{
  "proven_root_key": "<hex>",
  "proven_height": 3
}
```

Pass if `proven_height >= min` and (optional) root is in LC history.
