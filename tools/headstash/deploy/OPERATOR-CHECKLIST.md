# Headstash 2026 mainnet operator checklist (morocco-1)

Everything here is UNSIGNED. Nothing has been signed or broadcast. The artifacts are in `deploy/rendered-mainnet/`, and every size is in `rendered-mainnet/sizes.json`.

Roles:
- **P**: the proposer, terp1gnfqzr25fgds7zn7m6njd795aat5yaf6vk7hd9 (dabarber validator). It is a cold key that signs on Zigner in amino-json.
- **E**: the ephemeral session key. It lives in memory on the operator device (cw-orch-kms `ScriptSession::open_with_key`) and signs in direct mode, never on Zigner.
- **FX**: the fee-grant wallet, terp1fxccvvhhy43tvet2ah7jqwq4cwl9k3dxmww052. It grants claim fees directly, with no authz.
- **U**: a throwaway uploader key for the frontend chunks (direct mode, not Zigner). Its fees come from a capped FX feegrant (doc 18) that is revoked afterwards (doc 20). It holds no funds.
- **GOV**: the gov module, terp10d07y265gmmuvt4z0w9aw880jnsr700jag6fuq. It is the wasm admin and the contract owner.

Wasm: `cw_headstash.wasm`, sha256 76c737edaf25bf64ee3c1d93ae4943164d83b1901adcd893e3679ce5d62281de (1,133,358 B, 410,993 B gzip -9). The predicted address is terp1l3qtt2gwvwyshhlsr7l8tac4hmz7xwav5e0gpret92qw85hh83ts7298qn.

Frontend wasm: `cw_headstash_frontend.wasm`, sha256 fe096bf119e8e0b150fa32fff405f46b53917fe9a219ecfeda112cb3fe66c2a1 (357,352 B). The predicted address is terp1cjt5eu0egdrn4u3xpceevmj2ncagg2xt53kq92r584m3yd58lecq9d7f7y (salt sha256("headstash-frontend:"+label)). It is set as `frontend_contract` in the cw-headstash instantiate msg. The page manifest sha256 is 6a90d092… (`page-manifest.json`): 5 files and 9 chunks of ≤131,072 B, with the prover at 2774af96… (-Oz, reproducible build). The loader page is NOT part of the manifest, and it is not published.

## Read from mainnet, read-only, on 2026-10-28 at height 24,144,476
- P: acct 0, seq 116. Spendable 10,980.26 TERP and 39,947.5 THIOL.
- FX: acct 19, seq 262. Spendable 2,202.74 THIOL and 8.97 TERP. It is a DelayedVestingAccount whose vesting ended in 2023, so all of it is spendable.
- Global-fee minimum gas price: 0.05 uthiol. Consensus block max_bytes: 16,777,216.
- The groot-wan node config has max_tx_bytes 4,194,304 and max_txs_bytes 1 GiB. The ~181 KB MsgExec and the ~547 KB store tx both fit. Confirm the same limits on whichever node you broadcast through.

## Before you start
1. Fill the placeholders that are still open, then re-run `render_gov_session.py` (it runs validate_rendered_proposal):
   - **PROPOSAL_METADATA_URI** and **FORUM_URL** are still unknown.
   - **WAVS_POO_JSON** = `null`: WAVS is disabled in the 76c737 contract (`InstantiateMsg.wavs: Option`). Nothing is stored and no WAVS check runs; every WAVS read fails closed, and the `wavs_status` query returns `enabled: false`. No execute path can set it later. Only gov, as wasm admin, can change it, by a migrate to new code. The smart-account authenticator handlers were removed and `sudo` decodes only `HashMerchantSudoMsg`, so attaching the contract as an authenticator is refused on chain. The dry run checks this (`wavs_status_disabled`, `neg_wavs_*`). Spec Appendix W.
   - **ANCHOR_START_HEIGHT** (filled, = 1): required (`registration.anchor_start_height` is a non-optional u64). 1 means the first anchor can be accepted immediately after the campaign is opened.
   - These are filled at runtime: EXEC_GAS/EXEC_FEE_UTHIOL, STORE_GAS/STORE_FEE_UTHIOL, CODE_ID, NOTE_ZK_ID, PROPOSAL_ID_CIRCUIT, CLAIMER_ADDR and CLAIM_FEE_UTHIOL, FE_CODE_ID, FE_STORE_GAS/FE_STORE_FEE_UTHIOL, FE_UPLOADER_ADDR and FE_UPLOAD_EXPIRATION.
