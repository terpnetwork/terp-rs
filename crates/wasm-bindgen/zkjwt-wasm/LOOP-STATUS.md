# zkjwt.passkey.p256.v1 / flock.v1 loop

**HOST MANDATE:** Build/test/bench/ict **only on groot-wan**.

| Gate | Status | Notes |
|------|--------|-------|
| G1 Circuit | **PASS** | Axiom ECDSA P-256 (halo2-ecc) k=17 still G1-true. **Demo path is Flock** `zkjwt.passkey.flock.v1`: host `prove_flock`/`verify_flock_proof`, footer curve=7 prover=3, BLAKE3(public PI)+passkey-hashed claim. Host `flock_host_roundtrip_no_docker` PASS. |
| G2 ict-rs e2e | **IN PROGRESS (ante)** | Reverted `ExecuteMsg::Authenticate`. Product path: RegisterClaim execute + `tx smartaccount add-authenticator CosmwasmAuthenticatorV1` + a normal account tx with `TxExtension.selected_authenticators` and signature = `ZkJwtAuthPayload` JSON. Ante → sudo `AuthSudoMsg::Authenticate` → `on_auth_request` → `authenticate_payload` / `proof_instance_verify`. `local-zk` digest `52178c07b84b` (curve 7) unchanged. Axiom P-256 passkey-sign still later (curve 6). |
| G3 frontends | pending | Bind/register_claim stay execute. Do **not** call execute authenticate. Authorizing an action is a smart-account tx. Not Axiom passkey-sign. |
| G4 docs+bench | **PASS** (Axiom bench prior) | bench_zkjwt on groot-wan: prove ~2.8s, verify 2ms. Flock host prove/verify sub-ms. |

Last command: rebuild guest wasm on groot-wan (`rustup run 1.86.0`, `-p terp-zkjwt --features zk-host`) then re-run flock G2.
host=groot-wan
Next: G2 PASS only if ante/sudo Authenticate succeeds. A wasm-execute authenticate “PASS” is a fail.

flock e2e compiled: pending rebuild. store/verify/ante-auth: pending.
