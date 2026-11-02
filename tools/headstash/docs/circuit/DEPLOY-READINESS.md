# Headstash deploy readiness

2026-10-18. Branch `wip/headstash-epoch-claim` in the groot worktree
`crates/terp-rs-headstash-epoch`. This list is what a deploy can rely
on today. A loopback test is not a public-network deploy.

Paths below are in that worktree unless they start with `x/` or `app/`.
Those two live in the terp-core Go tree.

## Proven

### Halo2 claim on one loopback validator

`fa17369` (`fa173699661978d348b4933d3d9687506b64faea`). The paid note is
leaf 0 of a 32,769-note Poseidon-v1 suite tree, suite depth 16, on that
leaf's real path padded to circuit depth 32. Amount 15 uterp. Release
e2e 17 passed / 0 failed. Campaign POST 200. Sudo minted 1. Balance
delta 15 uterp. Replay of the nullifier, wrong context, wrong
destination, wrong amount, a seal before the cutoff, and a root mismatch
left the balance unchanged.

The registry rerun on the same loopback chain also passed 17 / 0.
The paid check calls `do_proof_instance_verify` under vk id 1. That
host call returned code 0 in 1.596s. Sudo minted 1. Balance delta
15 uterp. cw-headstash stores vk id 1 and the SHA-256 of the blob.

- Prover: `tools/headstash/circuits/halo2/src/circuit.rs` (`prove_epoch_claim`, `K = 19`)
- Lane: `tools/headstash/epoch/src/lane.rs`
- Loopback driver: `tools/hash-market/src/bin/headstash-epoch-e2e.rs`
- Script: `tools/hash-market/scripts/headstash-epoch-e2e.sh`
- Contract sudo and cutoff: `contracts/headstash/cw-headstash/src/epoch.rs`
  (`sudo`, `accepted_sudo_pays_the_staged_batch_only_at_the_cutoff`)

The in-proof canonical body, context-scoped nullifier, and lane accept
of `backend=halo2` landed in `a364626`. Amounts are u64 in `ee413fa`.
The earlier flock loopback payout is `02d6352` (16 passed / 0 failed).
Flock is not the epoch prover.

Release timings on this host, `fa17369`: tree 4.475s, keygen 70.334s,
prove 17.915s, replay prove 17.663s, cold self-check verify 44.211s,
proof 4800 bytes.

### Verifying-key cache and the registry claim path

`prepare_halo2_verifier` builds the epoch `VerifyingKey` into a
process-wide `OnceLock`. That cache serves the prover self-check.
Claim checks do not call `Proof::verify`.

Release test `warm_verify_stays_under_one_second`: key build 47.409s,
then one native verify of a real 4800-byte proof in 0.949s. The prover
self-check of the proof and the replay together was 1.079s after the
key existed. Those seconds are native `Proof::verify`.

The claim verifier is a registry inside an embedded `cosmwasm-vm`.
`install_epoch_host` deserializes the epoch blob once under vk id 1
before the process listens. A claim selects that cached key by `vk_id`
through `do_proof_instance_verify`. cw-headstash stores the same id and
the SHA-256 of the whole blob.

On the registry loopback run the vk build was 44.702s. One host verify
of the 4800-byte proof returned code 0 in 1.596s. Tree 4.435s, proving
key 75.699s, prove 18.658s, replay prove 26.326s, and the native
self-check of both proofs 3.647s. A release test registers a second
key, a K=10 offset circuit beside a K=8 equality circuit. Each proof
verifies only under its own id. A swapped proof returns code 1. An
unknown id is rejected.

### Suite-sized prove bench, not a snapshot

`ba0a85b`. `tools/headstash/circuits/halo2/src/snap_bench.rs`
`bench_epoch_depth16_and_real_set`. K=19, circuit depth 32, one shared
keygen 70.390s, proof 4800 bytes.

- 32,769 notes, index 0, suite depth 16: witness 4.454s; prove 15.577 / 17.393 / 15.829s; verify 0.607 / 0.590 / 0.515s
- 51,807 notes, index 1000, suite depth 16: witness 7.061s; prove 16.159 / 17.112 / 16.494s; verify 0.780 / 0.576 / 0.550s

Those leaves come from the suite test generator. They are not a
published eligibility list.

### Quorum fraction 0.667, in the module and in a keeper unit test

`x/hashmerchant/types/params.go` sets the default `quorum_fraction` to
`0.667`. `x/hashmerchant/keeper/vote_ext.go` confirms a root only when
voting power reaches that fraction. `x/hashmerchant/keeper/oracle_test.go`
`TestProcessVoteExtensionsQuorumConfirmsRoot` feeds two votes, power 70
and 30, and stores the majority root. `TestProcessVoteExtensionsRejectsMinorityManipulation`
feeds power 30 against an empty 70 and stores nothing. Both are in
terpd 6.4.0, commit `4d387d5cd738869c22177f2b7240bbe5a6b61550`.

This is two simulated powers in one process. It is not a chain of
several validators. See the gap below.

### Lab binary that already pays through sudo