2. Read the acct and seq of P and FX again right before each signature.
3. **Deposits: P can't fund both proposals at once.** Each proposal needs 10,000 TERP, which is 5,000 initial plus 5,000 top-up within the 72h deposit period. P holds 10,980 TERP, so run the proposals one after the other: the circuit proposal deposit is refunded when it passes, and only then do you submit the instantiate proposal. Another option is for a second depositor to add the top-up.
4. **FX THIOL top-up.** Claim fee-grant budget 4,761.7 THIOL (`FEEGRANT_RECOMMENDED_UTHIOL` 4,761,700,000) + uploader allowance 3.374003 THIOL (doc 18) + an assumed ~10 THIOL of FX's own grant-tx fees = 4,775.074003 THIOL. FX holds 2,202.74 THIOL, so send **2,572.334003 THIOL** (≥2,562.334003 without the 10 THIOL fee assumption) to FX before step 16.

## Steps, in order

| # | File | Who signs | What it does |
|---|------|-----------|--------------|
| 0 | none | operator device | Generate E in memory. **Regenerate `10-session-grant/` with the real E address and P's current sequence** using `gov-session/target/release/gov-session-grant --chain-id morocco-1 --granter P --grantee E --account-number 0 --sequence <seq> --ttl-secs 86400 --allowance 300000000 --fee-denom uthiol --type /cosmos.gov.v1.MsgSubmitProposal --type /cosmos.authz.v1beta1.MsgRevoke --type /cosmos.feegrant.v1beta1.MsgRevokeAllowance --out rendered-mainnet/10-session-grant`. The committed doc uses a placeholder grantee and an expiry that has already passed. Then re-render 12, 13 and 15 with `--eph-grantee E`. |
| 10 | `10-session-grant/sign_doc.json` | **P on Zigner** | One amino tx with three GenericAuthorizations (SubmitProposal, Revoke, RevokeAllowance) plus a BasicAllowance of 300 THIOL to E, TTL 86,400s. Sign doc is **1,419 B**, 1 QR frame, within the 65,536 B limit. The session only needs ~2 THIOL of fees: 25M gas plus 10M gas at 0.05. |
| 12 | `12-circuit-exec.unsigned.json` (~181 KB) | E (direct) | MsgExec{MsgSubmitProposal{MsgStoreFullCircuit}}. Proposer is P, initial deposit is 5,000 TERP debited from P, and the fee comes from the feegrant. It used 22.2M gas locally, so set EXEC_GAS ≥ 25M (fee ~1.25 THIOL). Record PROPOSAL_ID_CIRCUIT. |
| 13 | `13-store-wasm.unsigned.json` (~547 KB) | E (direct) | MsgStoreCode with sender E and instantiate permission limited to GOV. Upload is open on mainnet, and the fee comes from the feegrant. It used 7.6M gas locally, so set STORE_GAS ≥ 10M. Record CODE_ID. |
| 17 | `17-deposit-topup.unsigned.json` | **P on Zigner** | Sign doc 354 B. MsgDeposit of +5,000 TERP on PROPOSAL_ID_CIRCUIT within 72h. Use 30M gas, because the large proposal ran out of gas at 8M locally. |
| 13b | `13b-store-frontend-wasm.unsigned.json` (~186 KB) | E (direct) | MsgStoreCode of the frontend wasm with sender E and instantiate permission limited to GOV. It used 2.56M gas locally, so set FE_STORE_GAS ≥ 4M. Record FE_CODE_ID. Run it before 15. |
| 15 | `15-session-revoke.unsigned.json` | E (direct) | MsgExec that revokes each grant and the allowance, with MsgRevoke last. Run it as soon as 12 and 13 are done. Locally it used 109k gas. TTL expiry is only a backstop. |
| — | vote | validators | The circuit proposal passes. Record NOTE_ZK_ID from the events. |
| 14 | `14-instantiate-proposal.unsigned.json` | **P on Zigner** | Sign doc 6,112 B. Small gov proposal: CommunityPoolSpend to GOV, then the frontend Instantiate2 (FE_CODE_ID, admin GOV, owner GOV), then publish_manifest (activate), then the cw-headstash Instantiate2 (CODE_ID, admin GOV, owner GOV, frontend_contract set), then set_epoch_campaign, then bind_note_circuit. Defaults: ANCHOR_INTERVAL 3430, MIN_ANONYMITY_SET 1, CUTOFF fixed at block 44,444,444 (in set_epoch_campaign; on 2026-10-30 that was ~20.25M blocks / ~873 days away at 3.72 s/block), BURN_UNCLAIMED on. Initial deposit 5,000 TERP, then a top-up to 10,000 using the step-17 shape with the new proposal id, signed by P on Zigner. |
| 18 | `18-fe-uploader-feegrant.unsigned.json` | **FX on Zigner** | Sign doc 499 B. BasicAllowance of 3,374,003 uthiol to U, with FE_UPLOAD_EXPIRATION. Run it after 14 passes. |
| 19 | `19-fe-chunks/NN-fF-iI.unsigned.json` ×9 | U (direct) | UploadChunk txs. They are permissionless and content-addressed, so a tampered chunk is refused. The last chunk seals v1 and makes it current. Gas is 600k + 64·bytes, 47.48M in total (2.374 THIOL). The largest tx is 175 KB. |
| 20 | `20-fe-uploader-revoke.unsigned.json` | **FX on Zigner** | Sign doc 343 B. MsgRevokeAllowance for U. |
| 16 | `16-feegrant-terp1fx.unsigned.json` | **FX on Zigner** | Sign doc 450 B. One direct BasicAllowance in uthiol from FX for each claimer. Do the THIOL top-up first. |

