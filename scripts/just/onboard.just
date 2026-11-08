# scripts/just/onboard.just
# Onboard authz / grant graph (tools/onboard-authz, package terp-onboard-authz).
#
# Also a nested cargo project with its own [workspace]; the crate is embedded in
# the deployed sidecar rather than run as a separate process. See
# tools/hash-market/ONBOARD.md.

onboard := "tools/onboard-authz/Cargo.toml"

# Serve the onboard runtime (grant registration + mint).
onboard-runtime:
    cargo run -p terp-onboard-authz --bin onboard-runtime

# Emit curated grant JSON for terpd to execute.
prepare-authz dao runtime chain="120u-1" out=".oline-passkey-onboard":
    cargo run -p terp-onboard-authz --bin prepare-authz -- \
      --dao {{dao}} --runtime {{runtime}} --chain-id {{chain}} --out {{out}}

# Unit tests plus the in-process keeper simulation.
onboard-test:
    cargo test -p terp-onboard-authz
    cargo run -p terp-onboard-authz --bin local-workflow
