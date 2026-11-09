# HashMerchant support playbooks

Short recovery runbooks for **Path B** (validator / foreign-root mirror).  
Protocol detail lives in `x/hashmerchant/spec/` — do not fork it here.

| Playbook | When |
|----------|------|
| [sidecar-down.md](sidecar-down.md) | No VE data; `/health` fails; empty extensions |
| [root-stale.md](root-stale.md) | Sidecar up but on-chain root lagging |
| [escrow-expired.md](escrow-expired.md) | Roots confirm; contract sudo stopped |
| [wrong-chain-uid.md](wrong-chain-uid.md) | Mismatch between sidecar, registration, contract |

## Related

- Operator docs: `docs/hashmerchant/`
- Sidecar HTTP contract: `../sidecar-api.md`
- Support boundaries: `../SUPPORT.md`
- Install façade CLI (separate card): `../hm-support.sh` when present
- Lab truth: `tests/tsh/hashmerchant/`
