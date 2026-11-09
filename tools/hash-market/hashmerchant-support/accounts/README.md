# Accounts: feegrant bootstrap for demo defaults

Phase 1b §4.3.A — default demo accounts + gas sponsorship via `x/feegrant`.

| Script | Role |
|--------|------|
| `bootstrap.sh` | Create keyring keys, resolve addresses, optional dust fund, optional feegrant |
| `feegrant-batch.sh` | Batch `tx feegrant grant` (Basic or AllowedMsgAllowance) |
| `verify.sh` | Query allowances; optional smoke bank-send with `--fee-granter` |

Entry via façade:

```bash
./hm-support.sh accounts bootstrap --file … --out …
./hm-support.sh accounts feegrant  --granter ops --file … [--dry-run]
./hm-support.sh accounts verify    --file … [--granter ops] [--smoke]
```

## Shared `members.json` schema (`loyalty-members/v1`)

**Same file** feeds feegrant bootstrap **and** whitelist Merkle tooling (`whitelist/build.py`).

```json
{
  "version": "loyalty-members/v1",
  "members": [
    {
      "name": "alice",
      "address": "terp1…",
      "key": "alice"
    }
  ]
}
```

| Field | Required | Notes |
|-------|----------|--------|
| `version` | recommended | `loyalty-members/v1` |
| `members` | yes* | Or bare JSON array of member objects |
| `name` | optional | Display / demo label |
| `address` / `addr` | one of address or key | Bech32; whitelist **requires** address |
| `key` | optional | Keyring name for bootstrap when address empty |

\* Bare array form is also accepted by all scripts:

```json
[{"name":"alice","address":"terp1…"}]
```

Golden fixture (offline placeholders): `../whitelist/fixtures/members.json`.

Bootstrap writes resolved:

```json
{"version":"loyalty-members/v1","members":[{"name":"…","key":"…","address":"terp1…"}]}
```

## Defaults (caps, expiry, denom)

| Setting | Default | Override |
|---------|---------|----------|
| Spend limit | `1000000uterp` | `--limit` / `HM_FEEGRANT_LIMIT` |
| Expiry | 30 days (RFC3339 UTC) | `--expiry-days N` (`0` = none) / `HM_FEEGRANT_EXPIRY_DAYS` |
| Denom (dust / fees) | `uterp` | `HM_DENOM`, dust via `--dust 1000uterp` |
| Allowed msgs | wasm execute + instantiate + bank send | `--allowed-msgs CSV` / `HM_FEEGRANT_MSGS` |
| Basic-only allowance | off | `--basic` |
| Tx fees (CLI) | `400000uterp` | `HM_FEES` |
| Keyring | `test` | `HM_KEYRING` |

Default allowed message type URLs:

```
/cosmwasm.wasm.v1.MsgExecuteContract
/cosmwasm.wasm.v1.MsgInstantiateContract
/cosmos.bank.v1beta1.MsgSend
```

## Mainnet guard

`lib.sh` `hm_guard_mainnet` refuses chain ids `morocco-1`, `mainnet`, `terp-mainnet` unless:

```bash
export HM_MAINNET_OK=true
```

Dry-run mode skips chain calls and does **not** require `terpd` in PATH.

## Live lab sequence

```bash
export BIND=terpd
export HM_CHAIN_ID=localterp   # not mainnet
export HM_NODE=tcp://127.0.0.1:26657
export HM_HOME=$HOME/.terpd
export HM_KEYRING=test

# 1) Keys + addresses (optional dust from funder)
./hm-support.sh accounts bootstrap \
  --file whitelist/fixtures/members.keys.json \
  --out /tmp/members.resolved.json \
  --funder ops --dust 1000uterp \
  --feegrant --granter ops

# 2) Or feegrant alone against already-resolved members
./hm-support.sh accounts feegrant \
  --granter ops --file /tmp/members.resolved.json \
  --limit 1000000uterp --expiry-days 30

# 3) Verify queries + optional sponsored smoke
./hm-support.sh accounts verify \
  --file /tmp/members.resolved.json \
  --granter ops --smoke
```

### Exit criteria

- N members have clean `q feegrant grant <granter> <grantee>` (or grantee allowances list).
- Unfunded grantee completes a smoke tx with `--fee-granter <granter>` (gas paid by granter).
- Whitelist can consume the same resolved `members.json`.

## Offline / no `terpd`

```bash
./hm-support.sh accounts feegrant --file whitelist/fixtures/members.json --granter ops --dry-run
./hm-support.sh accounts bootstrap --file whitelist/fixtures/members.keys.json \
  --out /tmp/m.resolved.json --dry-run --feegrant --granter ops
./hm-support.sh accounts verify --file whitelist/fixtures/members.json --dry-run
```

## Proof in-tree

- Module: `cosmossdk.io/x/feegrant` (app wire)
- Suite: `tests/interchaintest/chain_start_test.go` → `testFeeGrant`
