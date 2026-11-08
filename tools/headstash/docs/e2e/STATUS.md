# HEADSTASH-E2E-LEAD status

Updated: 2026-10-12 19:40 GMT. One paying `ProcessHeadstash` is included. Height 77, code 0, tx `405F2A3E5B2F80D8DF466775CCFA6A4181FD9EF5FF085B008633785EC6DC18EA`. Contract `ibc/ATOM` went from `10000000` to `5000000`. The bank send was `5000000ibc/ATOM` to `terp1rhnn6ljajsgx9n69ujdwgau5hjrgd2elw7w7k4f4q3lvwxz4qvqqfqv9jc`. The page click of that same payload then committed at height 207 as `nullifier already exists`.

## Done

- Read `LEAD-BRIEF.md`. Named Headstash panes are absent. Recovered the suite from the tree.
- Laptop `~/abstract/terp-core` has no `.git`, no `crates/headstash`, and `crates/ict-rs` is docs only. `terp-orch` still names that missing path.
- groot-wan (`groot.local`) has the suite at the same paths: `crates/headstash`, `crates/ict-rs` (including `examples/headstash.rs`), Docker 29 client, image `registry.terp.network/terp-core:v6.4.0` (terpd 6.4.0, `proof_instance_*` strings present), local Hermes image `ghcr.io/informalsystems/hermes:1.13.1`, `terpd` 6.0.1 and `hermes` on the host PATH.
- Prebuilt wasm: `crates/headstash/artifacts/cw_headstash.wasm` (2026-10-05, contains `proof_instance_verify` and the personal-sign challenge error). Eligibility wasm: `contracts/cw-reg-eligibility/artifacts/cw_reg_eligibility.wasm`. No manifold wasm. VK blob: `artifacts/headstash_vk.bin` (17MB).
- Claim page on groot: `websites/dao-dao-ui/apps/dapp/pages/headstash.tsx` → `HeadstashClaimPage`. It queries LCD `EpochRoot`. It does not broadcast and does not send `ProcessHeadstash`.

## In progress

- Nothing is running. Round 11 finished `EXIT:2` because `anchor_mismatch` hit an account sequence mismatch. The paying claim passed. Chains are still up (`HEADSTASH_E2E_KEEP=1`): LCD `http://127.0.0.1:55076`, RPC `http://127.0.0.1:55079`, chain B LCD `http://127.0.0.1:55080`. Page `http://127.0.0.1:19820/`. Signer `http://127.0.0.1:19821/broadcast`.

## Crash 14:2x

Coordinator note 14:29 GMT: groot-wan lost power, came back, load about 130. No cargo, no `make headstash-e2e`, no extra chain containers, no nginx, no cloudflared until about 14:45 and the load had settled. One inventory SSH at 10:57 EDT (14:57 GMT). Host uptime was 32 minutes, so the reboot was about 10:25 EDT / 14:25 GMT. Load at that check was 6.32 8.84 9.91. Nginx and cloudflared were not started.

What survived on disk:

- Uncommitted sources: `crates/ict-rs/examples/headstash_local_e2e.rs` (`submit_positive_claim`, `max_body_bytes`), `terp-orch` `HEADSTASH_PROVE_ONLY`, the `NewFungible` `msg_mint` patch, the e2e scripts, the claim page, the signer, and `broadcast.spec.cjs`.
- `test-press` is still commented out of the headstash workspace members (`Cargo.toml` lines 8–9). The `zk-test-press` path dependency at line 101 is not a member.
- `target/` is still present for `crates/terp-rs`, `crates/ict-rs`, and `crates/headstash`. There is no repo-root `target/`.
- Docker volumes `headstash-e2e-local-headstash-a-1-val-0`, `headstash-e2e-local-headstash-b-1-val-0`, `headstash-e2e-local-hermes-relayer`, `ict-rs-cargo-git`, `ict-rs-cargo-registry`.
- Images were still local. The run log says `Image already exists locally, skipping pull`.

What the reboot wiped:

- `/tmp/headstash-e2e-run.log` from the pre-reboot run is absent. The Verified rows above that reboot are the remaining record. Container `98132ba647fc` did not come back. The containers seen at 10:57 were created by the make that started on this boot (`Up 2 minutes` against a 32-minute uptime).

