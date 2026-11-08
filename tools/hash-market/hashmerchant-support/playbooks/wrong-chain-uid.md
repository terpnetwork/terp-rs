# Playbook: wrong chain_uid / algo mismatch

**Audience:** support systems, integrators  
**Path:** B  
**Related:** [config](../../../docs/hashmerchant/config.md), `sidecar-api.md`

## Symptoms

- Sidecar healthy; still no confirmed root for expected chain
- `VerifyVoteExtension` rejects peer extensions (logs: chain not registered / disabled)
- Root exists for a **different** uid (e.g. `ethereum-lab` vs `ethereum-mainnet`)
- Contract registered on uid A while sidecar publishes uid B

## Quick checks

```bash
terpd q hashmerchant chains
curl -sf "${HASHMERCHANT_SIDECAR_URL}/vote-extension" | jq '{chain_uid, algo}'

# Compare exactly — case and spelling matter
terpd q hashmerchant chain ethereum-mainnet
terpd q hashmerchant root ethereum-mainnet keccak256
```

## Actions

1. List registered chains; pick the **canonical** `chain_uid` governance registered.
2. Align sidecar config (`chain_uid`, `algo`) to that registration.
3. Align contract registration: `register-contract` uses the same `chain_uid`.
4. Align demo/mint scripts and env vars (`CHAIN_UID`, `ALGO`) — one source of truth per environment.
5. If lab vs mainnet labels differ, document the mapping in the engagement checklist; never assume aliases.

## Do not

- “Fix” by registering duplicate chains without ops agreement
- Mix `oracle-agg-v1` roots with contracts that only understand `keccak256` without reading multi-source spec

## Exit criteria

- Sidecar VE `chain_uid` + `algo` == registered chain + contract registration
- Root query for that pair advances under quorum
