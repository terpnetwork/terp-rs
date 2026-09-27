# HashMerchant support tooling

Support-facing façade for the **already-proven** Path B mirror path  
(`tests/tsh/hashmerchant/`) plus **demo automation**: feegrants, loyalty whitelist, oracle-priced NFT mint recipes.

Plan: `docs/bootstrap/hashmerchant-loyalty-hermes-kanban-plan.md`  
Field e2e ladder (mock default): [`docs/hashmerchant/FIELD-E2E.md`](../../docs/hashmerchant/FIELD-E2E.md) · [`field-ladder-mock.sh`](./field-ladder-mock.sh) · residuals [`FIELD-RESIDUAL.md`](./FIELD-RESIDUAL.md)

## Quick start

```bash
cd tools/hashmerchant-support
chmod +x hm-support.sh accounts/*.sh demo/nft-oracle-mint/recipe.sh usb/build-kit.sh

./hm-support.sh prepare
./hm-support.sh install
./hm-support.sh whitelist self-test
./hm-support.sh whitelist build \
  --in whitelist/fixtures/members.json \
  --out /tmp/loyalty-wl
./hm-support.sh accounts feegrant \
  --file whitelist/fixtures/members.json \
  --granter ops \
  --dry-run
./hm-support.sh accounts verify \
  --file whitelist/fixtures/members.json \
  --granter ops \
  --dry-run
./hm-support.sh demo nft-help
```

Feegrant defaults, mainnet guard, and shared `loyalty-members/v1` schema:
see `accounts/README.md`.

## Layout

| Path | Role |
|------|------|
| `hm-support.sh` | CLI entry (installer façade card — may land on sibling branch) |
| `lib.sh` | Shared helpers |
| `accounts/` | Key bootstrap + feegrant batch |
| `whitelist/` | Merkle eligibility tree |
| `demo/nft-oracle-mint/` | Oracle price + whitelist mint recipe |
| `playbooks/` | Sidecar / root / escrow / chain_uid recovery |
| `SUPPORT.md` | Support role boundaries |
| `sidecar-api.md` | HTTP contract for sidecars |
| `usb/` | Offline kit packaging |

## Operator docs (monorepo)

Mirrored narrative for git-tracked browsing and terp-docs:

- `docs/hashmerchant/` — golden path, config, support role, playbook copies  
- Protocol SSOT: `x/hashmerchant/spec/` (link; do not fork)

## What this is not

- Not `terp-installer` (node binary bootstrap)  
- Not a full CRM Path A product  
- Not a private payments product (Merkle reflection + optional ZK-friendly proofs)  
- Oracle does **not** mint value or NFT supply
