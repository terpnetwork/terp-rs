# zkjwt.passkey.p256.v1 (`zkjwt-wasm`)

Browser / native Halo2 prover for **P-256 ECDSA over the WebAuthn digest**, with public **`msg_bind` bound to the signed challenge** (`challenge == msg_bind` in `clientDataJSON` at the authenticator layer). Circuit ID: `zkjwt.passkey.p256.v1`.

**MSRV 1.86** for published wasm notes / `rust-version`. Native prove/keygen on this crate currently needs **nightly** (`generic_const_exprs` in `halo2-ecc`). **Build, test, and bench only on groot-wan.** Do not cargo/bench this crate on a laptop.

G1 circuit is frozen. Do not rewrite the circuit here.

## Statement

Private witness: P-256 public key, ECDSA `(r,s)`, WebAuthn `authData`, canonical `clientDataJSON`.

The circuit **verifies ECDSA P-256 in-circuit** (`halo2-ecc` secp256r1) against

```
msghash = SHA256(authData || SHA256(clientDataJSON))  as Fq
```

Public `msg_bind` is the 32-byte `compute_msg_bind(chain_id, account, authenticator_id, msg_index, sign_mode_direct)` digest. The host / authenticator must put **that same 32-byte value** in the WebAuthn **challenge** so `challenge == msg_bind`. The Halo2 instances pin `msg_bind` as a public field; flipping it fails verify.

## 128-byte instance layout (one column, 4 × 32B)

Little-endian `Bn256::Fr` bytes, concatenated:

| Offset | Field | Meaning |
|--------|--------|---------|
| 0..32 | `nullifier` | `field(SHA256(b"terp-zkjwt/nf/v1" \|\| SEC1(pk) \|\| msg_bind))` |
| 32..64 | `claim` | `field(SHA256(b"terp-zkjwt/pk/v1" \|\| SEC1_uncompressed(pk)))` |
| 64..96 | `inclusion_set_root` | **constrained `0`** |
| 96..128 | `msg_bind` | `field(BE(msg_bind_32))` |

`k = 17` (`LOOKUP_BITS = 16`).

## How to prove (native / wasm)

Native (groot-wan):

```bash
cd crates/terp-rs/crates/wasm-bindgen/zkjwt-wasm
cargo test --release --offline --lib -- --test-threads=1
cargo run --release --offline --bin bench_zkjwt
```

Rust API: `prove_passkey(&PasskeyWitness)` / `verify_passkey`. Browser: `generate_proof(pk_sec1_hex, r_hex, s_hex, auth_data_hex, client_data_json, msg_bind_hex)` → base64 proof. Pack tx with `encode_auth_payload`.

Keygen (`ParamsKZG` + PK) is process-once (`OnceLock`). First prove includes keygen unless `warmup_keys()` ran.

## `ICT_ZKJWT=1` tests (G2 / ict-rs — not this gate)

Gated e2e lives in **ict-rs TestEnv on groot-wan only**. From repo root on groot:

```bash
ICT_ZKJWT=1 cargo test --release --offline -p ict-rs -- --test-threads=1
# or the suite example that loads terp-zkjwt artifacts:
ICT_ZKJWT=1 cargo run --release --offline -p ict-rs --example suite_wasm_smoke
```

Without `ICT_ZKJWT=1`, those tests must skip. This crate’s unit tests do **not** need the env var.

## Never on-chain

Do **not** put any of the following in CosmWasm / tx / events / public_inputs:

- passkey / WebAuthn **credential_id**
- P-256 **secret scalar** / device private key
- raw **authenticator assertion** (`authData`, `clientDataJSON`, `r`, `s`) as the contract witness
- JWT / OIDC **bearer tokens** or IdP refresh tokens
- browser **PRF / hmac-secret** outputs
- Halo2 **proving key**, **toxic waste**, or unstructured **SRS**
- full **SEC1 pubkey** unless already implied by the public `claim` commitment policy

On-chain: `circuit_id`, 128B instances, Groth16/Halo2 proof bytes, issuer / `claim_commitment` as specified by `terp-zkjwt`.

## Bench

See [BENCH.md](./BENCH.md). Host mandate: **groot-wan**.
