# Headstash 2026: mainnet deploy runbook (morocco-1)

Status: draft. Nothing here has been broadcast. Messages live in `tools/headstash/deploy/`.
Render them with `python3 tools/headstash/deploy/render.py decisions.json`. It reads
`tools/headstash/target/eligibility/` and fills `{{PLACEHOLDERS}}`. The eligibility root is
not hardcoded anywhere; it is read from the build at render time.

## Facts read from mainnet (2026-10-26 ~17:44 UTC, height 24,104,724, terp.api.m.anode.team)
- App version 6.4.0. `code_upload_access` Everybody, `instantiate_default_permission` Everybody:
  **no store-code proposal is needed.** `circuit_upload_access` Nobody (dest TerpNET DAO
  `terp14w2qva6...3kw2w`); not needed, the membership vk lives in the sidecar registry
  (vk id 2) and the contract stores only `vk_id` + `vk_hash`.
- Community pool 267,193,374.78 TERP. uterp supply 419,893,354.69 TERP.
- Old #32 contract `terp1qeyjez...40ehd` (code 8, admin gov): balance 0. Nothing to recover.
- hashmerchant: `escrow_denom` uterp, `min_escrow_amount` 1,000,000, `quorum_fraction` 0.667,
  `contract_gas_limit` 100,663,296, **no registered chains**.
- Gov: min deposit 10,000 TERP (50% initial), deposit 3 d, voting 4 d, quorum 33.4%,
  threshold 50%, veto 33.4%.

## Roles (who signs)
| Role | Signs | Recommended |
|---|---|---|
| Deployer | store, instantiate, initial `set_epoch_campaign`, ownership transfer | ops key, then hands off |
| Contract owner | `set_epoch_campaign` (final anchor), `stage_epoch_batch`, `sweep_unclaimed` | TerpNET DAO or ops multisig (decision) |
| Wasm admin | migrate | gov module `terp10d07y265gmmuvt4z0w9aw880jnsr700jag6fuq` |
| Governance | `MsgRegisterChain` + `MsgCommunityPoolSpend` | proposal |
| Relayer key | registration batches via relay | hot key, gas only |
| Anyone | `accept_anchor` | permissionless |
| Lane custody key | signs batches the oracle attests | sidecar operator, ed25519 |
| Validators | vote extensions for the accepted root | 2/3 power running the sidecar |

## Pre-flight (all must pass)
1. Eligibility rebuild final; `render.py` report shows root, identities, leaf total. Leaf total
   is slightly under 67M TERP; the excluded share stays unallocated in the pool.
2. Wasm built with the CosmWasm optimizer (Rust 1.91); sha256 recorded in
   `tools/headstash/artifacts/checksums.txt`; `cosmwasm-check` passes; matches the e2e code.
3. Prover sources committed (security review M4); `membership_vk_hash` prints `94d43261...ed11`.
4. Full e2e (`tools/headstash/e2e/run.py`) green on the final wasm and the mainnet binary version.
5. Decide whether mainnet needs the `b2339a9c` VE custody fix (upgrade) before payout.
6. Validator sidecar readiness: >= 2/3 voting power confirms `[hashmerchant] sidecar-url`.
7. Path delivery solved (security review H2).
8. Decisions file complete; `render.py` exits 0.

## Steps
1. **Store code** (deployer): `terpd tx wasm store cw_headstash.wasm --from deployer --gas auto
   --gas-adjustment 1.3 --gas-prices 0.05uterp --chain-id morocco-1`. Check `code-info` checksum.
2. **Instantiate** (deployer), admin gov:
   `terpd tx wasm instantiate <code> "$(cat rendered/01-instantiate.json)" --label headstash-2026-1
   --admin terp10d07y265gmmuvt4z0w9aw880jnsr700jag6fuq --from deployer`.
   Optional: `instantiate2` with a fixed salt to know the address in advance.
