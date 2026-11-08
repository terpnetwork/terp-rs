# HashMerchant — support role

This document defines what **support systems** do and do **not** do when wiring HashMerchant Path B.

Operator guides: `docs/hashmerchant/`.  
Protocol SSOT: `x/hashmerchant/spec/`.  
Playbooks: `playbooks/`.

## What support delivers

Support delivers a **repeatable, auditable wiring** of components the chain already understands:

1. Prerequisites (terpd present — use `terp-installer` if not)
2. Sidecar health (`/health`, `/vote-extension`)
3. Node config (`hashmerchant.sidecar-url` / `HASHMERCHANT_SIDECAR_URL`)
4. Contract register + escrow
5. Smoke: root confirms; optional inclusion verify; escrow still paid
6. Recovery via playbooks (sidecar / root / escrow / chain_uid)

## What support never owns

| Never | Why |
|-------|-----|
| Customer CRM / loyalty graph (Path A DB) | Operator data plane; support only handles published trees/roots when productized |
| Mainnet private keys on USB / chat | Authenticate on operator-controlled keyrings only |
| Inventing foreign roots | Consensus quorum only |
| Claiming HashMerchant is “private payments” | Module is **Merkle reflection + optional ZK-friendly rehash**, not a payments product |
| Minting value from oracle mids | Oracle may bound **price**; it does not create fungible supply |

## Path B vs Path A (label everything)

| Path | Job | Support v0 |
|------|-----|------------|
| **B — Foreign-root mirror** | Sidecar → VE → quorum `HashRoot` → sudo | **In scope** (e2e in `tests/tsh/hashmerchant/`) |
| **A — Operator program tree** | Snapshot operator DB → leaves/root → claims | Document only / thin whitelist demos — not full CRM product |

## Engagement lifecycle

```
Prepare → Install → Authenticate → Configure → Deploy → Integrate → Smoke/Monitor → Scale
```

| Stage | Exit |
|-------|------|
| Prepare | Path B scope, chain_uid, who holds keys |
| Install | Artifacts + checksums (installer façade / USB kit when available) |
| Authenticate | Keyring policy; no secrets on media |
| Configure | sidecar-url live; sample VE JSON good |
| Deploy | wasm code_id pinned |
| Integrate | register-contract + escrow query OK |
| Smoke/Monitor | Root age OK; playbook linked |
| Scale | Second env from same recipe |

## Threat boundaries (support checklist)

- [ ] Sidecar URL is operator-controlled
- [ ] Empty VE on sidecar failure (safe) — verified in logs once
- [ ] Quorum understood: one sidecar ≠ confirmed root
- [ ] Escrow monitoring owned by contract operator
- [ ] No PII required for Path B mirror smoke
- [ ] Marketing/loyalty copy not used as acceptance criteria

## Commands cheat sheet

```bash
terpd q hashmerchant params
terpd q hashmerchant chains
terpd q hashmerchant chain <chain_uid>
terpd q hashmerchant root <chain_uid> <algo>
terpd q hashmerchant escrow <contract>

terpd tx hashmerchant register-contract <contract> <chain_uid> <substore-keys> <escrow> --from <key>
terpd tx hashmerchant refill-escrow <contract> <amount> --from <key>

curl -sf "$HASHMERCHANT_SIDECAR_URL/health"
curl -sf "$HASHMERCHANT_SIDECAR_URL/vote-extension"
```

## Further reading

- Golden path: `docs/hashmerchant/golden-path.md`
- Config: `docs/hashmerchant/config.md`
- Spec overview: `x/hashmerchant/spec/README.md`
