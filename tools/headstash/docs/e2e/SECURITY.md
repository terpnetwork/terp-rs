# Headstash security notes

Static review of the claim path on groot-wan, 2026-10-12. Runtime rows are filled after `make headstash-e2e`. No mainnet or testnet transaction was sent for this review. No key, mnemonic, or env file was read.

## Findings

### High

1. **Host ports default to every interface.** `ict-rs` `create_container` published `host_ip` `0.0.0.0`. A local chain on a shared machine is then reachable off-box, including its test keyring inside the container. The e2e sets `ICT_PUBLISH_HOST=127.0.0.1` and fails if `docker port` is not loopback. The default for other suites is unchanged, so this remains open outside the harness.

### Medium

2. **Recipient check runs after proof verification.** `rr == i.recp` is after `proof_instance_verify`. A wrong recipient is not rejected until a proof verifies. A proof that verifies for one recipient and is submitted with another `rr` fails closed, but only at the end. Callers cannot observe the recipient error without a verifying proof.

3. **WAVS proof-of-possession does not bind the contract address.** The comment says the signed message is `<contract-addr>-<pubkey>`. `proof_of_ownership` hashes the public key with `bls12_381_hash_to_g2` and checks a pairing. The contract address is stored afterward and is not an input to the hash. A PoP generated against `MockApi` is what the harness submits. Whether terpd 6.4.0 wasmvm accepts that hash-to-curve is an instantiate result, recorded in the run, not assumed.

4. **No claim expiry.** Nothing in `cw-headstash` rejects an old claim. The expired-claim case cannot be executed. This is a product gap, not a passing test.

5. **Eligibility witness is a byte compare.** `cw-reg-eligibility` returns `ok` when the query proof equals `accepted_proof`. Anyone who can read that state can replay the accepted bytes. The harness deploys it so the witness address exists, and it does not treat a match as a claim.

6. **Double-claim is not independently visible without a valid proof.** Batch nullifier uniqueness is inside the per-claim loop, after that claim's proof check. Two copies of a claim whose proof does not verify fail at `circuit_id is 0` (or invalid proof) on the first copy. The `null:` error is not reached. State rolls back, so a failed proof does not store the nullifier. The duplicate-spend guard is real for a verifying proof. This run cannot show it until `store-circuit` and a real Halo2 proof both succeed.

### Low

7. **`NewFungible` in `process_headstash` stores the nullifier and then pays nothing.** The nullifier is written inside the claim loop, before the token arm. The arm is still the commented `add_message` stub in the Oct 5 wasm. A proof that verifies under a new-token strategy consumes the claim and mints no balance. `execute_mint` is a separate owner-only path: it asserts the owner, mints to the contract with `MsgMint`, then `BankMsg::Send`s. The claim arm does not call it. The local patch makes the claim arm follow that same mint-then-send. It is not in the wasm that the harness stores until that wasm is rebuilt. The harness payout under test is `ExistingFungible`.

8. **Oct 5 wasm is the non-`zk-api` build.** The claim error `headstash claim proof verify requires zk-api feature` is only compiled in `#[cfg(not(feature = "zk-api"))]`. That string is in `artifacts/cw_headstash.wasm`, and `circuit_id is 0` is not. Height 45 executed that error. `claim_mock_verify` is not a field on `InstantiateMsg`. A verifying claim needs a wasm rebuilt with `zk-api`.

9. **`BridgeCfg.mock_verify` remains on the bridge messages.** `BridgeMintNote` and `BridgeEgressBurn` can take a mock path. Those messages are not part of this claim harness and are not instantiated as the claim.

### Notes that are closed for this harness

- Empty WAVS set is rejected at instantiate.
- Empty genesis root is rejected.
- `examples/headstash.rs` no longer returns success for `HEADSTASH_CLAIM_MODE=mock`.
- `ICT_MOCK=1` is refused before containers start.
- The static claim page can broadcast `ProcessHeadstash` only through `127.0.0.1:19821`. The signer rejects an LCD or RPC that is not `http://127.0.0.1:`, rejects a contract that is not the harness `terp1` address, and rejects a message that is not `process_headstash`. It passes the key name `deployer` to `terpd` inside the container. It does not read a mnemonic. The `?lab=1` dao-dao page still does not broadcast.
- PIR `POST /notes/:id/pir` is 404 unless hash-market is built with `--features pir`. The harness does not start a stand-in server. A 404 from the static file server is not that gate.
- Community-notes dumps are not used as the genesis root.

## Runtime, first loopback run

Observed on groot before the host became unreachable. These are not guesses.

- Ports were published on `127.0.0.1` only when `ICT_PUBLISH_HOST=127.0.0.1`. The code default is still `0.0.0.0` for every other suite. Finding 1 stays open outside this harness.
- Instantiate with a `MockApi` proof-of-possession was accepted by terpd 6.4.0 wasmvm. Finding 3 is not a chain reject. The contract address is still not an input to the hash.
- `store-vk-param` / `store-circuit` of the 17,877,323-byte blob was rejected by the node RPC: `http: request body too large`. The K=18 circuit is not stored. `circuit_id` stays 0. On-chain `proof_instance_verify` was not exercised.
- `gas auto` on a rejecting `ProcessHeadstash` exits 1 with empty stdout. Stderr carries the contract error. A fixed-gas broadcast of a bad anchor, queried after inclusion, returned `code` 5 and `claim anchor does not match registered root_id 0`. The check is live. The harness parser did not record it on that run.
- The Hermes hop moved 1000 `uterp` to the destination chain and a transfer submitted after `hermes` was stopped did not increase the destination balance.
- The eligibility query with a non-matching proof returned `ok: false`.

## Contract source pass, round 2

Read from the copies of `cw-headstash` `headstash.rs` and `lib.rs` taken off groot. Not a new runtime.

- `process_headstash` order is unchanged: registered root, Poseidon-v1 domain, personal-sign challenge, nullifier, denom, `proof_instance_verify`, then `rr == i.recp`, then the token arm. A wrong recipient is still invisible until a proof verifies.
- WAVS `proof_of_ownership` still hashes the public key and not the contract address. The instantiate comment that names `<contract-addr>-<pubkey>` does not match the hash input.
- `ExecuteMsg::Mint` and `ExecuteMsg::Burn` call `cw_ownable::assert_owner` before tokenfactory messages. They are not an open mint.
- `msg_mint` sets `mint_to_address` to the contract, then the owner mint path sends onward. A claim that used `NewFungible` without the new arm would not send.
- There is still no expiry field.
- The signer is a new local component. It is loopback-bound and message-shaped as above. It is not running, so this is a review of the script, not an observed bind.

## What a later pass still has to show

- On-chain `proof_instance_verify` against the stored K=18 circuit, then a second claim with the same nullifier returning `null:`.
- A verifying proof with `rr` different from `i.recp` returning the recipient error and moving no funds.
- `store-vk-param` / `store-circuit` for the 17MB blob. If the transaction is rejected for size, that stays a gap.
- Hash-market built with `pir`, then `POST /notes/:id/pir` returning 404 when the feature is off and a real response when it is on.
