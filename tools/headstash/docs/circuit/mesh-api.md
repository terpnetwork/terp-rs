# Mesh-API

we are tasked to implement a group of new features to our node. These features
  introduce new server api routes, and are described at a high level here[Pasted text
  #1 +111 lines]. lets start with the proof verification. in
  packages/cw-ho/src/headstash/claim.rs ive defined the method that will verify proofs
  and then save them to storage to be processed if valid. we expect ethe proto defined
  MsgClaimHeadstash to be used that carries the proof public inputs and the proof
  itself being verified, and we expect the proof circuiit keys to be accessable in our
  nodes storage. This means headstashes had to have been registered to the node before
  proofs can be claimed. lets implement functions that register a new headsatsh
  instance into our storage node and perform all of the actions needed when a new
  headstash is registered. specifically, we will need to query an ipfs cid that will
  contain the verifying keys and metadata about the headstash, and then save a
  reference to a headstash by the contract addr

 *A Verifiable Service mesh of nodes acting an a proxy for broadcasting proofs and other data.*

- verifiable api powered by WAVS
- opt-in data provisioning for headstash instances metadata (ipfs wrapper compaitible)
- makes use of network and mesh design of ergors nodes for tick-like network consensus on actions.
- keep client server lightweight, define server query and msg as one server in proto file
- out of bounds for wavs runtime verification for cosmos-chain web-socket with rpc node for stateful updates to db (for recording new headstash instances and performing stateful updates when contracts are funded)
- dedicated API for broadcasting claims
- minimizing fee-grants
- delayed claiming support

## WAVS

### Verifiable Proof Verification

Cosmwasm cant perform pallas curve ops on-chain effeciently, so we validate proof in wavs, and then bind wavs-operator set msg to proofs via hashing so we pair a wavs action with a set of stateful events the wavs operator set authorizes each block.

`validate-proofs --> sign-hash of set of nullifier,msg tuples --> verify-in contract --> stateful events`

- mempool that allows for pre-verification/storage of proof verification and nullifier preparation

## Headstash Storage + IPFS Compatible

Node operators are able to enable an ipfs server wrapper around their nodes, and participate in providing storage of the headstash instance metadata files and other extended markets.

- API definition for uploading files to store for headstashes
- API definition for granting users access to upload files to store (default to anyone able to access api)
- feature to pin to ipfs gateway & respond with metadata

new storage layers:

    - headstash metadata classification 
    - pending headstashes to claim 
    - 

### Mesh Consensus

commonware network for node identity and communication

## Vote-Extension Server

*we are going to doing something fun.*

were going to make use of a side-car service that will allow any validator to participate in enhancing the censorship resistant to tx settlement of claiming headsatshes, by operating vote-extension servers that create binding hashes via blake3 of the nullifiers and msgs to be verified on chain much more effeciently.

*because we are using a wavs service, this proof verification can be done off-chain without worry that the verifcation process was corrupted*

### Design

- oracle connected to validator and curates the voteExtension containing sets of nullifiers to submit to headstash contracts
- allows smart-contracts to be designed for async-claiming *(provide proof via vote-extension, claim via authentication channel/manual smart-contract claim)*

### `ExtendVoteHandler`

`ExtendVoteHandler` returns a handler that extends a vote with the sidecars
pending nullifiers to store. In the case where oracle data is unable to be fetched
or correctly marshalled, the handler will return an empty vote extension to
ensure liveness.

```rs
// `abci/ve/vote_extension.go`: `ExtendVoteHandler() `

```

### `VerifyVoteExtensionHandler`

`VerifyVoteExtensionHandler` returns a handler that verifies the vote extension provided by
a validator is valid. In the case when the vote extension is empty, we return ACCEPT. This means
that the validator may have been unable to fetch nullifiers from the oracle and is voting an empty vote extension.

### Lightweight Client Server Interface

### Cosmos-Indexer

- Allows subscription of specific events for cosmos node
- Ensure we are always in sync with latests blocks via peer check of other rpc nodes
- fallback/retry on different peer via reuqesting for previous blocks

### Delayed Write Buffer Market

Allow users to particiapte in enchancing privacy by delay between network call and transaction settlement without compromise integrity of privacy during claim pending

<!-- 
### FeeGrant Market

- x402 api for feegrant provisioning. Non deterministic, programmable per api-node, unless requesting feegrant from authenticator node
- cosmos-sdk cosmwasm MaxCalls feegrant

```
/// MaxCallsLimit limited number of calls to the contract. No funds transferable.
/// Since: wasmd 0.30
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, Copy, PartialEq, ::prost::Message)]
pub struct MaxCallsLimit {
    /// Remaining number that is decremented on each execution
    #[prost(uint64, tag = "1")]
    pub remaining: u64,
}
impl ::prost::Name for MaxCallsLimit {
    const NAME: &'static str = "MaxCallsLimit";
    const PACKAGE: &'static str = "cosmwasm.wasm.v1";
    fn full_name() -> ::prost::alloc::string::String {
        "cosmwasm.wasm.v1.MaxCallsLimit".into()
    }
    fn type_url() -> ::prost::alloc::string::String {
        "/cosmwasm.wasm.v1.MaxCallsLimit".into()
    }
}
``` -->

## commonware_runtime Usage Guide

## Overview

`commonware_runtime` provides configurable async runtimes for executing tasks:

- **Production**: `tokio::Runner` (backed by [Tokio](https://tokio.rs)).
- **Testing/Simulation**: `deterministic::Runner` (deterministic execution with fixed seed; mocks time/network/storage).

**Status**: ALPHA – expect breaking changes.

Key traits:

- [`Runner`]: Starts root tasks synchronously: `runner.start(|ctx| async { ... }) -> Output`.
- [`Spawner`]: Spawns supervised child tasks (abort cascades on parent abort/completion/panic).
- [`Clock`]: Time ops (mockable).
- [`Network`/`Storage`]: I/O (mockable).
- [`Metrics`]: Prometheus integration.

All tasks supervised; panics/aborts propagate.

## Sync Binary with Async Task (e.g., CLI Tools)

Bridge async code (e.g., proving) to sync `main()` **without** `#[tokio::main]` or full Tokio:

### Cargo.toml

```toml
[dependencies]
commonware_runtime = { version = "0.1", features = ["tokio"] }  # Adjust version/path
zk_headstash = { ... }  # Your async crate
```

### src/bin/gen_headstash_circuit.rs

```rust
use std::error::Error;
use zk_headstash::deploy::suite::*;
use commonware_runtime::tokio::Runner;

/// # Create Headstash circuit proof from a note
///
/// Generates default Headstash [VerifyingKey] and [ProvingKey].
///
/// ```bash
/// cargo run --bin gen_headstash_circuit
/// ```
fn main() -> Result<(), Box<dyn Error>> {
    Runner::default().start(|_| HeadstashSuite::new().create_headstash_proof())?;
    Ok(())
}
```

**How it works**:

- `Runner::start<F>(self, f: F) -> Fut::Output` where `F: FnOnce(Context) -> Fut`.
- Injects `Context` (ignore with `|_|` if unused).
- Drives async root task to completion synchronously.
- Returns future's `Output` (e.g., `Result<(), E>` → propagates `?`).

## Advanced Usage

### Spawning Tasks

```rust
Runner::default().start(|ctx| async move {
    let handle = ctx.spawn(|_| async { /* child */ });
    handle.await?;  // Err(Error::Closed) if aborted
});
```

### Shutdown

```rust
let handle = ctx.spawn(|_| async move { /* long task */ });
ctx.stop(9, None).await?;  // Signals all tasks; waits for cleanup
handle.await;  // Closed if not finished
```

### Metrics

```rust
ctx.register("my_counter", "Help", Counter::default());
let metrics = ctx.encode();  // Prometheus text
```

### Testing (Deterministic)

```rust
use commonware_runtime::deterministic::Runner;

// Exact same API; time advances predictably
Runner::default().start(|ctx| async move {
    ctx.sleep(Duration::from_secs(1)).await;
    assert!(ctx.current() >= start + 1s);
});
```

## Full API Docs

Extracted from `lib.rs`:

> [!Execute asynchronous tasks with a configurable scheduler.]
>
> - [Runner], [Spawner], [Clock], [Network], [Storage], [Metrics], [Pacer].
> - Supervision: Children abort on parent exit/panic/abort.
> - Metrics prefix: `runtime_*`.

## Errors

Common: `Error::Exited`, `Closed`, `Timeout`, `Io(...)`.

## Features

- `tokio`: Tokio runtime (non-WASM).
- `deterministic`: Testing runtime.
- `iouring-*`: io_uring storage/network (if enabled).

For source/traits: See crate repo/docs.rs/commonware_runtime.
