# Headstash 2026 mainnet operator checklist (morocco-1)

Everything here is UNSIGNED. Nothing has been signed or broadcast. The artifacts are in `deploy/rendered-mainnet/phase-a/` and `phase-b/`, and every size is in `rendered-mainnet/sizes.json`.

Roles:
- **P**: the proposer, terp1gnfqzr25fgds7zn7m6njd795aat5yaf6vk7hd9 (dabarber validator). It is a cold key that signs on Zigner in amino-json.
- **E**: the ephemeral session key. It lives in memory on the operator device (cw-orch-kms `ScriptSession::open_with_key`) and signs in direct mode, never on Zigner.
- **FX**: the fee-grant wallet, terp1fxccvvhhy43tvet2ah7jqwq4cwl9k3dxmww052. It grants claim fees directly, with no authz.
- **U**: a throwaway uploader key for the frontend chunks (direct mode, not Zigner). Its fees come from a capped FX feegrant (doc A18) that is revoked afterwards (doc A20). It holds no funds.
- **GOV**: the gov module, terp10d07y265gmmuvt4z0w9aw880jnsr700jag6fuq. It is the wasm admin and the contract owner.

Wasm: `cw_headstash.wasm`, sha256 dd40ded55eb436f296fa8eac0d1794f69c5c7369f5a137b1b52dc2cb18e22c52 (857,272 B; v0.0.1 build: `--features zk-api`, no `legacy`, so the pre-v1/bridge/epoch-batch paths and sudo are compiled out (spec Appendix V); gov EmergencyReclaim (Appendix R) and the pauser role (Appendix P); largest function 38,344; cosmwasm-check pass). Reproducible: optimizer-arm64:0.17.0-rust1.91, wasm-opt -Os inline 1000. Built copies: `~/hs-backup-20261026/wasm-v001-tf-20261103/` on groot-wan (source 4de39b1, tf-denom folded in; earlier 40197344 superseded). The predicted address is terp1ed7t2p9g5myjp9phv2r2x8vmu7gu5dazzyq4sr7eja99vfynslgqp60r86.

**PAUSER (decided):** the foundation DAO terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w (confirmed by you; valid bech32, 32-byte contract address). It is set as `PAUSER` in decisions.mainnet.json and in the A14 instantiate msg (`"pauser"`). It can only Pause and Resume claims, registration and the grant relay. It cannot move funds, reclaim, sweep or change the pauser. Only a gov `update_pauser` proposal can replace it. The 2026-11-03 metadata names the pauser explicitly.

Metadata sha256 (changed with the 2026-11-03 wasm; upload before A14 is final): 01-deploy.json c58128ae4be5ca2546d5477209b6adee731e3e0113d49450eaeaabc8654fdef9, 02-circuit-store.json 4047b6bebd4832962b35cf5f198127928675f48385709daf2e1526daec1ca062, 03-fund-bind.json 5c972a9c172a0ccce90697ec73c8781f2c5a9bf9f48a9ff192ed9f32c5bc89dd.

Frontend wasm: `cw_headstash_frontend.wasm`, sha256 e72eee9b90012ce42fed2c0abd0b1fcc31b868168fa79d30d2f9d6fdd616cac2 (357,081 B). The predicted address is terp1llutyfy858ehucaleypljfgut9srz34kd5c06p27hzacv0gfyjwqg0e5n9 (salt sha256("headstash-frontend:"+label)). It is set as `frontend_contract` in the cw-headstash instantiate msg. The page manifest sha256 is 6a90d092… (`phase-a/page-manifest.json`): 5 files and 9 chunks of ≤131,072 B, with the prover at 2774af96… (-Oz, reproducible build). The loader page is NOT part of the manifest, and it is not published.

