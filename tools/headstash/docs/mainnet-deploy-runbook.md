# Headstash 2026: mainnet deploy runbook (morocco-1)

Status: draft. Nothing here has been broadcast. Messages live in `tools/headstash/deploy/`.
Render them with `python3 tools/headstash/deploy/render.py decisions.json --circuit <dir with params.bin, vk_body.bin>`
(`--partial` before the contract exists; `--dry-run` for an EXAMPLE sample in `deploy/rendered-example/`). It reads
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
   the eligibility root, depth, ctx, vk_id 3, the note vk_hash, amount classes, cutoff and anchor cadence.
   Verify with `{"epoch_campaign":{}}`.
5. **Hand ownership** (deployer, then new owner): `03-update-ownership-transfer.json`, then the
   owner executes `03b-update-ownership-accept.json`. Verify `{"ownership":{}}`.
6. **Governance proposal**: `rendered/submit-proposal.json`
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
Runs on groot-wan, 2026-10-26 (local times UTC-4).

- **cw-headstash lib tests**: 86 passed, 0 failed (`cargo test --release --features testwavs --lib`,
  standalone copy with commit 4a6b4ca sources), including `update_ownership_two_step_and_no_renounce`.
  The `tests/` integration suite needs `cw-headstash-manifold` and was not run.
- **Dev wasm** (nightly 1.98 + `wasm-opt -Os`, NOT the publishable artifact):
  sha256 `ba451cded17b3b74d0416453f0e472e7abed5dc7b540599d27625dd9bb291019`, 972,253 bytes.
  Publish only the cosmwasm/optimizer (Rust 1.91) build and record its sha in `{{WASM_SHA256}}`.
- **Wasm prover -> native verifier parity: PASS.** Deterministic witness (ctx 0x7e49, amount
  1,500,000, index 5). wasm `membership_key_com_hex` = native key_com `ca79849d...4bd39a07`;
  wasm proof (k=11, vk_id 2, 2,752 bytes, 4.4 s in node) verified natively in 291 ms; body root
  `0cac7380...9c811a1b` and nullifier `1289ce73...282ef312` equal the native values; public
  input order ctx, root, nullifier, dest_lo, dest_hi, amount; tampered proof rejected.
  vk_hash `94d43261...1eefed11` matches the contract config.
- **warm_verify_stays_under_one_second: FAIL under load** (epoch-claim circuit, 4.041 s warm
  verify, load average ~30 during the run; key build 78.8 s). Rerun on an idle host before
  relying on it; the membership verify path above took 0.29 s.

## Direct on-chain claim (vk_id 3, zk-api build) — supersedes lane steps 11–12
The direct path verifies the note proof in the contract (`proof_instance_verify`, pinned zk_id)
and pays immediately. hashmerchant `MsgRegisterChain`, lane staging and the sudo escrow
(steps 7, 11, 12) are **only needed for the optional lane path**.

Artifact: `tools/headstash/artifacts/cw_headstash.wasm` (`--features zk-api`, optimizer),
sha256 `3010d1baf4623a665f0631cb099af973f3de97b2fdb62a5e798f582d4e0a597b`, 1,096,689 B.

1. **Store code** (anyone; `code_upload_access` Everybody). Check the code-info checksum = sha above.
2. **Instantiate** (deployer, admin = gov) and set the campaign exactly as steps 2–5 above
   (registration epochs and anchors in `registration.rs` are unchanged).
3. **Governance proposal** bundles, in this order:
   a. `/cosmwasm.wasm.v1.MsgStoreFullCircuit` with `sender` = gov module
      `terp10d07y265gmmuvt4z0w9aw880jnsr700jag6fuq`, `auth {circuit_type 0, curve_type 0, k 11}`,
      `params_body`, `vk_body` (blob sha256 must equal the campaign `vk_hash`). Mainnet
      `circuit_upload_access` is Nobody, so this has to come from gov. Verified against terpd v6.4.0
      (84a11395, = mainnet): `StoreFullCircuit` -> `selectAuthorizationPolicy(sender)` returns
      `GovAuthorizationPolicy` for the gov authority, whose `CanCreateCode` is `true`, so
      `authorizeCircuitUpload` skips the Nobody check and the deposit. Local v6.4.0 node with
      `circuit_upload_access` Nobody: the gov proposal passed and emitted `store_full_circuit`
      (param_id, zk_id); the same msg from a user account failed `circuit deposit required`.
      Note: zk_id is chain-assigned (1 on a fresh chain), so take it from the event.
   b. `MsgCommunityPoolSpend` funding the contract.
   Bundling is safe: the zk_id is only known after execution, and the contract pays nothing until
   the owner binds it in step 4, so funds sit idle until then.
