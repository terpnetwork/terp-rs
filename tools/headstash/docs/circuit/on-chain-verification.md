
# On-Chain Verification

## Feature 1: Custom WasmVM

We have designed a fork of Cosmwasm that supports the uploading and use of zk-circuits by smart-contracts in a programmable manner. Smart contracts are now able to provide public instances during proof verification, unlocking useful trust-design primitives for circuits dedicated to this VM layer. To dive more into the custom VM layer we have devloped, checkout the [specification here](https://github.com/permissionlessweb/cosmwasm)

## Feature 2: Vote-Extension + Custom Module

- x/headstash
  - interface with wasm module + vote extensions
  - stored headstash contract proofs recorded by vote-extensions to state, accessable by cosmwasm contract queries
  - calls sudo entrypoint for headstash contract to process claims each block

headstash-api:

- api service performing proof validation and signature aggreagation.

headstsah-sidecar:

- lightweight runtime validators use that communicates with headstash-api aggregates proof claims , includes and
