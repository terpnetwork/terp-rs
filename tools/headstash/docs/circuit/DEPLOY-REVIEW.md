# Deploy review, wip/headstash-epoch-claim

Read of `c8fd4ecd23de454fee49301d593ca011ffa8a134` on 2026-10-18.
Parent is `a221190`. This is a source read. It does not re-run the
loopback e2e, the two-key host test, or `headstash-halo2-failure-modes.sh`.
That script stays with p3.

The host method on this branch is `Api::proof_instance_verify`, exported
as `do_proof_instance_verify`. A search of the epoch worktree does not
find the symbol `proof_instance_verification`.

## What already holds

**Registry.** `VmVerifier::register` in `tools/hash-market/src/ve/vm_verify.rs`
refuses `vk_id` 0, refuses an id above `u32::MAX`, and refuses an id
already in the map. The only production caller is `install_epoch_host`,
which registers the compiled epoch blob under id 1 and requires the
SHA-256 of that blob to equal `epoch_vk_hash()` of the same blob. A
second call keeps the first host. There is no HTTP route that uploads a
blob. The circuit querier answers a missing id with `NoSuchCircuit`, so
a miss cannot cold-load a different key.

**vk_id on the campaign.** `validate_campaign` in
`contracts/headstash/cw-headstash/src/epoch.rs` rejects a halo2 campaign
with `vk_id` 0 and a `vk_hash` that is not 32 bytes. `Campaign::open` in
`tools/headstash/epoch/src/lane.rs` rejects halo2 `vk_id` 0.
`vk_from_body` rejects halo2 `vk_id` 0 and a hash that is not 64 hex
characters. `install_campaign` then requires that hash to equal
`halo2_vk_hash(vk_id)` of the cache. A second `POST /ve/headstash/campaign`
returns `409` `vk is fixed` when the id or the hash would change, and
`409` `campaign already has claims` once a digest is stored.

**Claim check order.** `evaluate_halo2` returns `IngressError::Vk` when
the claim's `vk_id` is not the campaign's, before `verifies` increments.
A campaign hash other than 32 zero bytes must equal the cached hash for
that id, also before the counter moves. The host call is
`verify_with_host` into `do_proof_instance_verify`. `Ok(0)` accepts,
`Ok(1)` rejects. `settle` does not call this path.

**Listeners on the epoch path.**

| Process | Bind |
|---|---|
| `hash-market-server` | Default `127.0.0.1:9090`. `inspect_bind` refuses `0.0.0.0`, `[::]`, and any other non-loopback host unless `allow_public_bind` or `HASH_MARKET_ALLOW_PUBLIC_BIND` is `1`, `true`, or `yes`. The override is logged. |
| `headstash-lane` | Default `127.0.0.1:18521`. A `HASH_MARKET_BIND` that does not start with `127.0.0.1:` is refused. |
| `headstash-epoch-e2e` | Default `127.0.0.1:19080`. The same prefix check, then `local.ip().is_loopback()` on the socket. |
| frost inbox inside the server | `loopback_bind` accepts only `127.0.0.1`. |
| `hash-market-relay` | Default `127.0.0.1:9444` (`c8fd4ec`). Same `inspect_bind` opt-in as the server. The process does not open a socket yet. The guard runs before it waits. |
| `headstash-halo2-failure-modes.sh` | Sets `HASH_MARKET_BIND` to `127.0.0.1` and `ICT_PUBLISH_HOST=127.0.0.1`. A node that is not `tcp://127.0.0.1` exits. Not edited here. |

**Secrets in the epoch bins.** No `.env` or seed file is committed under
`contracts/headstash`, `tools/headstash`, or `tools/hash-market`. The e2e
reads `CUSTODY_SEED_FILE`, requires 32 bytes, and zeroes that buffer
after `SigningKey::from_bytes`. On a bad length it prints the length.
The lane prints `lane_up` and `prove_ok bytes`. It writes `vk.hash` and
`custody.pub` under `HASH_MARKET_DATA` and does not print either. Host
lines are `epoch-claim host_verify vk_id … code …` and
`epoch-claim host_err` plus the VM error string.

**Epoch payout is not the lab mock.** `settle` pays from the staged rows
when the stored `headstash-accepted` root matches and the stored height
is at or past `cutoff_height`. It does not read `BridgeCfg.mock_verify`.
`ProcessHeadstash` is a separate execute path. Without the `zk-api`
feature it returns an error. With `zk-api` it calls
`api.proof_instance_verify`. The comment in that function still names
`claim_mock_verify`; the code does not consult that flag.

## Gaps still open

1. **The chain does not check the verifying key at payout.** `sudo` stores
   the root and the height after checking `chain_uid` and the algo name.
   `settle` never reads `vk_id` or `vk_hash`. A campaign can record one
   hash while the sidecar cache holds another. The sudo still pays when
   the attestation matches the staged batch. The contract owner can set
   any non-zero halo2 id and any 32-byte hash until a batch is staged or
   the epoch is marked paid. There is no on-chain list of blobs and no
   admin of one. The registry admin is whoever starts the process.

