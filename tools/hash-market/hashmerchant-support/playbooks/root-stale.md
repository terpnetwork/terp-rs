# Playbook: root stale / missing

**Audience:** support systems, chain ops, contract integrators  
**Path:** B (foreign-root mirror)  
**Related:** [golden path](../../../docs/hashmerchant/golden-path.md), `x/hashmerchant/spec/05_vote_extensions.md`

## Symptoms

- Contracts have old sudo roots or none
- `terpd q hashmerchant root <chain_uid> <algo>` empty or lagging foreign height
- Mint / price-sync demos reject height as too old
- Sidecar healthy but on-chain root never updates

## Quick checks

```bash
terpd q hashmerchant chains
terpd q hashmerchant chain <chain_uid>
terpd q hashmerchant root <chain_uid> <algo>
terpd q hashmerchant params

curl -sf "${HASHMERCHANT_SIDECAR_URL}/vote-extension" | jq '{chain_uid, algo, foreign_height, root}'
```

## Actions

1. **Chain registered and enabled** — governance must have registered `chain_uid` with matching hash `algo` (e.g. `keccak256`).
2. **Sidecar payload matches** — `chain_uid` and `algo` in VE JSON must match registration exactly (typos → reject / no tally bucket).
3. **Quorum** — enough voting power must run sidecars that agree on the **same** root. One lonely sidecar never confirms.
4. **Vote extensions enable height** — chain must have passed the height where VE are active for the app.
5. **Contract path** — registration + non-expired escrow (see `escrow-expired.md`); otherwise sudo may not deliver even when roots confirm.
6. **Foreign source lag** — if sidecar feeds a stalled RPC, roots stall with the foreign chain; fix the feed, not the module.

## Mint / price demos

- Treat stale oracle mid as **do not mint under old price**
- Fail closed on price-sync if foreign height is older than policy threshold
- Oracle bounds **price only** — it does not mint fungible value or invent NFT supply

## Do not

- Lower `quorum_fraction` casually via governance to “make roots appear”
- Assume Path A loyalty CRM roots are the same as Path B foreign state roots

## Exit criteria

- Root query returns a recent `foreign_height` consistent with sidecar
- Registered contracts with active escrow receive sudo on confirmation
- Support smoke log shows root age within policy
