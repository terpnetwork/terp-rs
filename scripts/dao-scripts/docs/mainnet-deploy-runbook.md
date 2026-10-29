# Mainnet deploy runbook (`mainnet_deploy`)

Phased cw-orch deploy for **morocco-1**. Every signature goes through `cw-orch-kms` (`kms_gate`): one scoped,
expiring authz session per phase, approved by the cold key on the granter page (a random loopback port in 18700-18999, printed with its `ssh -L` line; `CW_ORCH_KMS_PORT` pins it), and revoked when the phase ends.
The runtime is **dry-run by default**. It writes `out/mainnet-deploy/plan-<phase>.json` and signs nothing.
It is **idempotent**: `out/mainnet-deploy/state.json` records code ids with checksums, contracts, done steps and txs.
Before reusing a stored code id, it compares the on-chain checksum and stops on a mismatch.
Contracts use `instantiate2` with salt `mainnet-deploy/<name>`, and the address is checked against the prediction.
There are **no migrate entrypoints**, and it never calls the cw-infuser `deploy`/`upload`/`permissionless`/`create_infusion_minter`/`benders`, `polytone_migrate` or `dels` bins.

```
cargo run -p dao-scripts --bin mainnet_deploy -- <phase|all> [--from P] [--only P] [--admin <addr>] [--broadcast]
# --admin defaults to the Foundation DAO terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w
# broadcasting also needs TERP_SCRIPT_BROADCAST=1 (and TERP_SCRIPT_BROADCAST_CHAIN=<id> off morocco-1)
```

| # | phase | does | approval |
|---|-------|------|----------|
| 0 | preflight | artifacts vs `checksums.txt`, node chain-id, broadcast gate, approval page port (random, re-picked if taken) | none (read-only) |
| 1 | upload | store 12 codes: oracle-registry/aggregator/twap-probe, marketplace, reserve-auction, svg-auction, raffle, nft-loan, passkey, recovery, cw721-svg, svg-minter | **[APPROVAL]** one MsgStoreCode session |
| 2 | daos | writes `daos_fiab_proposal.json` (FIAB via the Foundation pre-propose) and points to `steward_multi_dao` | **[APPROVAL]** Foundation proposal + vote, sent with `fiab_subdao`/`steward_multi_dao --broadcast` by a Foundation member |
| 3 | oracles | instantiate2 registry, then aggregator(registry), then twap-probe(mid) | **[APPROVAL]** instantiate2 session |
| 4 | nftfi | marketplace, reserve-auction, svg-auction, raffle, nft-loan (admin = `--admin`) | **[APPROVAL]** instantiate2 session |
| 5 | authenticators | fresh passkey/recovery codes (the old 84/85 are kept as `previous_codes`); `--smoke` = throwaway recovery + MsgAddAuthenticator (rehearsal only) | none on mainnet (per-account registration) |
| 6 | svg | factory instantiated with **owner = the kms granter** (or `--svg-owner`), 2× set_series (operators = [owner]), 3 drops; all owner calls are MsgExec as the granter. Only with `--svg-owner` ≠ granter: transfer_ownership → that address + `svg_accept_ownership.json` | **[APPROVAL]** session (+ the new owner accepts, if `--svg-owner`) |
| 7 | verify | `deploy_report.json`: every contract admin == `--admin`, factory owner (or pending owner) == `--svg-owner`/granter, both series' operators == [that owner], txs, gas. Fails on a mismatch | none |
| – | polytone | (not in `all`) checks the v3 codes 78/79/80; no gov proposal | none |

After every approved phase, check for leftover grants with cw-orch: `svg_ops grants --granter <granter> --operator <wallet>` (prints the authz + feegrant state; `0 authz, feegrant none` is clean). The session keys' own grants are revoked by the run itself.

## Rehearsal (local, 2026-10-26)
A throwaway `terpd v6.4.0` (chain `deploy-rehearsal-1`) ran under `/tmp`, bound only to 127.0.0.1, with the test keyring. It used `--signer local-test`: an in-process ephemeral key, which the runtime refuses for morocco-1/120u-1 and for any non-localhost gRPC.
- `all`: 21 txs, 37.9M gas. upload 11 tx / 36.18M, oracles 3 / 0.50M, nftfi 5 / 0.93M, authenticators (smoke) 2 / 0.31M. verify had 0 failures.
- `all` rerun: **0 txs** (idempotent).
- `cw_svg_minter.wasm` does not compile on the arm64 (macOS) wasmvm (`ImpossibleRelocation`). The rehearsal therefore used `--skip-code svg-minter` (local-test only), so the svg phase was **not rehearsed**. Re-rehearse on an x86_64 node before mainnet.