2. **`/ve/headstash/campaign`, `/claims`, and `/seal` have no authenticator.**
   Any client that can open the port can install a campaign whose hash
   matches the cache, submit claims, and seal. Loopback is the control.
   `allow_public_bind` or `HASH_MARKET_ALLOW_PUBLIC_BIND` on
   `hash-market-server` publishes those routes with the rest of the
   router. A second install with the same vk can still replace the
   allocation root, the cutoff, and the amount caps until the first
   claim is stored.

3. **A zero `vk_hash` skips the cache compare inside `evaluate_halo2`.**
   The comment marks this as the in-process fixture. HTTP halo2 install
   cannot store that hash, because install requires equality with the
   loaded blob. Any other caller of `Campaign::open` can. Production
   sidecars should not construct a halo2 campaign with a zero hash.

4. **`seal` flips the batch root when `HEADSTASH_LIE_ROOT` and
   `HEADSTASH_LAB_LIE` are both exactly `1`.** A production environment
   has to leave both unset. One flag alone does not flip the root.

5. **One server listener is not covered by `inspect_bind`.** A state-root
   provider with `mode = "grpc"` binds `provider.address` as given
   (`tools/hash-market/src/bin/server.rs`). Example configs still show
   `0.0.0.0` for that address. The epoch lane and the e2e do not start
   this transport. `headstash-lane` also does not re-check
   `local.ip().is_loopback()` after `TcpListener::bind`. The e2e does.
   An opted-in relay bind of `0.0.0.0` is still advertised as
   `ws://0.0.0.0:9444`. The `wss` URL rewrites that host to `localhost`.

6. **Lab verify remains in the same crate, off the epoch settle path.**
   Owner `SetBridgeCfg` with `mock_verify` makes bridge and egress accept
   a non-empty proof (`cfg!(test)` does the same inside unit tests; a
   release wasm is not built with `cfg(test)`). test-press corridor bins
   still default that lab flag for other products. Deploy of this wasm
   should not send `SetBridgeCfg` with `mock_verify`. The embedded host
   in `vm_verify.rs` is built on `cosmwasm_vm::testing::{MockApi,
   MockStorage, MockQuerier}`. Those types are the in-process VM shell.
   The accept/reject decision is `do_proof_instance_verify`.

7. **The Rust failure-mode model does not sweep.**
   `tools/hash-market/src/bin/headstash-failure-modes.rs` sets
   `campaign_open = false`, `unclaimed = 3`, and `sweep = false`, then
   prints `unclaimed_funds_after_campaign_end`. It does not execute
   `SweepUnclaimed`. The chain script is the one in `a221190`. This
   review did not run it.

8. **The live epoch wasm is the pre-sweep file.**
   `/tmp/headstash-epoch-chain/cw_headstash.wasm` is 896512 bytes,
   sha256 `99af6ea3a583d22fb731039afb07da16c3a1399b7c2d44dcf638ab5777a29e93`.
   It does not contain `sweep_unclaimed`. The sweep build is
   `/tmp/cw_headstash-sweep.wasm`, 902068 bytes, sha256
   `b1c3455a74df5fdeedaf344499f5f4fff46045278d65ae86245772bb82815740`.
   Storing it means a new code id. The running epoch chain should not
   be re-inited to do that.

9. **Logs are filtered by phrase, and two public files are mode-default.**
   `scrub` in the e2e drops lines that contain `mnemonic`, `priv_key`,
   or `private key`, drops a 12-word lowercase line, and keeps eight
   lines of at most 400 characters. A hex secret in command stderr would
   pass that filter. The e2e writes `instantiate.json`, `campaign.json`,
   and `stage.json` under the chain home. `campaign.json` carries the
   vk hash and the custody public key. The lane's `vk.hash` and
   `custody.pub` are created with the process umask, not an explicit
   `0600`.

10. **The epoch circuit still uses halo2-base for secp256k1 field types.**
    The uses are in `circuits/halo2/src/circuit/gadget/secp256k1_chip.rs`
    and `circuits/halo2/src/suite/suite.rs`, behind host-crypto. The
    k256 replacement is not on this commit. Cherry-pick stays deferred
    until that removal is committed on its own branch.

11. **`tests/itl_frost_notes_live.rs` writes a regtest config that listens
    on `0.0.0.0` and sets cookie auth off.** The epoch scripts do not
    start it. It is outside the claim path and should not be run for
    this deploy.

## Before a deploy

Leave `HASH_MARKET_ALLOW_PUBLIC_BIND` unset and do not set
`allow_public_bind`. Leave both lab lie variables unset. Do not set
bridge `mock_verify`. Do not add a grpc provider on a public address.
Store the sweep wasm as a new code id when the target chain should
sweep, and point the campaign `vk_hash` at the sidecar's
`install_epoch_host` output. Treat gaps 1 and 2 as the ones that still
need a design decision: chain-side vk enforcement, and authentication
on the three `/ve/headstash` routes, if those routes will ever be
reachable beyond the host.
