# install-runtime (packaging target)

Planned `hm-support` subcommand for product installs. L3 full mode currently starts
`hash-market` **Docker images** from ICT; this doc is the operator-facing recipe.

## Images

```bash
# Mint / trees host (B4)
docker build -t hash-market:mint \
  -f crates/terp-rs/tools/hash-market/Dockerfile \
  --build-arg FEATURES=server,blossom,client,nostr \
  crates/terp-rs

# Validator sidecar (B5)
docker build -t hash-market:sidecar \
  -f crates/terp-rs/tools/hash-market/Dockerfile \
  --build-arg FEATURES=full-sidecar \
  crates/terp-rs
```

## Run mint

```bash
docker run --rm -p 9090:9090 \
  -v "$PWD/profiles/P-mint.full.toml:/etc/hash-market/config.toml:ro" \
  -v hm-data:/var/lib/hash-market \
  hash-market:mint
curl -sf http://127.0.0.1:9090/health
```

## Run sidecar (lab static VE)

```bash
docker run --rm -p 9090:9090 \
  -v "$PWD/profiles/P-sidecar.full.toml:/etc/hash-market/config.toml:ro" \
  -v hm-side:/var/lib/hash-market \
  hash-market:sidecar
curl -sf http://127.0.0.1:9090/vote-extension   # root alias
curl -sf http://127.0.0.1:9090/ve/vote-extension
```

## terpd

```bash
export HASHMERCHANT_SIDECAR_URL=http://127.0.0.1:9090
# module calls GET $HASHMERCHANT_SIDECAR_URL/vote-extension
```

## ICT

```bash
# Field default (mock_runtime.py — no image build)
ICT_HM_RUNTIME=mock cargo run --example hashmerchant_bootstrap_ubuntu --features docker
# or: just l3-bootstrap-mock / just field-ladder-mock

# Optional full (images must already exist; not collab default)
ICT_HM_RUNTIME=full cargo run --example hashmerchant_bootstrap_ubuntu --features docker
```

Field ladder + residuals: `docs/hashmerchant/FIELD-E2E.md`, `tools/hashmerchant-support/FIELD-RESIDUAL.md`.  
Image pins: `crates/ict-rs/examples/testdata/hashmerchant-bootstrap/IMAGES.md`.