## Read from mainnet, read-only, on 2026-10-28 at height 24,144,476
- P: acct 0, seq 116. Spendable 10,980.26 TERP and 39,947.5 THIOL.
- FX: acct 19, seq 262. Spendable 2,202.74 THIOL and 8.97 TERP. It is a DelayedVestingAccount whose vesting ended in 2023, so all of it is spendable.
- Global-fee minimum gas price: 0.05 uthiol. Consensus block max_bytes: 16,777,216.
- The groot-wan node config has max_tx_bytes 4,194,304 and max_txs_bytes 1 GiB. The ~181 KB MsgExec and the ~403 KB store tx both fit. Confirm the same limits on whichever node you broadcast through.

## Before you start
1. Fill the placeholders that are still open, then re-run `render_gov_session.py` (it runs validate_rendered_proposal):
   - **FORUM_URL** (filled) = https://discord.terp.network. Both proposal summaries end with it.
   - **PROPOSAL_METADATA_URI_DEPLOY** / **_CIRCUIT** / **_FUND** are filled in `decisions.mainnet.json`: 01-deploy.json (A14) https://s3.terp.network/static/headstash/cid/c58128ae4be5ca2546d5477209b6adee731e3e0113d49450eaeaabc8654fdef9, 02-circuit-store.json (B12) https://s3.terp.network/static/headstash/cid/4047b6bebd4832962b35cf5f198127928675f48385709daf2e1526daec1ca062, 03-fund-bind.json (B14) https://s3.terp.network/static/headstash/cid/5c972a9c172a0ccce90697ec73c8781f2c5a9bf9f48a9ff192ed9f32c5bc89dd. Each is hosted byte-for-byte (HTTP 200, sha256 = `proposal-metadata/SHA256SUMS`). If you edit the metadata files, re-run `proposal_metadata.py`, re-host, re-fill and re-render, since the sha changes. The render refuses any URI that isn't ipfs:// or https://, or that exceeds 255 B (MaxMetadataLen).
   - **WAVS_POO_JSON** = `null`: WAVS is disabled in the f7873d2d contract (`InstantiateMsg.wavs: Option`). Nothing is stored and no WAVS check runs; every WAVS read fails closed, and the `wavs_status` query returns `enabled: false`. No execute path can set it later. Only gov, as wasm admin, can change it, by a migrate to new code. The smart-account authenticator handlers were removed and `sudo` decodes only `HashMerchantSudoMsg`, so attaching the contract as an authenticator is refused on chain. The dry run checks this (`wavs_status_disabled`, `neg_wavs_*`). Spec Appendix W.
   - **ANCHOR_START_HEIGHT** (filled, = 1): required (`registration.anchor_start_height` is a non-optional u64). 1 means the first anchor can be accepted immediately after the campaign is opened.
   - These are filled at runtime: EXEC_GAS/EXEC_FEE_UTHIOL, STORE_GAS/STORE_FEE_UTHIOL, CODE_ID, NOTE_ZK_ID, PROPOSAL_ID_DEPLOY, PROPOSAL_ID_CIRCUIT, PROPOSAL_ID_FUND, CLAIMER_ADDR and CLAIM_FEE_UTHIOL, FE_CODE_ID, FE_STORE_GAS/FE_STORE_FEE_UTHIOL, FE_UPLOADER_ADDR and FE_UPLOAD_EXPIRATION.
2. Read the acct and seq of P and FX again right before each signature.
3. **Deposits: P can fund only one proposal at a time.** There are three proposals (A, B1, B2), each needing 10,000 TERP (5,000 initial plus a 5,000 top-up within the 72h deposit period). P holds 10,980 TERP, so run them one after the other; each deposit is refunded when the proposal passes. A second depositor adding the top-ups is an alternative.
4. **FX THIOL top-up.** Claim fee-grant budget 4,761.7 THIOL (`FEEGRANT_RECOMMENDED_UTHIOL` 4,761,700,000) + uploader allowance 3.374003 THIOL (doc A18) + an assumed ~10 THIOL of FX's own grant-tx fees = 4,775.074003 THIOL. FX holds 2,202.74 THIOL, so send **2,572.334003 THIOL** (≥2,562.334003 without the 10 THIOL fee assumption) to FX before step B16.