Half-finished:

- The negative-case harness rebuild from before the reboot never got a finished binary run. The gas-limit edits were on disk. Those chains died with the reboot.
- On this boot, `make headstash-e2e` (wrapper pid 39470) was already running when the load was checked. It had passed the prover (compile failed, exit 101) and was creating the IBC channel. It then finished. Log `/tmp/headstash-e2e-round2.log` ends `EXIT:2` (`make` prints `Error 1` for the recipe). Report `/tmp/headstash-e2e/report.json`. No `claim_proof.json`.
- Playwright Chromium did finish downloading (`PLAYWRIGHT_READY`). The first click failed before any broadcast. See the click row below.

## Verified

Commands on groot-wan, 2026-10-12, before the host rebooted. Those containers are not still running. Report: `/tmp/headstash-e2e/report.json` (that file is replaced when the new run writes one). Run log: `/tmp/headstash-e2e-run.log`. Script exit 1 because of the five `fail` rows below.

- `docker port` on chain A container `98132ba647fc`: `1317/tcp -> 127.0.0.1:55000`, `9090/tcp -> 127.0.0.1:55001`, `26656/tcp -> 127.0.0.1:55002`, `26657/tcp -> 127.0.0.1:55003`. Chain B LCD `127.0.0.1:55004`, RPC `127.0.0.1:55007`. No `0.0.0.0` publish.
- Image used: `registry.terp.network/terp-core:v6.4.0`. Log line: `Image already exists locally, skipping pull`.
- Wasm store: claim code 1, witness code 2.
- Claim instantiate passed. WAVS proof-of-possession from `print_wavs` was accepted by terpd 6.4.0. Contract `terp14hj2tavq8fpesdwxxcu44rty3hh90vhujrvcmstl4zr3txmfvw9skqhzr5`.
- Witness instantiate passed. Query `verify_min_holders` with a one-byte proof returned `{"data":{"ok":false}}`.
- Hop: tx `FB343C86AF38C38CD7A802EB3D5D0A7C8CAE5CCEC6723388A0939E1E598B5EBA`. Chain B received 1000 of `ibc/942CC0E3A5718E6BCB73E47975E225EB003CF654D256737B458EFBA5140AD6FE`. Chain A `uterp` went from 20000000000 to 19999999000.
- Hop timeout: relayer stopped, tx `E49F00AD686BA2E4BC51766DFD49B6E85525090FAD0B1AC04801D662EF145789`. Destination balance stayed 1000.
- `store-circuit`: gap. RPC `400` `http: request body too large` on the 17MB blob.
- Static page `http://127.0.0.1:19820/index.html` returned HTTP 200. LCD `http://127.0.0.1:55000/cosmos/base/tendermint/v1beta1/node_info` returned HTTP 200. The page's buttons were not clicked in a browser.
- `print_wavs` build finished in 11m 13s, exit 0. `headstash_local_e2e` first build exit 0.

Harness rows that are not passes: `anchor_mismatch`, `bad_denom`, `double_claim`, `bad_proof`, `wrong_address` all `fail` with `invalid tx JSON: EOF while parsing a value at line 1 column 0`. That string is the empty-stdout parser, not the contract.

### Same boot as the 14:25 GMT reboot

Log `/tmp/headstash-e2e-round2.log`. Report `/tmp/headstash-e2e/report.json`. Recipe exit 1, `make` exit 2. Chain A LCD `http://127.0.0.1:55004`, RPC `http://127.0.0.1:55007`. Chain B LCD `http://127.0.0.1:55008`. Claim `terp14hj2tavq8fpesdwxxcu44rty3hh90vhujrvcmstl4zr3txmfvw9skqhzr5`. Witness `terp1nc5tatafv6eyq7llkr2gv50ff9e22mnf70qgjlv737ktmt4eswrq64jqpj`. These addresses match the pre-reboot run because the test keys and account sequence are deterministic. They are new containers.