3. Put the address in `decisions.json` as `CONTRACT_ADDR`, rerun `render.py`. This derives `ctx`.
4. **Open campaign** (deployer): execute `rendered/02-set-epoch-campaign-open.json`. It carries
   the eligibility root, depth, ctx, vk_id 2, vk_hash, amount classes, cutoff and anchor cadence.
   Verify with `{"epoch_campaign":{}}`.
5. **Hand ownership** (deployer, then new owner): `03-update-ownership-transfer.json`, then the
   owner executes `03b-update-ownership-accept.json`. Verify `{"ownership":{}}`.
6. **Governance proposal**: `rendered/proposal-headstash-fund-and-oracle.json`
   (`terpd tx gov submit-proposal <file> --from proposer`). Registers the `headstash-claim`
   oracle source and sends the funding amount to the contract. 3 d deposit + 4 d vote.
7. **Escrow for sudo gas** (any funded account): `terpd tx hashmerchant register-contract
   <contract> headstash-claim bank 1000000uterp`. Top up with `refill-escrow`.
   The airdrop escrow is the contract's own uterp balance from step 6.
8. **Relay** (operator): run `headstash-register-relay` on 127.0.0.1:<port other than 8791>
   behind the single nginx (`proxy_set_header X-Real-IP $remote_addr`), env
   `HEADSTASH_CONTRACT`, `HEADSTASH_CHAIN_ID=morocco-1`, `HEADSTASH_ELIGIBILITY=tree.json`,
   `HEADSTASH_FLUSH_SECS=6`, `HEADSTASH_MAX_BATCH=25`, relayer key in the OS keyring.
9. **Registration window**: anyone sends `04-accept-anchor.json` each `ANCHOR_INTERVAL` from
   `ANCHOR_START_HEIGHT`. Close when the registration count passes `MIN_ANONYMITY_SET` and the
   announced date arrives; take the last anchor.
10. **Freeze allocation** (owner): `05-set-epoch-campaign-final-anchor.json` with
    `FINAL_ANCHOR_B64`. Lane: post the same campaign to `/ve/headstash/campaign`.
11. **Claims**: users prove in the browser (vk id 2, K=11) and submit to the lane.
12. **Seal cadence**: owner stages each lane batch (`08-stage-epoch-batch.example.json`, max_rows
    rows); lane seals at each epoch height; validators attest `headstash-accepted`; sudo pays.
    Suggested cadence daily until cutoff.
13. **Cutoff and sweep** (owner): after `CUTOFF_HEIGHT`, after the last staged batch is paid,
    `09-sweep-unclaimed.json` sends leftovers to `SWEEP_RECIPIENT`.

## Abort / rollback
- Before step 6 passes: nothing is funded; abandon the contract.
- Proposal fails: no funds move. Fix and resubmit.
- After funding, before payouts: owner stops staging; gov can migrate (admin) to a code that
  returns funds, or wait for cutoff and sweep. Relay can be stopped at any time.
- Bad batch staged but not attested: lane withholds the seal; nothing pays.
- Oracle misbehaviour: gov disables the chain (`MsgRegisterChain` with `enabled:false`, or params).

## Monitoring
- Contract uterp balance vs sum of paid rows (`wasm` events `minted`).
- `epoch_campaign`, `registration_tree` (count, anchors) every block range.
- Relay `/register/status`, queue depth, failed counts; relayer gas balance.
- hashmerchant escrow (`terpd q hashmerchant escrow <contract>`), VE inclusion of
  `headstash-accepted` roots, lane health.
- Alert on any `set_epoch_campaign` or `update_ownership` event.

## User decisions
Contract owner; wasm admin (recommend gov); funding amount (leaf total vs 67M); `CAMPAIGN_LABEL`
(enters ctx); `CUTOFF_HEIGHT`; anchor start/interval; `MIN_ANONYMITY_SET`; sweep recipient;
WAVS operator set; lane custody key and public lane URL; upgrade to the VE fix; amount
privacy approach (review H3); path delivery (H2); proposal timing and proposer/deposit.

## Verification evidence
Filled in from this job's runs; see the final report.
