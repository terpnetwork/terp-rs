# terp-beacon

A CosmWasm drand beacon for Terp raffles. It verifies League of Entropy beacons in-contract and speaks the same request and callback JSON as nois-proxy, so a raffle contract can point `nois_proxy_addr` here without a code change.

Randomness for a round is `sha256(drand_signature)`, the same derivation drand and nois use.

## Schemes

One contract instance is pinned to one drand chain at instantiate.

| Scheme id | Network | Public key | Signature | Previous signature |
| --- | --- | --- | --- | --- |
| `pedersen-bls-chained` | mainnet `default` | 48-byte G1 | 96-byte G2 | required, or loaded from stored round-1 |
| `bls-unchained-g1-rfc9380` | quicknet | 96-byte G2 | 48-byte G1 | must be omitted |

Chain parameters (hash, pubkey, genesis, period) come from `https://api.drand.sh/<chain>/info`.

## Raffles wiring

1. Instantiate this contract with the drand chain you want (quicknet is the 3 second beacon).
2. Set the raffle's `nois_proxy_addr` to this contract.
3. Set `nois_proxy_coin` to a coin this contract lists in `prices`. An empty price list means the request is free and any coins the raffle attaches are accepted. A raffle always attaches `nois_proxy_coin`, so that denom must be one the raffle contract can pay.
4. Leave the raffle job id as `raffle-{id}`. Nothing else in the raffle message changes.

Raffle creation sends `get_randomness_after` with `after` set to the raffle end plus the raffle contract's nois timeout. Selling out, or `update_randomness`, sends `get_next_randomness`. For that message, `after` is the block time plus 5 seconds (the same margin nois-proxy uses).

The request commits to one drand round, the same way nois `round_after` does: round 1 when `after` is before genesis, otherwise the first round whose publish time `genesis + (round - 1) * period` is strictly after `after`. That round is stored on the job and is the queue key. Delivery loads that beacon by key and sends its randomness only. A later round does not fill a job that is still waiting on an earlier one. The round must be verified, and registered attestations must reach `required_attestations` (default 0).

`add_round` and `attest` deliver up to 10 jobs committed to that round. `process_queue` continues the backlog, at most 10 rounds and 10 jobs in the transaction. If the committed round is already stored and deliverable, the request delivers in the same transaction.

The callback is a wasm execute on the requester:

```json
{
  "nois_receive": {
    "callback": {
      "job_id": "raffle-0",
      "published": "1793769897000000000",
      "randomness": "e8909f9607b19b365f4dfcc02683151e8c752f482c93299b103263836a3f6755"
    }
  }
}
```

`published` is the drand round time in nanoseconds, not the chain block time. `randomness` is 32 bytes hex.

## Attestors

Admin registers compressed secp256k1 public keys. Anyone may submit a signature from a registered key. The signed message is the 32-byte hash:

```text
SHA-256( ASCII("terp-beacon/attestor/v1\n") || round as 8-byte big-endian || randomness bytes )
```

The signature is 64-byte compact `r || s` (not DER). Removing a key drops it out of the attestation count. Keys that remain registered must still meet `required_attestations` before a round is delivered.

## Extension point

Beacon rows are stored under `(source_id, round)`.

| Source | Id | Status |
| --- | --- | --- |
| Drand | 1 | verified rounds |
| Attestor | 2 | signatures over a drand round |
| IBC | 3 | reserved, not written |

A later IBC v2 client to a hash-market runtime should verify the packet, then write the same beacon record the drand path writes, using source id 3. No IBC entry point is implemented.

## Instantiate

Quicknet, free requests, drand-only delivery:

```json
{
  "admin": "stars1admin",
  "chain_hash": "52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971",
  "pubkey": "83cf0f2896adee7eb8b5f01fcad3912212c437e0073e911fb90022d3e760183c8c4b450b6a0a6c3ac6a5776a2d1064510d1fec758c921cc22b0e17e63aaf4bcb5ed66304de9cf809bd274ca73bab4af5a6e9c76a4bc09e76eae8991ef5ece45a",
  "genesis_time": "1692803367000000000",
  "period_seconds": 3,
  "scheme": "bls-unchained-g1-rfc9380",
  "required_attestations": 0,
  "prices": [],
  "callback_gas_limit": 500000
}
```

`prices`, `required_attestations`, and `callback_gas_limit` may be omitted. Default prices are empty (free). Default attestations are 0. Default callback gas is 500000.

To charge the same fee the raffle tests use:

```json
"prices": [{ "denom": "unois", "amount": "500000" }]
```

## Execute

Submit a quicknet round. `previous_signature` may be omitted.

```json
{
  "add_round": {
    "round": 33655511,
    "signature": "8cbec8c08a98c04fd0d6215df7ace810e7bca69d30e92eb2fe3b14a3b29b061d599e5cc98ed894d5a4628646cbdbbaaa"
  }
}
```

