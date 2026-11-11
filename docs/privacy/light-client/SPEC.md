# Zcash IBC v2 light client — zakura × dynamic FROST committee × hash-market (Workstream B)

Status (2026-11-10): **registry, rotation and aggregator implemented** under
`contracts/light-clients/` (uncommitted). 08-wasm client contract still to write.

## 1. Contract family
| Role (label) | Crate | Status |
|---|---|---|
| `registry` | `cw-lc-registry` | implemented: 7 unit tests |
| `rotation` | `cw-lc-rotation` | implemented: 2 unit tests + integration |
| `aggregator` | `cw-lc-aggregator` | implemented: 2 multi-contract integration tests (`tests/committee.rs`) |
| `client` | 08-wasm IBC v2 client | spec only (§7) |
| `vault` | reserved (TODO §9) | — |

All three: clippy `-D warnings` clean, `wasm32-unknown-unknown` lib check ok, cw-orch `interface` feature.

## 2. Labelling spec (`cw-lc-registry/src/label.rs`)
`terp/lc/<role>/<target>/v<major>`; role ∈ {registry, rotation, aggregator, client, vault};
target = lowercase slug 2–48; major v1..v999 (no leading zero). Each contract rejects a label of another role.
Client ids: slug 3–64 (`zcash-regtest-0`).

## 3. Registry (operators, clients, node state, k-of-n)
- Operators (zakura p2pv2 attestation services) register peer id, endpoint, ed25519 key, capability tags.
  **New:** `weight` (default 1, admin/gov-set via `SetOperatorWeight`; 0 = not eligible for the committee).
- Clients: target chain, attestor set, `min_attestations` (k-of-n). Signed `ReportNodeState`, public queries,
  `ConsensusTip` = highest (height, hash) reported by ≥ k fresh, still-registered attestors.
- **hash-market capability quorum (new, `quorum.rs`).** Config `hash_market_capability {chain_uid, algo, require}`.
  hm-vext `x/hashmerchant` stores a `HashRoot` for a capability only once its `CapabilityQuorum`
  (attesting power ≥ max(threshold, gov floor 0.334), `min_attesters`) is met, so the latest HashRoot *is* the
  on-chain quorum verdict. The registry queries `/terp.hashmerchant.v1.Query/HashRoot` over gRPC (hand-rolled
  protobuf, no prost) and:
  - rejects a report whose tip conflicts with the quorum root at the same height (`QuorumConflict`);
  - withholds `ConsensusTip` on conflict, or when `require = true` and no root is reachable (fail closed);
  - exposes the verdict in `ConsensusTip.quorum` (`Agrees | Conflicts | QuorumBehind | QuorumAhead | Unavailable`) and `QuorumRoot {}`.
  Assumption: for PoW header capabilities the root bytes are the block hash in display order (same as `tip_hash`).

## 4. Dynamic committee (`cw-lc-rotation`)
Membership is never configured by hand. Each epoch snapshots the registry for one client:
eligible = attestor ∧ registered ∧ advertises `preconf` ∧ weight > 0 ∧ reported within `active_window_blocks`.
- **Weights → shares.** `shares = min(weight, max_shares_per_member)`; heaviest `max_members` kept (ties by address),
  then ordered by address; shares are contiguous 1-based ids (= FROST identifiers, TC01 index + 1).
- **Threshold.** `t = ceil(total_shares × threshold_bps / 10000)`, `threshold_bps ∈ [5001, 10000]` (default 6667, 2/3).
  Any valid signature therefore carries a weighted supermajority of the snapshot. Hard rule: no single member may
  hold ≥ t shares (`Concentrated`). `min_members ≥ 2`.
- **Keys reuse existing wire formats.** `FE01` (frost_escrow FROST-Ed25519 group key; verified with the host
  `ed25519_verify` since a FROST group signature is plain Ed25519) or `TC01` (threshold_committee secp256k1 roster;
  ≥ t distinct partials, each via host `secp256k1_verify`). Encoded (t, n) must equal the epoch's (threshold, total_shares).
  `threshold_committee` got a default `lab` feature (OS-rng keygen) so contracts build it rng-free for wasm;
  `frost_escrow` gained `sign_message_with(msg, signer_ids)` for weighted signer subsets.
- **Lifecycle.** `Pending` → `Active` → `Retired | Faulted`; stale pending → `Expired` after `dkg_timeout_blocks`.
  - Proposed by `Bootstrap` (nothing active/pending), `RotateShard` (admin or active member; planned),
    `ReportLivenessFault` (anyone; aggregator reports a stall), `ReportFault` (aggregator only; equivocation).
  - Off-chain DKG/reshare (kms-shard / hash-market `itl_frost_rotate`: membership change = new dealing at an epoch boundary).
  - `AckEpoch{group_key}` by members; `ActivateEpoch` needs acked shares ≥ t for that exact key, a `proof_sig` by the
    new key over the activation digest (proves t live shares), and — for planned rotation — a `handoff_sig` by the
    outgoing key over the same digest. The old key serves until the hand-off lands ("never empty").
  - Activation digest: BLAKE3(len-prefixed `terp/lc/rotation/activate/v1` ‖ chain_id ‖ rotation addr ‖ client_id ‖ epoch ‖ group_key ‖ members_hash).

