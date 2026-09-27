# cw-ics08-wasm-terp

08-wasm light client for Terp IAVL hasher modes:

- `hybrid` (default): `ibc` / transfer / ICA / hooks → SHA-256; other stores → BLAKE3
- `blake3`: every IAVL store, including `ibc` (full-BLAKE3 chain A)
- `sha256`: stock ICS-23

Counterparties that only speak 07-tendermint cannot verify a full-BLAKE3 `ibc`
store. They store this bytecode with `tx ibc-wasm store-code` and create an
`08-wasm-*` client whose `VerifyMembership` uses `Blake3IavlSpec` + tendermint
simple merkle (app hash).

Instantiate / sudo / query match ibc-go v11 `types/contract_api.go`.

**Hashing:** `VerifyMembership` is ics23 `apply_leaf` / `apply_inner` (`HashOp`
on each op → `Blake3IavlSpec` vs `IavlSpec`). One hash walk per merkle layer.
This contract does **not** import CosmWasm `deps.api.blake3_256` (portable on a
foreign counterparty). Feature `bench` adds DIY IAVL hashing and a wrong-alg
cross-check for lab/benches; production wasm is built **without** `bench`.

**Relayer-native inner encoding:** `ClientState.data` / `ConsensusState.data` are
ICS-07 Tendermint protobufs so Informal Hermes `--wasm-checksum` (PR #3943 /
#3958) can create/update without a Terp fork. `hasher_mode=auto` selects IAVL
spec from the proof's ICS-23 `HashOp` (`BLAKE3` vs `SHA256`). Lab JSON
`{hasher_mode, root_hex}` still instantiates.

**On-chain store:** `terpd tx ibc-wasm store-code <wasm>` (08-wasm module / gov).
Not `tx wasm store` (x/wasm). Then Hermes
`create client --host-chain C --reference-chain A --wasm-checksum <sha256>`
(or `tx ibc client create` with `/ibc.lightclients.wasm.v1.ClientState`).

**Lab:** `UpdateState` / `VerifyClientMessage` extract app hash + height from a
Tendermint header protobuf (or JSON `{root_hex, revision_height, timestamp_ns}`).
Validator signatures are not checked.

```bash
# From crates/terp-rs — Docker optimizer (rustc 1.86), not host cargo wasm
./scripts/build-ics08-wasm-terp.sh
```

Artifact dest: `crates/terp-rs/artifacts/cw_ics08_wasm_terp.wasm`.