Submit a chained mainnet round. `previous_signature` is the previous round's signature hex.

```json
{
  "add_round": {
    "round": 6611295,
    "signature": "b09f499631ee78b28d65fa2fbeb1face1574c7167d2c696a9969b294867e973db82d5a3c19f4279fed884a202050edf6038bb3c1fbbd3d5a92ae3eb9f55f4c4eea8af329ba6c3804cd481b865d7fe3da242e5176db4a23700a02c8a28049a297",
    "previous_signature": "884fdec24063045af3845f86fd39a5770fd8e6d70649bdfa746dcbef5065521f82e236d4c355ea2baf3e2febacecde22149f9d54b1f71b8190638437ea4987ef574125ed9f2ad28d70a6de5c07d1827b71a6f4772b9206ab2c2558f5d5a8ed37"
  }
}
```

Unverified or tampered signatures return `InvalidSignature` and nothing is stored.

```json
{ "add_attestor": { "pubkey": "02aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" } }
```

```json
{ "remove_attestor": { "pubkey": "02aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" } }
```

```json
{
  "attest": {
    "round": 33655511,
    "pubkey": "02aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "signature": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  }
}
```

```json
{ "transfer_admin": { "new_admin": "stars1next" } }
```

```json
{
  "update_config": {
    "required_attestations": 1,
    "prices": [{ "denom": "unois", "amount": "500000" }],
    "callback_gas_limit": 800000
  }
}
```

```json
{ "withdraw": { "recipient": "stars1admin", "denom": "unois" } }
```

```json
{ "process_queue": {} }
```

Raffle request, identical to nois-proxy:

```json
{ "get_next_randomness": { "job_id": "raffle-0" } }
```

```json
{
  "get_randomness_after": {
    "after": "1700000000000000000",
    "job_id": "raffle-0"
  }
}
```

Job ids are at most 64 bytes. A second request with the same requester and job id, while the first is still queued, is rejected.

## Queries

```json
{ "config": {} }
```

```json
{ "beacon": { "round": 33655511 } }
```

```json
{ "attestors": {} }
```

```json
{ "attestations": { "round": 33655511 } }
```

```json
{ "job": { "requester": "stars1raffle", "job_id": "raffle-0" } }
```

```json
{
  "job": {
    "requester": "stars1raffle",
    "job_id": "raffle-0",
    "after": "1793769894000000000",
    "round": 33655511
  }
}
```

`job.round` is the committed drand round. `beacon.deliverable` is true when that round is verified and the attestation threshold is met. `config.source_ids.reserved_ibc` is `3`.

## terp-rs integration

`nftfi-raffle` uses the beacon through `RandomnessSourceMsg::Callback { verifier }`. `RequestDraw` sends `ExecuteMsg::Provide { raffle_id, owner }`; the beacon queries the raffle for its end time, picks the first quicknet round published strictly after it, and answers with the `raffle_id`, `round`, `randomness` and `owner` attributes the raffle reply reads. `Provide` fails until that round has been added (and attested, when `required_attestations > 0`).

- cw-orch interface: `terp_beacon::TerpBeacon` (feature `interface`).
- Suite: `terp_orch::suites::nftfi::NftfiSuite::deploy_on` uploads and instantiates the beacon, then instantiates the raffle with `Callback { verifier: beacon }`.
- E2E: `tests/tests/nftfi_flows.rs::raffle_draw_with_quicknet_beacon` (quicknet round 33655511).
- Deploy: `tests/suite/terp-orch/src/bin/terp_beacon_deploy.rs` (cw-orch-kms, dry run by default).
- Schema: `cargo run -p terp-beacon --bin schema` writes `schema/`.

### Gas

`cargo test -p terp-beacon --features fixtures --test gas -- --nocapture`. KV schedule from `contracts/nfts/raffle/tests/gas.rs`; BLS pairing CPU is charged by the chain's bls12_381 host functions and is not in these figures.

| action | kv gas |
|---|---:|
| instantiate (quicknet) | 38902 |
| add_attestor | 12836 |
| update_config | 26167 |
| add_round (quicknet 33655511) | 20300 |

## Attribution

This contract is derived from, and stays wire-compatible with, the Nois contracts:

- [noislabs/nois-contracts](https://github.com/noislabs/nois-contracts), in particular `nois-drand` (round verification and submission) and `nois-proxy` (the `GetNextRandomness` / `GetRandomnessAfter` request and `NoisReceive` callback JSON, `round_after`, and the 5 second safety margin).
- [noislabs/drand-verify](https://github.com/noislabs/drand-verify) performs the BLS12-381 drand signature verification (`drand-verify` crate).

Both are Apache-2.0. Credit for the protocol design and verification code belongs to the Nois authors.