## Decisions applied (2026-10-26)
- FIAB: wasm + DAO admin = Foundation DAO; created via a Foundation proposal (ExecAs propose + Vote through the kms session, `--foundation-pre-propose`, `--no-vote` to skip voting). Execution is by Foundation governance.
- Stewards: parent/admin = x/gov module account (queried; `--gov-parent` override); guard rejects anything else / the Foundation.
- Passkey + recovery: upload only, code ids in state.json; `--smoke` local-test only.
- Fees: 4% (400 bps / 0.04) raffle, loan, marketplace, reserve-auction, svg-auction.
- Revenue auction: not deployed. Oracles run before nftfi/svg; `oracle_pricing` passed to raffle + svg-minter (`--oracle-alt-assets <json>`, `--oracle-enforce`; default enforce=false, no alt assets).
- Under `--signer kms` code is stored as raw wasm: terpd ante lacks wasm GasRegisterDecorator, so StoreCodeAuthorization rejects gzip in MsgExec.
- gRPC: networks.toml overrides morocco-1 to 127.0.0.1:9090; scripts now use load_network(false), pass `--grpc`.

## Roles (2026-10-26)
| role | holder |
|------|--------|
| wasm admin (every contract, incl. svg-minter and drops) | `--admin` (Foundation DAO) |
| raffle / nft-loan `owner`, fee + treasury addrs, oracle-registry admin | `--admin` |
| svg-minter factory owner (cw-ownable) | **kms granter** (the Zigner phone address), or `--svg-owner <addr>` |
| series operators (marain-orbs, latent-benders) | [factory owner] |
| contract / code `creator` | the ephemeral session key. Not a role: only feeds the instantiate2 address derivation; grants nothing |

The ephemeral session key never holds a privileged role: it is gone when the process exits. Owner-gated calls are
MsgExec with the granter as inner sender, so owning from the granter lets you delegate (authz) and rotate wallets.
All MsgStoreCode messages set `instantiate_permission = Everybody` explicitly (also `itl_consume`). The session
CodeGrant has no permission constraint, and x/wasm accepts any permission in that case.

## Day-2 ops: `svg_ops` (cw-orch-kms)
Every step is a cw-orch command. No `terpd tx` / `terpd q` step is used.
```
cargo run -p dao-scripts --bin svg_ops -- <exec|grant-operator|operator-exec|revoke-operator|grants> --contract <factory> [--msg JSON|--msg-file F] [--broadcast]
# same gate as mainnet_deploy: --chain-id, TERP_SCRIPT_BROADCAST=1 (+ TERP_SCRIPT_BROADCAST_CHAIN off morocco-1); --granter or the approval-page challenge
```
- **Granter acts directly** (create a drop, set_series, update_ownership, …): `svg_ops exec --contract <factory> --msg-file create.json`.
  The phone approves one session scoped to that contract + top-level key. The tx is MsgExec{MsgExecuteContract{sender: granter}}, and the grants are revoked afterwards.
