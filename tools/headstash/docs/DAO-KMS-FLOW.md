# Generic DAO flow in `mainnet_deploy` (cw-orch-kms)

All DAO actions go through `scripts/dao-scripts/src/bin/mainnet_deploy.rs` with `--signer kms`.
The member's cold key (the granter, signed with Zigner) grants a short-lived session key that sends
`MsgExec` with the member as the inner sender. Fees are paid in `uthiol` from a feegrant. The session
revokes itself when the phase ends, unless you pass `--retain`. We never use the terpd CLI here.

## Phases

| phase | does |
|---|---|
| `dao-propose` | submits one proposal; can also vote and execute it |
| `dao-vote` | votes on `--proposals` (default `yes`) |
| `vote` | legacy alias of `dao-vote`; target defaults to Foundation pre-propose A |
| `dao-execute` | executes `--proposals` that have `passed`; votes only if `--vote` is given |
| `daos` | legacy FIAB + steward Foundation proposals (unchanged, hardcoded) |

## Target resolution (all DAO phases, from chain)

The target is picked in this order:
1. `--proposal-module M`: core comes from `M {dao:{}}`. Pre-propose is `--pre-propose`, else `M {proposal_creation_policy:{}}`.
2. `--dao CORE`: the first module that is `enabled` in `CORE {proposal_modules:{}}`. Pre-propose comes from the policy, as above.
3. `--pre-propose P`: module comes from `P {proposal_module:{}}` and core from `P {dao:{}}`.
4. Default: `--foundation-pre-propose` (Foundation pre-propose A).

If the creation policy is `anyone`, the proposal goes straight to the module as
`{propose:{title,description,msgs,proposer:null}}`. Otherwise it goes to the pre-propose as `{propose:{msg:{propose:{...}}}}`.

**Deposits:** the phase reads the pre-propose `config.deposit_info`. If a deposit is required, it refuses
before any grant, because the scoped grant carries no funds. Deposit support is still TODO.

## dao-propose flags

- **Input (use exactly one):**
  - `--propose-file F --propose-sha HEX`: a rendered pre-propose body. The sha is required.
  - `--msgs-file F --title T [--description D] [--propose-sha HEX]`: a JSON array of CosmosMsg.
- `--vote yes|no|abstain`: votes after submitting. Leave it out to not vote.
- `--execute [--wait-pass --wait-secs N]`: executes once the status is `passed`. Without `--wait-pass` the proposal must already be passed. With it, the phase polls every 3 s, for at most N seconds (default 300).
- `--retain`, `--ttl-secs`, `--account-index`, `--granter`: session controls.

## Checks before any grant

- A sha mismatch refuses, with "refused before any grant".
- **instantiate2:** each instantiate2 address is re-predicted from the on-chain code checksum, with the DAO core as creator and the msg salt. Every `wasm.execute.contract_addr` and `bank.send.to_address` in the proposal must be one of these addresses; otherwise the phase refuses. If the proposal has no instantiate2, this check is skipped and the skip is printed.
- On a `--execute` run, `vote`/`dao-vote`/`dao-execute` refuse a granter that has already voted on a proposal that is still open. That run has to come from a second member.

## Authz scope

The scope covers only the chosen contracts and methods: `ScopedCall{address, keys, calls+1}` for each contract.
- `dao-propose`: `propose` on the pre-propose (or on the module, for direct submit), plus `vote` and/or `execute` on the module when you ask for them.
- `dao-vote`: `vote` on the module, `calls = |proposals|`.
- `dao-execute`: `execute` on the module, plus `vote` only with `--vote`.

There are also a feegrant spend limit from `planned_spend_limit(steps+1)` and revoke messages.

## State

Proposal ids are stored in `--state` under `dao-propose:<sha16>`. This means each different file is a separate proposal, and re-running the same file is a no-op.

## Failure modes

| case | result |
|---|---|
| sha mismatch, missing title/msgs, bad `--vote` | refused before the granter challenge or grant |
| unresolvable dao/module | error before any grant |
| deposit required | refused before any grant |
| instantiate2 target mismatch | refused before any grant |
| submit or vote tx fails | phase errors; the session still revokes (Drop) unless `--retain` |
| `--execute` while still open after the wait | errors with the status; no execute is sent; run `dao-execute` later |
| execute signer already voted (open) | refused before any grant |
| macOS Local Network Privacy | run from the host's own Terminal |

## Local test signer

`--signer local-test` only works against `127.0.0.1`. The local key stands in for the member, so it is the inner
sender of the same `MsgExecuteContract`, without the authz wrap. The Zigner approval is not exercised locally.
e2e: `KMS_DEPLOY_BIN=... HARNESS=dao STOP_AFTER_PROP1=1 tools/headstash/e2e/run-local.sh`.

## Not implemented

- `circuit-propose` (x/gov MsgSubmitProposal wrapping the circuit store). It needs a generic-authz grant for
  `/cosmos.gov.v1.MsgSubmitProposal` with a deposit and proto encoding of the inner circuit msg, which is a bigger change.
- Rewriting `daos` on top of the generic path.