- `tx_body_limit` pass. Container config has `rpc.max_body_bytes = 67108864` and `mempool.max_tx_bytes = 67108864`. Genesis text also contains `67108864`.
- `loopback_publish`, `store_wasm` (codes 1 and 2), `claim_instantiate`, `witness_instantiate`, `witness_bad_proof` (`{"data":{"ok":false}}`) pass.
- `bad_denom` pass. Committed height 41, code 5, tx `DBFB3AAD570E898D763C8C6FF98C52D0E108A8FE1C4AA84CE283DCF0F0548D50`, log `incorrect token denom`.
- `bad_proof` pass. Committed height 45, code 5, tx `FCCA66E62B653EC015BAC2B3A5C9486F3619C9EB2C182D6B7202A5F0B22BC342`, log `headstash claim proof verify requires zk-api feature + zk wasmvm, or claim_mock_verify=true for lab`.
- Hop pass. Tx `FEBD7BF1CBB964C12765C4FD635DABF7D1C4C66695A72B96952042A109BDF2A5`. Chain B received 1000 of `ibc/942CC0E3A5718E6BCB73E47975E225EB003CF654D256737B458EFBA5140AD6FE`. Chain A `uterp` went from 20000000000 to 19999999000.
- Hop timeout pass. Tx `A755A4148453B4F6C5AC67E7F9D93351CF75EA30DF4210261C7294A49ECE59C3`. Destination balance stayed 1000. Relayer was stopped.
- Page click pass, after a CORS fix, not in the make recipe. Playwright on groot clicked `Broadcast ProcessHeadstash`. Tx `F52195EEB2357D9263B68FC39FFDCC7FB8F2893415693F092B40068914832DA6` committed at height 848, code 5, log `claim anchor does not match registered root_id 0`. The payload was the harness `anchor_mismatch` message, so this is a broadcast of a rejecting claim, not a successful claim. The first click in the recipe printed `TypeError: Failed to fetch` because the signer answered OPTIONS with 501. The signer now answers that preflight with 204 and `access-control-allow-origin: http://127.0.0.1:19820`.

### zk-api wasm run (proof reused, params rejected)

Log `/tmp/headstash-e2e-round3.log`. `EXIT:2` because the Playwright click timed out. No case status is `fail`. Chain A LCD `http://127.0.0.1:55012`, RPC `http://127.0.0.1:55015`. Chain B LCD `http://127.0.0.1:55016`. Claim `terp14hj2tavq8fpesdwxxcu44rty3hh90vhujrvcmstl4zr3txmfvw9skqhzr5`. Witness `terp1nc5tatafv6eyq7llkr2gv50ff9e22mnf70qgjlv737ktmt4eswrq64jqpj`.

- `tx_body_limit`, `loopback_publish`, `store_wasm` (codes 1 and 2), `claim_instantiate`, `witness_instantiate`, `witness_bad_proof` pass.
- `anchor_mismatch` pass. Height 52, code 5, tx `D4479ADD43F11E8516BC0B206F594D9D354526CC987B06255C293B72ED202B9D`, log `claim anchor does not match registered root_id 0`.
- `bad_denom` pass. Height 54, code 5, tx `F1B72462BD83938BAD61B5E21BE1E0ECE0A2BAA7FCF91E9F012705D6F9B829EA`, log `incorrect token denom`.
- `bad_proof` pass. Height 59, code 5, tx `6BB33F7FF535892566419ED9B28AF1D3DD5A553F0D82E0DD374553411376AC94`, log `circuit_id is 0: SetCircuitId after store-circuit before real verify`.
- `store_circuit` gap. `param bytes cannot be longer than 33554432 bytes` (`x/wasm/types/tx.go:99`). Params length 33554500.
- `positive_claim` gap. The host proof was not submitted.
- `double_claim` and `wrong_address` gap. The proof gate returned `circuit_id is 0` before the nullifier and recipient checks.
- Hop pass. Tx `CCE869101333499FD5A23345BCAF761D4DDE8636B178F365734190CEB2EFCABE`. Chain B received 1000 of `ibc/942CC0E3A5718E6BCB73E47975E225EB003CF654D256737B458EFBA5140AD6FE`. Chain A `uterp` went from 20000000000 to 19999999000.
- Hop timeout pass. Tx `B546AEBF330D8459B3C045B222D7E8B5C0243B0E2C6D0A1222FEE336F7F913B3`. Destination balance stayed 1000.
- Playwright click timed out at 180s. `broadcast.json` kind is `positive`. The new signer could not bind `19821` because an older signer was still listening, and that older process answered POST with 502. `#out` never received a txhash.