## Afterwards
- Check that the frontend admin and owner are both GOV, that config `current` is 1, and that U's allowance is gone.
- Check that the contract admin and owner are both GOV, the balance equals FUNDING, and the campaign root and bound zk_id are correct.
- Run `terpd q authz grants P E` and `terpd q feegrant grant P E`. Both must come back empty or not found.

## Verified locally (v6.4.0, test keys standing in for P/E/relay-granter)
`tools/headstash/e2e/run_direct.py` with GOV_ROLES=1 covered session grant, E's MsgExec proposal, proposer = granter, the 5 TERP initial deposit from the granter, the vote passing, the circuit stored, the 1305e4 and fe096bf1 wasms stored by E, the frontend instantiated by gov (admin, creator and owner all GOV) with the manifest published, a non-gov publish refused, U uploading 9/9 chunks on an FX-style feegrant (tampered chunk refused, sealed, current 1, grant revoked), the page loaded from chain with every hash checked, register and claim of 43/43 notes, replays rejected, the instantiate proposal passing with admin GOV, direct fee grants, claims, the revoke, and E being refused after the revoke.

## Totals (estimated at 0.05 uthiol)
- E session gas: exec ≥25M, store ≥10M, FE store ≥4M, revoke ~0.11M. That is ≈39M gas, ≈1.96 THIOL, out of the 300 THIOL session allowance.
- U uploads: 47.48M gas, 2.374 THIOL (from FX).
- P: 2 × 10,000 TERP deposits, run one after the other, plus Zigner tx fees of a few THIOL.
- FX: 4,761.7 THIOL claim budget + 3.374003 THIOL uploader allowance + ~10 THIOL fees − 2,202.74 held = top-up of **2,572.334003 THIOL**.

## Not verified
- Zigner amino rendering of the gov v1 and wasm msg types in steps 14 and 17 on a real device.
- A reproducible build of the 76c737 and fe096bf1 wasms against SOURCE_COMMIT. The prover is reproducible (2774af96, via scripts/build-browser-prover.sh).
- The real upload gas on mainnet. Doc 18 carries 1 THIOL of headroom over the estimate.
- Whether the broadcast node's mempool limits match groot-wan's.