- **Grant an operator wallet** (granter side; the granter's phone signs):
  `svg_ops grant-operator --contract <factory> --operator <operator addr> --keys create_series_collection,create_svg_collection --calls 10 --ttl-secs 86400`.
  The phone signs a **retained** MsgGrant (ContractExecutionAuthorization: factory only, AcceptedMessageKeysFilter, MaxCallsLimit = calls+1) and a feegrant to that address. Nothing is executed.
- **Operator acts** (operator side, its own cold key):
  `svg_ops operator-exec --granter <granter> --operator <operator addr> --contract <factory> --msg-file drop.json [--bind 127.0.0.1:<port>] --broadcast`.
  It queries and prints the grant first (allowed keys, calls left, expiry, fee allowance). A missing, revoked, expired or used-up grant, or a message key the grant doesn't allow, is refused before anything is signed.
  The operator's Zigner never signs the contract msgs. It signs one short session grant (≤ 900 s) to an ephemeral key on the box: generic MsgExec authz + a feegrant capped at the planned spend. The ephemeral key then sends MsgExec{MsgExec{MsgExecuteContract{sender: granter}}} for each `--msg-file` (repeatable; up to the calls left). The session's grants are revoked when it closes (`session closed: session grants revoked`). `--signer kms` is the default and needs no mnemonic.
  Fallback only: `--signer env` signs with `MAIN_MNEMONIC` exported in the operator's own shell (never the repo `.env`).
  Large requests don't fit one QR code, so the approval page animates Zigner raptorq frames (1000-byte frames, about 4 fps). A 15 KB doc becomes 31 frames. Phone-side routing of batched Cosmos payloads needs the Zigner change in the turn-5 spec. Until that ships, use `--signer env` for msgs that don't fit.
- **Revoke an operator**: `svg_ops revoke-operator --contract <factory> --operator <operator>` sends MsgExec{MsgRevoke(MsgExecuteContract), MsgRevokeAllowance} as the granter.
  Check afterwards with `svg_ops grants --granter <granter> --operator <operator>` (expects `0 authz, feegrant none`).
- Making an operator a **series operator** is separate (contract-level, not authz): `svg_ops exec --msg '{"set_series":{"series":"…","config":{"operators":["<granter>","<op>"], …}}}'`.
  Operators still run on the series cadence, and owner calls still need the granter.

## Rotating ownership / operators
- **Owner rotation** (cw-ownable): `svg_ops exec --msg '{"update_ownership":{"transfer_ownership":{"new_owner":"<new>","expiry":null}}}'`.
  Then the new owner accepts with `{"update_ownership":"accept_ownership"}`, signed by the new wallet, or by the new granter through `svg_ops exec --granter <new>`.
  Until the accept lands, `ownership{}` shows `pending_owner`. Re-run `verify` with `--svg-owner <new>`.
- **At deploy time**, `--svg-owner <addr>` adds a final `svg:offer-ownership` step and writes `svg_accept_ownership.json` for that address to execute.
  With the default (owner = granter), no offer is made and any stale file is removed.
- **Operator rotation**: `set_series` with the full new `operators` list. It replaces the list, so include the owner if it should stay. Revoke the old wallet's authz grant with `revoke-operator`.
- `renounce_ownership` is irreversible. Don't use it.

## Rehearsal: granter-owned factory (local, 2026-10-26)
Fresh `deploy-rehearsal-1`, `--signer kms`, with a mock Zigner phone as the granter:
- `all`: upload 12 tx / 85.15M, daos 4 / 3.44M, oracles 3 / 0.54M, nftfi 5 / 1.00M, svg 6 / 3.42M (no offer step).
  verify: 12 contracts, 0 failures. Ownership owner = granter, pending none. Both series' operators = [granter].
- `svg_ops exec` (the granter creates a drop): 1 tx, 1.70M gas, code 0. The session grants are revoked.
- `svg_ops grant-operator --calls 3` + drop: a second wallet got a retained CEA (factory, 2 keys, remaining 3 after 1 use) + a feegrant, and created a drop via MsgExec (1.70M gas).
- `svg_ops revoke-operator`: 97.7k gas. Afterwards there are 0 authz and 0 feegrants by the granter.
- `all` rerun: **0 txs**, verify 0 failures, 0 grants left.

## Rehearsal: operator-exec through cw-orch (local, 2026-10-26)
Fresh `deploy-rehearsal-1` + `all`; two mock Zigner phones (granter on 8741, operator on 8742). No terpd CLI was used; the node binary only ran the chain.
- `grant-operator --operator <op> --keys set_series --calls 2`: one cold-signed retained grant (on-chain MaxCalls 3).
- `operator-exec` ×2 (`set_series`, kms signer, fee from the granter): 174k / 160k gas, code 0. Calls left 2 → 1 → 0. Fee allowance 15,000,000 → 14,990,010 uthiol after the first.
- A disallowed key (`create_svg_collection`): refused before signing. A third call: refused, "no calls left". The operator phone signed exactly 2 docs.
- `revoke-operator`: 97.6k gas. Then `operator-exec` was refused ("no MsgExecuteContract grant"), and `grants` showed `0 authz, feegrant none`.

## Turn-5 rehearsal (session-path operator-exec, 127.0.0.1 throwaway chain)
| step | result |
|---|---|
| grant-operator (forced 300-byte frames) | phone decoded 7 batched frames into a 1044-byte request; grant 3 calls + feegrant |
| operator-exec run 1 (1 msg) | session key → nested MsgExec code 0, collection created, session closed, 2 calls left |
| operator-exec run 2 (2 msgs, one session) | 2 txs code 0, 2 collections, session closed, 0 calls left |
| disallowed update_config | refused before signing |
| session-key grants after close | 0 authz, no feegrant (both keys) |
| revoke-operator | code 0; grants 0 authz, feegrant none |
| docs signed | granter 2 (grant, revoke), operator 4 (2 per session) |