### Round 10, circuit stored, public inputs rejected

Log `/tmp/headstash-e2e-round10.log`. `EXIT:2` because one or more cases failed. Image `terp-core:headstash-circuit-limit`. Chain A LCD `http://127.0.0.1:55068`, RPC `http://127.0.0.1:55071`. Chain B LCD `http://127.0.0.1:55072`. Claim `terp14hj2tavq8fpesdwxxcu44rty3hh90vhujrvcmstl4zr3txmfvw9skqhzr5`. Witness `terp1nc5tatafv6eyq7llkr2gv50ff9e22mnf70qgjlv737ktmt4eswrq64jqpj`.

- `tx_body_limit`, `loopback_publish`, `store_wasm` (codes 1 and 2), `claim_instantiate`, `witness_instantiate`, `witness_bad_proof` pass.
- `store_circuit` pass. Detail `circuit_id=1`. This is the post-synthesis constraint system.
- `bad_denom` pass. Height 66, code 5, log `incorrect token denom`.
- Hop pass. Tx `45384D77035C3BC965F79DBA540FAE5083F38C0B3A1389667446C046E0A75ED5`. Chain B received 1000 of `ibc/942CC0E3A5718E6BCB73E47975E225EB003CF654D256737B458EFBA5140AD6FE`. Chain A `uterp` went from 20000000000 to 19999999000.
- Hop timeout pass. Tx `EB401AA3C4A50191754C72575BE0DEFEE13C26E8DD7EAB4DF024A4A84B638188`. Destination balance stayed 1000.
- `anchor_mismatch` fail. The harness recorded sdk code 32, `account sequence mismatch, expected 8, got 7`, tx `D6412686DB30C33500EF52FCCC94CD228D6C33A7EF6973FF1F4EFC157E7EA26A`. That hash is not in the block store. It is not the contract anchor error.
- `double_claim` fail. Height 68, code 29, tx `DD9520E2BECE344E0BF206322A50FBDB3A9BAB2BD5BB00ADE03BF58E43F71E97`, gas used 156746. Same 200-byte public-input error. The nullifier check is after the challenge and before verify, but the earlier positive claim rolled back, so this tx also reached verify.
- `bad_proof` fail. Height 70, code 29, tx `DB78AE670675726483E8297B92910675221596245EB933C74004D0CB3F47CB9F`, gas used 152167. Same 200-byte error. This is not `invalid headstash proof`.
- `wrong_address` fail. Height 72, code 29, tx `0B1D1E582624A219E103713D945E9C0FEA77A663D5ADCAEF9FD4BD46664B4462`, gas used 152167. Same 200-byte error. Recipient is checked only after verify.
- `positive_claim` fail. Height 74, code 29, tx `601C56DD3771C5824B65DB1172F8CB99CBD7AEBD597EA0ED4C3FBAEFC133F59D`, gas used 222193. Balance `10000000 -> 10000000`. Log `vesta public-input bytes length 200 is not a multiple of 32`.

### Round 11, paying claim

Log `/tmp/headstash-e2e-round11.log`. `EXIT:2` because `anchor_mismatch` failed. Image `terp-core:headstash-circuit-limit`. The script reused the host-verified K=18 proof and `artifacts/cw_headstash_zkapi.wasm` (1177086 bytes). Chain A LCD `http://127.0.0.1:55076`, RPC `http://127.0.0.1:55079`. Chain B LCD `http://127.0.0.1:55080`. Claim `terp14hj2tavq8fpesdwxxcu44rty3hh90vhujrvcmstl4zr3txmfvw9skqhzr5`. Witness `terp1nc5tatafv6eyq7llkr2gv50ff9e22mnf70qgjlv737ktmt4eswrq64jqpj`. Container `d184c7b810062c3b9a960613d18f1abcb4937436b10ffa1bd5eb9b1be088309c`.

