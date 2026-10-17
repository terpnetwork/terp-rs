# Headstash local e2e

Run this on groot-wan, from the terp-core checkout that contains `crates/headstash` and `crates/ict-rs`.

```sh
make headstash-e2e
```

The script is `crates/headstash/scripts/headstash-e2e.sh`. From `crates/headstash`, `just headstash-e2e` calls the same script.

It refuses `ICT_MOCK=1`. It publishes chain ports on `127.0.0.1` only. It uses the Terp and Hermes images already on the host and does not pull. Default image is `registry.terp.network/terp-core:v6.4.0`.

`HEADSTASH_E2E_KEEP=1` leaves the chains up. The lab claim page is `http://127.0.0.1:19820/` on that machine. **Broadcast ProcessHeadstash** posts to the signer on `http://127.0.0.1:19821/`. The click test is `e2e/claim/broadcast.spec.cjs`. An HTTP 200 from the HTML file is not that click.

## Access

Development binds `127.0.0.1` only. Open the page and the chain on groot, or from the laptop through an SSH port-forward:

```sh
ssh -N -L 19820:127.0.0.1:19820 -L 55000:127.0.0.1:55000 groot-wan
```

Development does not use the Cloudflare tunnel. Do not start or rely on `cloudflared` or nginx. Forward each loopback port the laptop must open. The claim page is `19820`, the signer is `19821`, and the chain LCD and RPC are the `127.0.0.1` ports in `/tmp/headstash-e2e/endpoints.json` for that run. `55000` in the example is one of those chain publishes.

Reports: `/tmp/headstash-e2e/report.json` and `endpoints.json`.

The dao-dao claim page reads those endpoints when the URL contains `lab=1`.

See `SPEC.md` for the case list and `SECURITY.md` for the review. Gaps stay gaps: expiry, manifold wasm, PIR, Penumbra, and a host proof that did not compile or did not pay. A mock proof is not a pass.