4. **Bind** (owner): `rendered/06-bind-note-circuit.json` (set `NOTE_ZK_ID` from the proposal's store event). The contract
   checks sha256(circuit data) == vk_hash and refuses a rebind ("circuit is fixed").
5. **Relay** with fee grants: `HEADSTASH_FEEGRANT=1`, `HEADSTASH_GAS_PRICE=0.05uthiol`,
   `HEADSTASH_CLAIM_GAS` (400000 default), bind 127.0.0.1:18795 (not 8791) behind the single nginx,
   no IP logging. The granter key holds the THIOL budget below.
6. **Claims**: the page waits until the note's registration is in an accepted anchor plus 0–2
   random extra anchor intervals, creates a fresh wallet per note, requests a grant (one tx,
   short expiry, MsgExecuteContract to this contract only), and broadcasts to the user-selected RPC.
7. **Cutoff and sweep** as step 13.

### Fee-grant budget (direct claim)
Read-only mainnet query (2026-10-27 01:08 UTC = 01:08 Africa/Dakar, height 24,110,663,
terp.api.m.anode.team): globalfee `minimum_gas_prices` = **0.05 uthiol** (only denom listed);
recent mainnet txs pay fees in uthiol. **Fees are THIOL, not TERP.** The relay granter must hold
uthiol and run with `HEADSTASH_GAS_PRICE=0.05uthiol` (the allowance denom follows it). The page
takes the fee denom and amount from the grant's `spend_limit`.

Inputs: 98,841 notes, 17,791 identities. Measured on the local zk-api build:
registration ≈ 133,164 gas per message + 314,545 gas per note (32 notes 10,198,618; 11 notes
3,593,164; 2 Solana notes 785,218) → ≈ 338.5k gas per note averaged over one message per identity.
Claim gas used 262,462–262,602, but fees are charged on the 400,000 limit. Grant tx ≈ 123,440 gas.

| Item | Gas | THIOL @ 0.05 |
|---|---:|---:|
| notes × (registration + claim limit) = 98,841 × (338.5k + 400k) | 73.0 B | **3,649.8** |
| + one feegrant tx per note (relay pays) | 12.2 B | 610.0 |
| = base budget | 85.2 B | **4,259.8** |
| with 1.3× registration gas adjustment | 95.2 B | 4,761.7 |

Recommendation: fund the granter with ~4,800 THIOL (top up in tranches; unspent allowances
expire). Lowering `HEADSTASH_CLAIM_GAS` to 300,000 (claims use ~262.6k) saves ~494 THIOL.

### Direct-path e2e evidence
`RESET=1 tools/headstash/e2e/run-local.sh` (`run_direct.py`) on groot-wan, 2026-10-26 21:08 local (UTC-4):
**57 passed, 0 failed**, report `/tmp/headstash-mvk-chain/run-1793063360/report.json` (log `e2e6.log` beside it).
Every negative in the security review addendum D-tests passed. Gas: store_circuit 10,242,506; bind 570,551;
claim 262,528–262,652 (mean 262,561); accept_anchor 135,937 / 138,501; grant tx fee 3,086 uterp at 0.025.

### Production fee-grant model: operator-funded direct mode (decision 2026-10-27)
The relay runs in **direct mode**; there is no authz mode. Each participating sidecar operator is the
fee granter from its **own** account: `HEADSTASH_FEEGRANT=1`, `HEADSTASH_GAS_PRICE=0.05uthiol`, relayer key in
the OS keyring, bind 127.0.0.1:18795 behind the single nginx. The operator account holds THIOL.
Funding: the foundation DAO (or the community pool by gov, `{{FEEGRANT_FUNDING_SOURCE}}`) bank-sends each
named operator `{{OPERATOR_n_ADDR}}` its share `{{OPERATOR_n_BUDGET_UTHIOL}}` of the ~4,800 THIOL budget above.
Loss is bounded by what was sent: an operator can only grant from its own balance. Top up in tranches
based on grants used (relay `/register/status`, on-chain `feegrant` events with granter = operator).
Template: one `MsgCommunityPoolSpend` (uthiol) per operator; repeat for each operator. If the foundation DAO
funds instead, drop them and send `BankMsg::Send` uthiol from the DAO.
Read-only check 2026-10-27 01:08 UTC (01:08 Africa/Dakar): the community pool holds 572,865,724 THIOL.