`proof_instance_verify` received eight 32-byte scalars. The challenge still uses the first 168 bytes of the 200-byte wire.

- `tx_body_limit`, `loopback_publish`, `store_wasm` (codes 1 and 2), `claim_instantiate`, `witness_instantiate`, `witness_bad_proof`, `store_circuit` (`circuit_id=1`) pass.
- `bad_denom` pass. Height 69, code 5, tx `B6023784EFBF271F10F84C33FDF0FA9345EE37EA30133F1E76DBFEF1893A35D7`, gas used 118649. Log `incorrect token denom`.
- `bad_proof` pass. Height 73, code 5, tx `16D271A155C16A9EA773EE6E108AEE4C5C807D90EAC4A6D769ED9DE617038AB2`, gas used 160230. Log `invalid headstash proof`.
- `positive_claim` pass. Height 77, code 0, tx `405F2A3E5B2F80D8DF466775CCFA6A4181FD9EF5FF085B008633785EC6DC18EA`, gas wanted 20000000, gas used 265479. Wasm event `action=process_headstash`. `coin_spent` `5000000ibc/ATOM`. Transfer sender is the claim contract, recipient `terp1rhnn6ljajsgx9n69ujdwgau5hjrgd2elw7w7k4f4q3lvwxz4qvqqfqv9jc`. Contract balance of `ibc/ATOM` is `5000000` (was `10000000`). `uterp` on the contract stayed `5000000`.
- Hop pass. Tx `7BE2C1FFC0CDB3EF2302082FEDC0F546BD30FDD3DE2650EE89CED0CC2BEE3E2E`. Chain B received 1000 of `ibc/942CC0E3A5718E6BCB73E47975E225EB003CF654D256737B458EFBA5140AD6FE`. Chain A `uterp` went from 20000000000 to 19999999000.
- Hop timeout pass. Tx `9DEDFC5C131238257F0F3B7136650BD2B05B5C30C5D2626852765752AC96D3B6`. Destination balance stayed 1000.
- Page click pass as a broadcast. Playwright printed `PLAYWRIGHT_CLICK_OK txhash=9D35DF145F4A63D68959EABDA7F83D600C72C818B2A9D88B1FE1EBC99E43EE26`. That tx committed at height 207, code 5, gas used 177530, log `nullifier already exists`. `broadcast.json` kind is `positive`. The harness claim had already spent the nullifier, so this click is the duplicate, not a second payment.
- `anchor_mismatch` fail. Sync ack code 32, `account sequence mismatch, expected 8, got 7`, tx `B4D68DFDE4AAA6E8CCBBB0AF445B899DE50EF814FB01A7AAED02E3A8A5F311A7`. That is not the contract anchor error.
- `double_claim` stayed a gap in the harness. The committed tx is height 71, code 5, tx `01E17D281AE93D2A7F44D2D0485ADC2FAC111763E520A69296A8518508CF6AAB`, log `invalid headstash proof`. That was before the paying claim, so the batch duplicate was not the error. The later page click is the nullifier duplicate.
- `wrong_address` stayed a gap. Height 75, code 5, tx `AEEEA00E851FF4C21EC561A68B97DC67277811A0FF461FA725B6BFCF22AD0F1B`, log `invalid headstash proof`. Recipient is checked only after verify.

## Gaps

