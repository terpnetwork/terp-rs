# scripts/just/hash-market.just
# HashMerchant sidecar (tools/hash-market) and its support CLI.
#
# tools/hash-market is a nested cargo project with its own [workspace], so it
# is excluded from the root workspace and every recipe goes through
# --manifest-path.

hm := "tools/hash-market/Cargo.toml"

# Run the sidecar server.
hashmerchant:
    cargo run -p hash-market-server

# Build the sidecar and run the onboard grant-graph tests.
hashmerchant-test:
    cargo test -p terp-onboard-authz
    cargo build --manifest-path {{hm}} -p hash-market-server

# amd64 even on arm64: the Akash deployment target is amd64.
#
# linux/amd64 image for the o-line `oline hashmerchant deploy` path.
hashmerchant-docker:
    docker build --platform linux/amd64 -t hash-market:local -f tools/hash-market/Dockerfile .
