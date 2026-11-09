# Headstash claim — frontend spec

Branch `wip/headstash-epoch-claim` at `fa17369`. Circuit: `tools/headstash/circuits/halo2`, K=19. Native prove on an M1 Pro is about 17s. Browser keygen/params is about 70s the first time.

## Data flow

1. The claim page loads proving params and the proving key once. Cache them in IndexedDB or Cache Storage. Later visits skip keygen. A precomputed params download may replace keygen.
2. The user builds the witness in the page: `rseed`, Merkle path, eligible key, and an ECDSA `personal_sign` over the canonical message. That witness is posted into a dedicated Web Worker. It never goes to the network and never stays on the main thread.
3. The worker runs the Halo2 prover as wasm. Threads use `wasm-bindgen-rayon` and `SharedArrayBuffer`, which require `Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Embedder-Policy: require-corp`. The build is release, `opt-level = 3`, LTO, SIMD. Progress events come back to the page. The main thread stays responsive.
4. The worker returns only `proof_hex`, `message`, and `denom_hex`, plus `vk_id` if a later spec adds it.
5. The page POSTs that body to `/ve/headstash/claims` on the single nginx bound to `127.0.0.1`. Nginx forwards to hash-market at `127.0.0.1:19080`. Dev does not open a Cloudflare tunnel.

Canonical message, `amount` as a decimal u64:

```
headstash-claim-v1:{ctx_hex}:{root_hex}:{nullifier_hex}:{destination}:{amount}
```

Signed with `personal_sign`.

## Security note

The witness (rseed, Merkle path, eligible key, ECDSA signature) is browser-only. Do not log it, do not put it in the POST body, and do not read it from `.env`. The server sees a proof and the public message fields, not the key that produced them. Loopback nginx is the only ingress. A missing COOP/COEP pair means the threaded prover must not start.
