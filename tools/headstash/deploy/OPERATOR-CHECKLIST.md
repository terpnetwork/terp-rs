# Headstash 2026 mainnet operator checklist (morocco-1)

Everything here is UNSIGNED. Nothing has been signed or broadcast. The artifacts are in `deploy/rendered-mainnet/`, and every size is in `rendered-mainnet/sizes.json`.

Roles:
- **P**: the proposer, terp1gnfqzr25fgds7zn7m6njd795aat5yaf6vk7hd9 (dabarber validator). It is a cold key that signs on Zigner in amino-json.
- **E**: the ephemeral session key. It lives in memory on the operator device (cw-orch-kms `ScriptSession::open_with_key`) and signs in direct mode, never on Zigner.
- **FX**: the fee-grant wallet, terp1fxccvvhhy43tvet2ah7jqwq4cwl9k3dxmww052. It grants claim fees directly, with no authz.
- **GOV**: the gov module, terp10d07y265gmmuvt4z0w9aw880jnsr700jag6fuq. It is the wasm admin and the contract owner.

Wasm: `artifacts/cw_headstash.wasm`, sha256 43a8e5d88a3af34dc09bd63f85f88a1b2b0a008a4b68ef45a83ed434894149a9 (1,177,086 B, 327,630 B gzipped). The predicted Instantiate2 address is in `sizes.json` (`contract_addr_predicted`).

## Read from mainnet, read-only, on 2026-10-28 at height 24,144,476
- P: acct 0, seq 116. Spendable 10,980.26 TERP and 39,947.5 THIOL.
- FX: acct 19, seq 262. Spendable 2,202.74 THIOL and 8.97 TERP. It is a DelayedVestingAccount whose vesting ended in 2023, so all of it is spendable.
- Global-fee minimum gas price: 0.05 uthiol. Consensus block max_bytes: 16,777,216.
- The groot-wan node config has max_tx_bytes 4,194,304 and max_txs_bytes 1 GiB. The ~181 KB MsgExec and the ~547 KB store tx both fit. Confirm the same limits on whichever node you broadcast through.

## Before you start
1. Fill the placeholders that are still open, then re-run `render_gov_session.py` (it runs validate_rendered_proposal):
   - **WAVS_POO_JSON**, **ANCHOR_START_HEIGHT**, **PROPOSAL_METADATA_URI** and **FORUM_URL** are still unknown.
   - These are filled at runtime: EXEC_GAS/EXEC_FEE_UTHIOL, STORE_GAS/STORE_FEE_UTHIOL, CODE_ID, NOTE_ZK_ID, PROPOSAL_ID_CIRCUIT, CLAIMER_ADDR and CLAIM_FEE_UTHIOL.
2. Read the acct and seq of P and FX again right before each signature.
3. **Deposits: P can't fund both proposals at once.** Each proposal needs 10,000 TERP, which is 5,000 initial plus 5,000 top-up within the 72h deposit period. P holds 10,980 TERP, so run the proposals one after the other: the circuit proposal deposit is refunded when it passes, and only then do you submit the instantiate proposal. Another option is for a second depositor to add the top-up.
4. **FX THIOL top-up.** The claim fee-grant budget is ~2,891 THIOL and FX has 2,202.74, so send **at least 688.3 THIOL** to FX before step 16. Add about 10 THIOL for FX's own grant-tx fees. Note that `decisions.mainnet.json` still says `budget_uthiol` 4,761.7 THIOL. Reconcile that number with the 2,891 budget before you fill CLAIM_FEE_UTHIOL.

## Steps, in order

