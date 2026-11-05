# Validator guide: Headstash sidecar

Status (2026-10-26): the Headstash sidecar path is exercised on local test chains
(4-validator Docker e2e and a loopback lane). There is no mainnet campaign, no
release tag and no published image for the Headstash build yet. On-chain attestor
registration does not exist yet (see `attestor-registry-spec.md`). Items marked
UPCOMING will change.

## What the sidecar does today
- `hash-market-server` runs beside terpd. During ABCI++ ExtendVote, terpd's
  x/hashmerchant keeper GETs `{sidecar-url}/vote-extension`, verifies the custody
  attestation, and returns an **empty extension (abstain)** when the fetch fails
  or the signature does not verify (`x/hashmerchant/keeper/vote_ext.go`).
- Batches are applied when attestations reach `quorum_fraction` (default 0.667,
  `x/hashmerchant/types/params.go`).
- With `--features headstash` the server mounts under `/ve`:
  `POST /ve/headstash/claims` (public), `POST /ve/headstash/campaign` and
  `POST /ve/headstash/seal` (operator-signed: headers `x-hash-market-pubkey`,
  `x-hash-market-signature`; key must be in `operator_pubkeys`). Claims are
  verified natively against the multi-VK registry (vk_id 1 = K=19, vk_id 2 = K=11
  membership); a campaign's `vk_id`/`vk_hash` cannot change once set (409).

## Prerequisites
- Terp validator on a terpd build with x/hashmerchant vote extensions.
- Rust toolchain (stable) on the host. Builds of this repo are offline-sensitive;
  use the lockfile (`--locked`).
- Exactly one nginx for anything public. Do not add a second proxy or tunnel.
- A custody key for signing extensions. TKMS is recommended for mainnet
  (`tools/hash-market/docs/custody.mdx`). Never commit or paste it.

## Build (UPCOMING: release tag)
```sh
cd terp-rs
git checkout wip/headstash-membership-vk   # replace with the release tag when published
cargo build --release --locked -p hash-market --features server,headstash --bin hash-market-server
```
Feature `full-sidecar` = server, ve, blossom, nostr, client, privval; `headstash` adds
ve plus the epoch/claim verifier. This exact command was not re-run for this guide.

## Configure
`config.toml` (start from `tools/hash-market/config.example.toml`):
```toml
bind = "127.0.0.1:9090"     # non-loopback refuses to start unless allow_public_bind
chain_id = "morocco-1"
data_dir = "/var/lib/hash-market"
ve_enabled = true
# signing_key = "..."       # custody; prefer TKMS
# operator_pubkeys = []     # only campaign/seal operators
# [[providers]]             # see docs/sidecar.md
```
Environment variable names (values are yours; never publish them):
- `HASH_MARKET_BIND`: overrides the bind; keep it on `127.0.0.1`.
- `HASH_MARKET_ALLOW_PUBLIC_BIND`: leave **unset**.
- `HEADSTASH_LIE_ROOT`, `HEADSTASH_LAB_LIE`: lab-only fault injection, leave **unset**.
- `RUST_LOG`: e.g. `info`.
- terpd: `HASHMERCHANT_SIDECAR_URL=http://127.0.0.1:9090`, or `app.toml`
  `[hashmerchant] sidecar-url` / `sidecar-timeout` (default 1s). Env wins.
Do not set bridge `mock_verify`.

## Ports
| Bind | Process |
|---|---|
| 127.0.0.1:9090 | hash-market-server (vote extension, `/ve/headstash/*`) |
| 127.0.0.1:9091+ | grpc providers, if configured |
| 127.0.0.1:18521 | headstash-lane (dev/test) |
| 127.0.0.1:19080 | headstash-epoch-e2e listener (dev/test) |
| 127.0.0.1:8791 | headstash-register-relay (dev; CORS for one origin) |
| 127.0.0.1:9444 | hash-market-relay (Nostr stub, not the register route) |

## nginx (your one proxy)
Expose only the public claim route; keep vote-extension, campaign and seal on loopback.
```nginx
location = /ve/headstash/claims {
    limit_except POST { deny all; }
    # allowlist your claim page origin(s)
    proxy_pass http://127.0.0.1:9090;
}
location / { return 404; }
```

## Verify
```sh
curl -s http://127.0.0.1:9090/health
curl -s http://127.0.0.1:9090/providers
journalctl -u terpd | grep -E "sidecar fetch failed|sidecar attestation failed verification"
```
Those two log lines mean your node is abstaining.

## Registering to attest (UPCOMING)
Today the attestor set is the validator set whose sidecars produce valid extensions;
there is no registration step. A proposed on-chain registry is in
`attestor-registry-spec.md` (not implemented).