## Why Phase A needs a proposal
Upload is open on mainnet, but both codes are stored with instantiate permission AnyOfAddresses[GOV], and the contracts must have gov as admin, creator (Instantiate2 address) and owner. Only gov can instantiate them, and cw-headstash refuses an instantiate without at least 1 token, so Phase A is one small proposal (1 uterp from the pool). The proposal-free alternative (P in the instantiate permission, P instantiates and later hands ownership to gov) is not used: it gives a cold key temporary ownership of both contracts and changes the predicted addresses. No step that moves the airdrop funds is outside a gov proposal.

## Phase A: deploy both contracts, publish and seal the claim page (no circuit, no funding)

| # | File (`rendered-mainnet/phase-a/`) | Who signs | What it does |
|---|------|-----------|--------------|
| A0 | none | operator device | Generate E in memory. **Regenerate `phase-a/10-session-grant/`** with the real E address and P's current sequence using `gov-session/target/release/gov-session-grant --chain-id morocco-1 --granter P --grantee E --account-number 0 --sequence <seq> --ttl-secs 86400 --allowance 300000000 --fee-denom uthiol --type /cosmos.gov.v1.MsgSubmitProposal --type /cosmos.authz.v1beta1.MsgRevoke --type /cosmos.feegrant.v1beta1.MsgRevokeAllowance --out rendered-mainnet/phase-a/10-session-grant`. The committed doc has a placeholder grantee and an expired TTL (2026-10-29 10:51 UTC). Re-render with `--eph-grantee E`. |
| A10 | `10-session-grant/sign_doc.json` | **P on Zigner** | Three GenericAuthorizations plus a 300 THIOL BasicAllowance to E, TTL 86,400 s. Sign doc 1,419 B, 1 QR frame. |
| A13 | `13-store-wasm.unsigned.json` (~403 KB) | E (direct) | MsgStoreCode, sender E, instantiate permission GOV only, fee via the feegrant. STORE_GAS ≥ 10M. Record CODE_ID. |
| A13b | `13b-store-frontend-wasm.unsigned.json` (~186 KB) | E (direct) | Same for the frontend wasm. FE_STORE_GAS ≥ 4M. Record FE_CODE_ID. |
| A15 | `15-session-revoke.unsigned.json` | E (direct) | MsgExec revoking every grant and the allowance (MsgRevoke last). Run right after A13b. |
| A14 | `14-deploy-proposal.unsigned.json` | **P on Zigner** | Sign doc 4,562 B, metadata = 01 URI, acct 0 seq 116 (amino sha256 5e521456c8e1c25eec9a9b47ae0de79230a7a7de7d19716878bf7d0cfa0c256a; file 6,222 B sha256 b14e6bd2053dff0ef65b2536702f1300832017dbcfe0a087eb96f2859622721d). **Gov proposal A**: CommunityPoolSpend 1 uterp → GOV, frontend Instantiate2 (admin + owner GOV), publish_manifest, cw-headstash Instantiate2 (admin GOV; the contract owner is the instantiating sender = GOV, the msg carries no `owner` field; frontend_contract set, 1 uterp). No campaign, no circuit: claims are refused until B14. 5,000 TERP initial deposit. Record PROPOSAL_ID_DEPLOY. |
| A17 | `17-deposit-topup.unsigned.json` | **P on Zigner** | Sign doc 353 B. +5,000 TERP on PROPOSAL_ID_DEPLOY. |
| — | vote | validators | Proposal A passes. Check both contracts: admin, creator and owner GOV; cw-headstash balance 1 uterp, `reclaim_status` null. |
| A18 | `18-fe-uploader-feegrant.unsigned.json` | **FX on Zigner** | Sign doc 499 B. Capped, expiring BasicAllowance (3,374,003 uthiol) to U. |
| A19 | `19-fe-chunks/NN-fF-iI.unsigned.json` ×9 | U (direct) | UploadChunk txs, content-addressed; the last seals v1 and makes it current. 47.48M gas total. |
| A20 | `20-fe-uploader-revoke.unsigned.json` | **FX on Zigner** | Sign doc 343 B. Revoke U's allowance. |