`build/terpd-v6.4.0-darwin-arm64` reports version 6.4.0, commit
`4d387d5cd738869c22177f2b7240bbe5a6b61550`, build tags
`netgo,ledger,static_wasm,v64`. `app/app.go` at that commit registers
`x/hashmerchant` and the vote-extension handlers. The loopback chain
`headstash-epoch-e2e` (home `/tmp/headstash-epoch-chain`, RPC
`127.0.0.1:36667`) runs that binary. Genesis has
`vote_extensions_enable_height` 2, one gentx, and hashmerchant
`quorum_fraction` `0.667`. `fa17369` is the run where that sudo paid
15 uterp.

## Not proven

### More than one validator

The epoch script collects one gentx
(`tools/hash-market/scripts/headstash-epoch-e2e.sh`). The live genesis
has `gentx_count` 1. One validator always clears 0.667. A chain with
several validator processes, a split vote, and a real 0.667 threshold
has not been run.

### Tier-2 attesting-set gossip and validation

Specified in `tools/headstash/docs/circuit/HASHMARKET-HEADSTASH-INTEGRATION.md`
under "Tier 2 checks" and "Q1. How the trusted set shares proofs":
receipt threshold, amount-class caps, `audit_s` spot-checks, digest
pull, abstain on a partial tree. The campaign body carries
`trusted_verify_threshold`
(`tools/hash-market/src/ve/headstash.rs`). There is no gossip transport,
no second attester process, and no `audit_s` runner. Tier 2 is not
proven.

### ict-rs Docker multi-node e2e

Not run. Docker Desktop on groot-wan is hung and must not be restarted.
No image pull and no compose of an ict-rs headstash network was done
for this branch.

### Real eligibility snapshot

No published eligibility file was loaded. `notes.json` was not an
input. The 32,769-note payout and the 51,807-note bench both use suite
test leaves (`tools/headstash/circuits/halo2/src/suite/suite.rs`,
`generate_test_leaves`). Building the real list, publishing its root,
and paying a member of that list is not proven.

### Campaign funding, and refunds of what is not claimed

Create, register, cutoff, and exact sudo mint are proven on the
loopback (`fa17369`, `register-contract` in the e2e bin, cutoff tests
in `epoch.rs`). The payout minted new tokens. It did not spend a
deposited allocation. `contracts/headstash/cw-headstash/src/tokenfactory.rs`
has `requires_prefund`; the test under it is commented out.
`epoch.rs` has no refund of unclaimed funds. A creator deposit, a
drawdown, and a refund of the remainder are not proven.

### Public chain upgrade

The sudo path exists in the 6.4.0 lab binary above. A network whose
binary does not include `x/hashmerchant` at `4d387d5` needs an upgrade
before that sudo path exists. This checklist does not record the live
public version, and no public upgrade has been run from this branch.

### Claim page, local prover, nginx

Not built. `127.0.0.1:19820` still serves the older static page.
`crates/headstash/e2e/claim/signer.py` still broadcasts
`ProcessHeadstash` for chain `headstash-a-1`. The intended ingress is
one nginx on `127.0.0.1` only, a local prover for keygen and prove, and
`POST /ve/headstash/claims` to the hash-market lane. Nginx was not
started. No page builds a halo2 proof.

### Wasm prover at K=19

Not measured. The host circuit is K=19. Native release proving-key
build on this machine was 75.699s and prove was 18.658s on the registry
rerun (`fa17369` was 70.334s and 17.915s). Nobody has
built that prover for wasm or recorded its memory or time. The guest
serializer in `tools/headstash/circuits/halo2/src/circuit_guest.rs` is
still the older 200-byte layout and is not this epoch verifier.
`getrandom_backend="unsupported"` in the worktree `.cargo/config.toml`
is for the contract wasm build, not a browser prover.

### Security review

Not done. `a364626` changed `circuit.rs`, `claim_auth.rs`, `lane.rs`,
the poseidon gadget, and the integration spec. `epoch.rs` has payout
and cutoff tests. Those tests are not a review of the circuit or of
the contract. No review note exists.

### Penumbra ibc-callbacks bridge

Not run. `tools/headstash/docs/circuit/CLAIM-WORKFLOW.md` describes
post-claim IBC of `uterp` to Penumbra and a shield in Prax, and it
says to dry-run in ict-rs before a live channel. There is no
ibc-callbacks e2e and no Penumbra channel test on this branch.

### Sidecar down, and an epoch that misses quorum

Not run on a chain. The keeper unit test above shows a minority vote
stores no root. Nothing here stops the lane, leaves the vote extension
empty, and shows that a later block does not pay. Nothing here runs an
epoch whose validators stay under 0.667. The one-validator lab chain
cannot show that miss.

## What a deploy would still have to add

A second and third validator, the tier-2 receipt and spot-check path,
the real eligibility root, a funded campaign with a refund of the
unclaimed remainder, the claim page and the local prover behind one
loopback nginx, a decision on wasm versus the local prover after a
real K=19 wasm measurement, a review of `a364626` and `epoch.rs`, and
the Penumbra hop as its own e2e. The 6.4.0 lab binary is the sudo
baseline. A public chain on an older binary needs the upgrade first.