| # | File | Who signs | What it does |
|---|------|-----------|--------------|
| 0 | none | operator device | Generate E in memory. **Regenerate `10-session-grant/` with the real E address and P's current sequence** using `gov-session/target/release/gov-session-grant --chain-id morocco-1 --granter P --grantee E --account-number 0 --sequence <seq> --ttl-secs 86400 --allowance 300000000 --fee-denom uthiol --type /cosmos.gov.v1.MsgSubmitProposal --type /cosmos.authz.v1beta1.MsgRevoke --type /cosmos.feegrant.v1beta1.MsgRevokeAllowance --out rendered-mainnet/10-session-grant`. The committed doc uses a placeholder grantee and an expiry that has already passed. Then re-render 12, 13 and 15 with `--eph-grantee E`. |
| 10 | `10-session-grant/sign_doc.json` | **P on Zigner** | One amino tx with three GenericAuthorizations (SubmitProposal, Revoke, RevokeAllowance) plus a BasicAllowance of 300 THIOL to E, TTL 86,400s. Sign doc is **1,419 B**, 1 QR frame, within the 65,536 B limit. The session only needs ~2 THIOL of fees: 25M gas plus 10M gas at 0.05. |
| 12 | `12-circuit-exec.unsigned.json` (~181 KB) | E (direct) | MsgExec{MsgSubmitProposal{MsgStoreFullCircuit}}. Proposer is P, initial deposit is 5,000 TERP debited from P, and the fee comes from the feegrant. It used 22.2M gas locally, so set EXEC_GAS ≥ 25M (fee ~1.25 THIOL). Record PROPOSAL_ID_CIRCUIT. |
| 13 | `13-store-wasm.unsigned.json` (~547 KB) | E (direct) | MsgStoreCode with sender E and instantiate permission limited to GOV. Upload is open on mainnet, and the fee comes from the feegrant. It used 7.6M gas locally, so set STORE_GAS ≥ 10M. Record CODE_ID. |
| 17 | `17-deposit-topup.unsigned.json` | **P on Zigner** | Sign doc 354 B. MsgDeposit of +5,000 TERP on PROPOSAL_ID_CIRCUIT within 72h. Use 30M gas, because the large proposal ran out of gas at 8M locally. |
| 15 | `15-session-revoke.unsigned.json` | E (direct) | MsgExec that revokes each grant and the allowance, with MsgRevoke last. Run it as soon as 12 and 13 are done. Locally it used 109k gas. TTL expiry is only a backstop. |
| — | vote | validators | The circuit proposal passes. Record NOTE_ZK_ID from the events. |
| 14 | `14-instantiate-proposal.unsigned.json` | **P on Zigner** | Sign doc 3,355 B. Small gov proposal: CommunityPoolSpend to GOV, then Instantiate2 (CODE_ID, admin GOV, owner GOV), then set_epoch_campaign, then bind_note_circuit. Defaults: ANCHOR_INTERVAL 3430, MIN_ANONYMITY_SET 1, CUTOFF auto (~2y), BURN_UNCLAIMED on. Initial deposit 5,000 TERP, then a top-up to 10,000 using the step-17 shape with the new proposal id, signed by P on Zigner. |
| 16 | `16-feegrant-terp1fx.unsigned.json` | **FX on Zigner** | Sign doc 450 B. One direct BasicAllowance in uthiol from FX for each claimer. Do the THIOL top-up first. |

## Afterwards
- Check that the contract admin and owner are both GOV, the balance equals FUNDING, and the campaign root and bound zk_id are correct.
- Run `terpd q authz grants P E` and `terpd q feegrant grant P E`. Both must come back empty or not found.

## Verified locally (v6.4.0, test keys standing in for P/E/relay-granter)
`tools/headstash/e2e/run_direct.py` with GOV_ROLES=1 covered session grant, E's MsgExec proposal, proposer = granter, the 5 TERP initial deposit from the granter, the vote passing, the circuit stored, the 43a8e5 wasm stored, the instantiate proposal passing with admin GOV, direct fee grants, claims, the revoke, and E being refused after the revoke.

## Not verified
- Zigner amino rendering of the gov v1 and wasm msg types in steps 14 and 17 on a real device.
- A reproducible build of the 43a8e5 wasm against SOURCE_COMMIT.
- Whether the broadcast node's mempool limits match groot-wan's.