After Phase A both contracts are deployed and can be funded. Nothing is spent beyond 1 uterp.

## Phase B: circuit store, then gov binds and funds

| # | File (`rendered-mainnet/phase-b/`) | Who signs | What it does |
|---|------|-----------|--------------|
| B0 | none | operator device | Generate a new E. Generate `phase-b/10-session-grant/` with the same command (`--out rendered-mainnet/phase-b/10-session-grant`) and re-render B12/B15 with `--eph-grantee E`. |
| B10 | `10-session-grant/sign_doc.json` | **P on Zigner** | Second session grant (~1,419 B). |
| B12 | `12-circuit-exec.unsigned.json` (181,753 B, sha256 2709cc7b0157ac39ef8e98633854c11e30faf4f9df3e9c84eb4d00e9e3d674da; metadata = 02 URI) | E (direct) | MsgExec{MsgSubmitProposal{MsgStoreFullCircuit}}, **gov proposal B1** (circuit upload is Nobody on mainnet). EXEC_GAS ≥ 25M. Record PROPOSAL_ID_CIRCUIT. |
| B15 | `15-session-revoke.unsigned.json` | E (direct) | Revoke the session right after B12. |
| B17a | `17-deposit-topup-circuit.unsigned.json` | **P on Zigner** | Sign doc 354 B. +5,000 TERP on PROPOSAL_ID_CIRCUIT. |
| — | vote | validators | B1 passes. Record NOTE_ZK_ID. |
| B14 | `14-fund-bind-proposal.unsigned.json` | **P on Zigner** | Sign doc 2,902 B, metadata = 03 URI (amino sha256 ec35c009aacdddac0c7f065a64b5b5c3a0c000c9c95704035a8ac6088591ea5e; file 4,348 B sha256 300059c0e2d0c9bce11f970337dfbe947c6b1d81ff1f3b356feed297f07a7da5). **Gov proposal B2**: set_epoch_campaign (cutoff 44,444,444, burn on), bind_note_circuit NOTE_ZK_ID, CommunityPoolSpend 66,456,668,999,999 uterp straight to the contract (FUNDING − the 1 uterp from A). Record PROPOSAL_ID_FUND. |
| B17b | `17-deposit-topup-fund.unsigned.json` | **P on Zigner** | Sign doc 351 B. +5,000 TERP on PROPOSAL_ID_FUND. |
| — | vote | validators | B2 passes: contract balance = FUNDING, claims open. |
| B16 | `16-feegrant-terp1fx.unsigned.json` | **FX on Zigner** | Sign doc 450 B. One direct BasicAllowance per claimer. Do the THIOL top-up first. |

## Emergency and end of campaign (gov proposals, not rendered)
- **Before the cutoff (h ≤ 44,444,444):** a proposal with `MsgExecuteContract{sender GOV, msg {"emergency_reclaim":{}}}` sends the whole contract balance to the community pool (MsgFundCommunityPool from the contract) and halts claims, registration and sweep for good. Only GOV (contract owner and wasm admin) can run it. After the cutoff it is refused.
- **After the cutoff:** `{"sweep_unclaimed":{}}` burns what is left. Before the cutoff it is refused.
- **Fast halt (no proposal):** the pauser (or GOV) sends `{"pause":{}}` to stop claims and registration right away, and `{"resume":{}}` to restart them. Pausing does not extend the cutoff. Reclaim and the post-cutoff sweep still work while paused. After a reclaim, resume is refused. Check state with `{"pause_status":{}}`.

