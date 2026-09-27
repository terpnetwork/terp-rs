# polytone-any-proxy

Voice-side dispatcher: **`CosmosMsg::Any`** for IBC circuit execute.

Stock polytone note packets `JSON-CosmosMsg` **without** CosmWasm 2 `Any`. Call this contract with `WasmMsg::Execute` from the ICA, then it emits protobuf `/cosmwasm.wasm.v1.MsgExecuteContract`.

Design: `docs/ibc/POLYTONE-ANY-PROXY.md`.

```bash
cargo test -p polytone-any-proxy
cargo build -p polytone-any-proxy --target wasm32-unknown-unknown --release --lib
# copy wasm to crates/terp-rs/artifacts/polytone_any_proxy.wasm
```

Lab instantiate: `{"unrestricted":true}` so the ICA can `Dispatch` before `SetInstantiator`.

# Polytone proxy + `CosmosMsg::Any` (circuit dispatch over IBC)

**Status:** research / lab  
**Audience:** ict-rs e2e, CosmWasm integrators  
**Not:** a public IBC product. Stock polytone still speaks `JSON-CosmosMsg` without `cosmwasm_2_0` `Any`.

## Why a dedicated proxy

Polytone **note** packets `Vec<CosmosMsg<Empty>>`. Upstream note/proxy are compiled **without** `cosmwasm_2_0`, so `CosmosMsg::Any` is not in the guest enum. `CosmosMsg::Wasm` still works (see ict-rs `examples/polytone.rs`).

Proving circuits on Terp (zk-wasmvm `zk_no_rick`, future headstash/bridge) are invoked as **wasmd** `MsgExecuteContract` (`/cosmwasm.wasm.v1.MsgExecuteContract`). That is a protobuf **Any**, not a JSON `wasm.execute` object.

A dedicated **any-proxy** on the voice chain:

1. Is called by the **stock ICA proxy** via `WasmMsg::Execute` (compatible with existing polytone wasm).
2. Emits `CosmosMsg::Any { type_url, value }` so the host dispatches protobuf the same way a native tx would.
3. Encodes circuit `proove` payloads as `MsgExecuteContract` bytes (sender = ICA proxy).

```text
chain A user
  → polytone note.Execute { msgs: [ WasmMsg::Execute(any_proxy, Dispatch) ] }
  → IBC JSON-CosmosMsg
  → voice instantiates ICA proxy (first call)
  → ICA proxy executes WasmMsg on any-proxy
  → any-proxy submessages CosmosMsg::Any(MsgExecuteContract)
  → zk_no_rick.proove  (host VK verify)
  → optional callback to chain A tester
```

Oracle / mint rules are unchanged: this path **only verifies**. It does not mint from a proof.

## Message surface

`ExecuteMsg::Dispatch { msgs: Vec<AnyMsg> }` where `AnyMsg { type_url, value }`.

Only the **instantiator** (lab deployer or ICA proxy if that address instantiated it) may dispatch. Empty `msgs` acks success (account-create pattern).

Helper: `encode_wasm_execute_any(sender, contract, json_msg) → AnyMsg` with

`type_url = "/cosmwasm.wasm.v1.MsgExecuteContract"`.

## Demo: no-rick over IBC

1. Two Terp zk-wasmvm chains (ict-rs Docker, `local-zk`).
2. Polytone note (A) ↔ voice (B), same handshake as `examples/polytone.rs`.
3. `zk_no_rick` + VK on B (`tx wasm headstash`).
4. `polytone-any-proxy` on B.
5. Note execute on A dispatches Any-wrapped `{ "proove": { "cid": 1, "forbidden": "rick", "proof": "<fixture>" } }`.
6. Query B: proof tx / contract events succeed (`code=0`).

**Honest:** fixture proof must be a real Halo2 proof for the guest VK. Empty / `ADD_` placeholders fail. `mock_verify` is not this path.

## Code

| Piece | Path |
|-------|------|
| Contract | `contracts/ibc/polytone-any-proxy` |
| ict-rs example | `crates/ict-rs/examples/polytone_any_norick.rs` |
| Unit tests | `cargo test -p polytone-any-proxy` |

Do not use stock `polytone_proxy` as `proxy_code_id` for Any packets until note+proxy are rebuilt with `cosmwasm_2_0`.