- No on-chain expiry field in `cw-headstash`. The expired-claim case cannot be executed. It will be recorded as a gap, not a pass.
- `cw-reg-eligibility` compares proof bytes. It is not a Halo2 witness. The harness will not treat a byte match as a claim.
- Penumbra chain in ict-rs is a stub. The hop is two local Terp chains, which is the dry-run the claim spec already names.
- `examples/headstash.rs` returns `mock_removed` for `HEADSTASH_CLAIM_MODE=mock`. It is not the e2e entry.
- Recipient check runs only after `proof_instance_verify`, so a wrong address is not a separate error until a proof verifies.
- Round 11 included a paying `ProcessHeadstash`. See that subsection. The earlier `E0277` on `OsRng` was fixed with `rand::rng()` before the prove. Round 10's 200-byte public-input rejection is the run before the 256-byte encoding.
- The Oct 5 wasm takes the `#[cfg(not(feature = "zk-api"))]` branch. `bad_proof` at height 45 is that branch. A host proof cannot verify in that wasm. Round 11 stored the zk-api wasm that passes eight 32-byte scalars to `proof_instance_verify`.
- `store_circuit` is a gap. The first attempt on this boot used `--param-key` and the chain answered `vk param 4608: not found` (tx `7767C3ABC54F08566225E4905E7B59CF7E6CA77079AEDBB708E2EE61C2B6D7B5`, height 39). The rerun used `--k 18 --circuit-type 0 --curve-type 0` and the CLI rejected the body: `param bytes cannot be longer than 33554432 bytes` (`x/wasm/types/tx.go:99`). The params section is 33554500 bytes. RPC `max_body_bytes` was already 67108864, so this was not a 413. There is no chunked store message. The proof was not submitted.
- `anchor_mismatch` inside the recipe failed with sdk code 32, `account sequence mismatch, expected 6, got 5`, tx `528D2DD1D9D1B4B78D3ED097C7BE3E1606C98728967988C9E175AF560C88E6DA`. That is not the contract anchor error. The later page click did get `claim anchor does not match registered root_id 0`.
- `double_claim` and `wrong_address` stayed gaps because the proof gate returned the zk-api error before the nullifier and recipient checks.
- `NewFungible` in the Oct 5 wasm does not mint. The source patch is inside the zk-api wasm and has not been balance-tested. The suite proof is `ExistingFungible` `ibc/ATOM`, so this run does not exercise `NewFungible`.
- Laptop has no Docker. The run is on groot, loopback only.

## Decisions for user

- Work on groot-wan because that is where the suite and Docker are. No GitHub push.
- Image tag for the local chains: `registry.terp.network/terp-core:v6.4.0` already on disk. No pull.
- Chains stay up after a run (`HEADSTASH_E2E_KEEP=1`) so the claim page can query them. Set `HEADSTASH_E2E_KEEP=0` to tear down.
- **Payout under test is `ExistingFungible` of the proof denom `ibc/ATOM`.** The Oct 5 wasm can only pay `uterp` and cannot verify this proof. The zk-api wasm includes the `NewFungible` mint-and-send patch. That arm is not what this suite proof exercises, and it has not been balance-tested. A successful claim for this run is a contract balance drop of `5000000` `ibc/ATOM`.
- The positive proof's recipient is the one `HeadstashCircuitSuite::suite_backed_claim_pair` puts in the instance. The harness does not overwrite it with `user-a`. `user-a` remains the IBC hop account. The claim pays the instance recipient when the proof verifies.
- Local tx size uses the same `config_file_overrides` key the pir live chain already uses: `config/config.toml` with `rpc.max_body_bytes` and `mempool.max_tx_bytes` at 67108864, plus genesis `consensus_params.block.max_bytes` through `modify_genesis`. This is the loopback harness only.

## HUMAN STEPS

groot-wan is up. Load at 15:46 GMT was 2.59 4.31 4.61. Development binds `127.0.0.1` only. From the laptop:

```sh
ssh -N -L 19820:127.0.0.1:19820 -L 55000:127.0.0.1:55000 groot-wan
```

Do not start or rely on `cloudflared` or nginx. `55000` is an example chain publish. The live ports are in `/tmp/headstash-e2e/endpoints.json`.

The prover already wrote `claim_proof.json`. The e2e script reuses that file. The command, if the file is missing, is:

```sh
HEADSTASH_PROVE_ONLY=1 \
HEADSTASH_CLAIM_PROOF=/tmp/headstash-e2e/claim_proof.json \
cargo test -p terp-orch --features headstash --test export_claim_proof -- --ignored --nocapture
```

from `crates/terp-rs`. If that test does not compile, the compiler tail is the gap. Do not substitute a mock proof.

`test-press` stays commented out of `crates/headstash/Cargo.toml` until the `cw-private-dex` git pin for `cw-orch` / `cw-minus` contains `cosmwasm-schema`. The export test does not use `test-press`.

Playwright has to be installed on groot for the click (`playwright` plus its Chromium). The script is `e2e/claim/broadcast.spec.cjs`. A page HTTP 200 is not that click.