## Afterwards
- Check that the frontend admin and owner are both GOV, that config `current` is 1, and that U's allowance is gone.
- Check that the contract admin and owner are both GOV, the balance equals FUNDING, and the campaign root and bound zk_id are correct.
- Run `terpd q authz grants P E` and `terpd q feegrant grant P E` for both session keys. Both must come back empty or not found.

## Verified locally
Re-run 2026-11-02 on the v0.0.1 legacy-off wasm 40197344 + frontend e72eee9b (same scripts): pause **109/0**, emergency reclaim **90/0**, burn **89/0**, browser `gov_frontend_e2e.py` **57/0**. Earlier runs below:
Local morocco-1-like chain, terpd v6.4.0, test keys standing in for P/E/FX/U, wasms f7873d2d + fe096bf1, 2026-10-30:
- `run_direct.py` GOV_ROLES=1 RECLAIM=1 PAUSE=1 (pauser scenario, wasm e8690f93): **109 PASS / 0 FAIL**. Gov sets the pauser. The pauser pauses, and while paused claims, registration, the grant relay and CheckNoteClaim are refused. The pauser cannot reclaim, sweep or change the pauser. After resume a claim succeeds. A re-pause is followed by a gov reclaim, which succeeds while paused, and resume is refused after the reclaim. Reclaim/burn/browser were re-run on e8690f93: 90/0, 89/0, 57/0.
- `run_direct.py` GOV_ROLES=1 RECLAIM=1 (emergency scenario): **90 PASS / 0 FAIL**. Order: session A (stores, revoke) → proposal A (contracts deployed, admin/creator/owner GOV, balance 1 uterp, reclaim_status null, bind refused) → session B → B1 circuit → B2 campaign+bind+pool spend (balance = FUNDING) → registrations/claims → non-gov reclaim refused → gov EmergencyReclaim passes before cutoff, contract emptied, community pool credited (+reclaimed, plus block community tax) → claim/grant/register/sweep refused → second reclaim proposal FAILED.
- `run_direct.py` GOV_ROLES=1 (burn scenario): **89 PASS / 0 FAIL**. Gov burn before cutoff FAILED (campaign has not ended), after cutoff claims/registration closed, gov reclaim after cutoff FAILED (only SweepUnclaimed allowed), gov sweep burns the rest.
- `gov_frontend_e2e.py` (browser): **57 PASS / 0 FAIL**. Frontend manifest published by gov in A, 9/9 chunks uploaded on a feegrant (tampered chunk refused), page loaded from chain, 43/43 notes registered and claimed in the browser, 43/43 replays refused. In this harness the chunk upload runs after B2 rather than between A and B1 (on mainnet A18–A20 follow A); the docs do not depend on the order.
- Local circuit zk_id fell back to 1.

## Totals (estimated at 0.05 uthiol)
- E session gas: A: store ≥10M, FE store ≥4M, revoke ~0.11M; B: exec ≥25M, revoke ~0.11M. ≈39M gas, ≈1.96 THIOL in total, out of a 300 THIOL allowance per session.
- U uploads: 47.48M gas, 2.374 THIOL (from FX).
- P: 3 × 10,000 TERP deposits (A, B1, B2), run one after the other, plus Zigner tx fees of a few THIOL.
- FX: 4,761.7 THIOL claim budget + 3.374003 THIOL uploader allowance + ~10 THIOL fees − 2,202.74 held = top-up of **2,572.334003 THIOL**.

## Not verified
- Zigner amino rendering of the gov v1 and wasm msg types in steps A14, B14 and the top-ups on a real device.
- (Done 2026-11-02: cw_headstash 40197344 and frontend e72eee9b rebuilt byte-identically from the consolidate tree after a /tmp wipe; SOURCE_COMMIT pinning still to be stated by the build owner.) The prover is reproducible (2774af96, via scripts/build-browser-prover.sh).
- The real upload gas on mainnet. Doc A18 carries 1 THIOL of headroom over the estimate.
- Whether the broadcast node's mempool limits match groot-wan's.
