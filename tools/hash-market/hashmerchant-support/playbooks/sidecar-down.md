# Playbook: sidecar down

**Audience:** support systems, validator ops  
**Path:** B (foreign-root mirror)  
**Related:** `sidecar-api.md`, [config guide](../../../docs/hashmerchant/config.md)

## Symptoms

- No new `HashRoot` confirmations (`terpd q hashmerchant root <chain_uid> <algo>` lagging)
- Node logs: `sidecar fetch failed, returning empty extension` (or equivalent)
- `hm-support` smoke / `curl` against sidecar fails `/health`
- Contracts stop receiving fresh sudo roots while chain still produces blocks

## Quick checks

```bash
# 1. Health
curl -sf "${HASHMERCHANT_SIDECAR_URL}/health" | jq .

# 2. Vote-extension body (must match registered chain_uid + algo)
curl -sf "${HASHMERCHANT_SIDECAR_URL}/vote-extension" | jq .

# 3. Node still listening for empty VE (safe)
# Empty extensions do NOT halt consensus — only stop root confirmation.
```

## Actions

1. Confirm process/container is up; restart sidecar if dead.
2. Confirm network path **from the validator process** to the sidecar (localhost vs mesh DNS vs Docker bridge).
3. Validate `/vote-extension` JSON against `tools/hashmerchant-support/sidecar-api.md` (legacy simple body or modular attestations).
4. Confirm `app.toml` `[hashmerchant] sidecar-url` or `HASHMERCHANT_SIDECAR_URL` points at the live base URL (no trailing path mistakes).
5. If prolonged outage: **leave** `sidecar-url` set (empty VE is safe) or clear URL intentionally; do not invent emergency roots.

## Do not

- Paste emergency roots by hand into consensus
- Disable vote-extension verification to “force” roots
- Point production validators at untrusted public sidecars without custody review

## Exit criteria

- `/health` returns OK
- `/vote-extension` returns well-formed JSON every interval
- After quorum of healthy validators, `terpd q hashmerchant root <chain_uid> <algo>` advances