## 5. Preconfirmation + fallback (`cw-lc-aggregator`)
- terpz-style **proactive** signing: committee members sign every new header commitment as their zakura nodes see it
  (NU7 20–25 s) without a relayer request; anyone submits `SubmitPreconf{epoch, header, signature}`.
  Digest: BLAKE3(`terp/lc/preconf/v1` ‖ chain_id ‖ aggregator ‖ client_id ‖ epoch ‖ height ‖ hash ‖ prev ‖ chain_work).
- Rules: epoch must be the active one; header must link to a same-epoch preconfirmed parent (`BrokenLink`);
  same header again = no-op; an unconfirmed header from an older epoch may be replaced (reorg under the new committee);
  a confirmed one never.
- **Equivocation**: two valid group signatures for different headers at one height in one epoch → evidence stored
  (`Evidence{epoch}`), tx succeeds and calls rotation `ReportFault` → epoch Faulted, next epoch proposed.
- **Confirm{height}**: registry (quorum-checked) k-of-n tip must be ≥ height + `confirm_depth` and the preconf
  chain must hash-link from the tip down to height; heights ≤ tip − depth become `Confirmed`.
- **Liveness**: stalled = active epoch with no preconf for `stall_blocks` Terp blocks (since max(last preconf, activation)).
- **BestHeader**: active ∧ not stalled → latest preconf (`Preconf`); otherwise the registry tip (`RegistryFallback`).
  This is the k-of-n fallback while rotating, faulted or stalled.

## 6. Exercised (tests)
`cw-lc-aggregator/tests/committee.rs` (cw-multi-test, real FROST DKG + TC01 keys):
1. 6 operators (weights 1/1/1/3-without-preconf/2/0) → committee a,b,c,e, 4-of-5 shares; non-member ack, wrong (t,n),
   under-threshold acks all rejected; preconf 100..103 by different weighted subsets; 3-share subset cannot sign;
   broken link rejected; confirm 100..101 at depth 2; equivocation at 103 → epoch faulted, registry fallback, epoch 2 pending.
2. Planned `RotateShard` after a weight change (4-of-4 shares → 4-of-5), hand-off required, FROST → TC01 switch,
   reorg of an unconfirmed epoch-1 header, stall → fallback → `ReportLivenessFault` → epoch faulted → expiry → re-bootstrap.

## 7. IBC v2 client binding (next)
08-wasm client (`ClientKind::Ics08Wasm`) reading aggregator `BestHeader` / `Preconf{height}` status for
`update_state`; `verify_membership` against ZIP-221 / note-commitment roots (zk proofs via `Api::proof_instance_verify`).
`BindIbcClient` links registry client ↔ IBC client id.

## 8. Deployment needs (not done)
- hm-vext: add `/terp.hashmerchant.v1.Query/HashRoot` to the wasm accepted-queries list
  (`app/keepers/acceptedQueries.go` has no hashmerchant entry), and ideally a `CapabilityQuorum` query so contracts can
  read threshold/min_attesters directly. Until then `require = false` degrades to `Unavailable`.
- wasm artifacts need the optimizer (not run).

## 9. TODO ideas (not scheduled)
Encrypted vaults/escrows for bridge clients (`vault` role); proof-powered app-layer state; on/off-ramp registries.

## 10. IBC v2 client + zap-1 payment inclusion (round 6, implemented)
- `cw-lc-zcash` (role `client`, label `terp/lc/client/<target>/v1`) binds `registry`, `aggregator`, `registry_client_id`, `ibc_client_id`, `accept_preconfirmed`.
- Committee headers may carry `payment_root`: the root of a zap-1 Merkle tree of payment events that the zakura nodes saw in that block. It is signed as part of the preconf digest.
- `VerifyPaymentInclusion` verifies a zap-1 leaf + path + IBC v2 `app_data_hash` against that root. It reuses `cw-ics08-wasm-zap1::verify_zap1_ibcv2`. Records are keyed by app_data_hash, with a replay guard on `(source_client, sequence)`.
- `PaymentInclusion {app_data_hash_hex}` → `{inclusion, status}`. The status is live: `Reorged` if the header hash at that height changed. Settlement (A's auctions) releases only on `Confirmed`.
- Reference for a trust-minimised mode: Himess/seam (EVM relay that checks Equihash/difficulty/most-work and uses confirmationWork rather than block counts; RISC Zero shielded-payment proofs, no txid on chain).
