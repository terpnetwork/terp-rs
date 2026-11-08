# Mainnet go-live checklist (morocco-1 → osmosis-1 / penumbra-1 / juno-1 / atomone-1)

> **Nothing here has been run.** Each step marked **[APPROVAL]** needs explicit
> owner approval at the time it is run. Scripts default to dry-run. A broadcast
> needs the per-chain broadcast env flag *and* a cw-orch-kms session that the
> owner signs on the Zigner phone. Every session revokes its authz + feegrant
> on close, on panic and on early return. `kms_gate::kms_broadcast` then checks
> on-chain that 0 grants remain.

## 0. Pre-flight (local, read-only)
- [ ] `cargo test -p cw-orch-kms --lib` (59 pass) and `cargo test -p dao-scripts` (kms_gate 13 pass).
- [ ] Local self-relay e2e:
  - `cargo run -p cw-orch-hermes-relayer --example kms_self_relay`
  - Chains from `/tmp/localnet2.sh`.
  - Must print `ALL OK`.
- [ ] `deploy_preflight` dry-run against each target chain. Check that the chain-id matches the node.

## 1. Secrets hygiene — **[APPROVAL]**
- [ ] **Rotate the Penumbra relayer spend key.**
  - It is stored in plaintext in `~/.ibc-hermes/config.toml` (line 41).
  - Treat it as exposed: generate a new key, move the funds, and keep the new key out of plaintext config (keyring/secret file, 0600).
  - Restart only the rotko Penumbra hermes, during a window the owner chooses.
  - *Listed only; not done by the agent.*
- [ ] Confirm no mnemonic/.env is committed: `git status` and `git log -p` for secret names.

## 2. Node — **[APPROVAL]**
- [ ] Run a synced morocco-1 full node: `catching_up=false` and the latest height matches public RPCs.
- [ ] Expose gRPC/RPC to the relayer host only.

## 3. Relayers — **[APPROVAL]**
- [ ] Penumbra: **rotko hermes fork** (stock hermes cannot relay penumbra-1).
- [ ] osmosis-1 / juno-1 / atomone-1: **stock hermes 1.13**.
- [ ] Packet filters:
  - Add **juno-1**.
  - Add the polytone wasm ports (`wasm.<note>`, `wasm.<voice>` per chain) next to `transfer`.
- [ ] Fund relayer keys on every chain. Check `hermes health-check`.
- [ ] Without a running relayer, cw-orch scripts can relay their own packets: `HermesRelayer::wait_ibc_from_txhash`, the in-process ibc-relayer. These self-relay txs are signed by the relayer key (`TEST_MNEMONIC`/`MAIN_MNEMONIC`), **not** by the kms session.

## 4. New IBC clients/connections/channels — **[APPROVAL] per chain**
- [ ] Dry-run `ibc_create_clients` for osmosis-1, penumbra-1, juno-1 and atomone-1. Review the plan: trusting period, max clock drift, trust level.
- [ ] Open `relay_grants` sessions: scoped MsgCreateClient/UpdateClient/ConnOpen*/ChanOpen*, expiring, revoked on close.
- [ ] Broadcast one chain at a time. Record client/connection/channel ids in `ibc_info`.
- [ ] Verify `query_grantee_state` = 0 grants after each chain.

## 5. Contract deploys — **[APPROVAL] per tx**
- [ ] mainnet_deploy: 4% fees, x/gov steward parent, FIAB via Foundation proposal (then Foundation members vote/execute), oracle alt-assets JSON reviewed, `--grpc http://192.168.10.101:9090`.
The phased runtime and its approval gates are in [mainnet-deploy-runbook.md](mainnet-deploy-runbook.md) (`mainnet_deploy all`, dry-run first).
1. [ ] Upload + instantiate2: **cw721-svg**, then the **cw721-svg-minter** series.
2. [ ] **Benders** collection.
3. [ ] **feeshare** registration for the new contracts.
4. [ ] NFT pricing: **`svg_markets`**. Dry-run first, then broadcast.
- [ ] After each step: check code ids/addresses against the plan file; confirm 0 grants remain.

## 6. Post-go-live
- [ ] Monitor relayer logs for stuck packets. Use script self-relay to clear specific packets (`wait_ibc_from_txhash`).
- [ ] Confirm no retained authz grants: `svg_ops grants --granter <granter> --operator <wallet>` for each operator wallet.
- [ ] Confirm no session-key grants are left (each operator-exec prints `session closed: session grants revoked`).
