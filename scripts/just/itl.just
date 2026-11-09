# scripts/just/itl.just
# ITL product-plane harnesses.
#
# These reach OUTSIDE this repository into the monorepo sibling
# crates/itl/ and crates/itl-zk-cw-orch/, and into tools/onboard-authz which is
# a nested cargo project with its own [workspace]. Paths are therefore
# monorepo-relative, not repo-relative: expect these to work only from a full
# terp-core checkout.

orch := "tools/onboard-authz/Cargo.toml"
itl := "../../itl"

# Speakeasy multi-device AnyOf authenticators.
itl-auth-test:
    cargo test --manifest-path {{orch}} --test itl_auth_e2e -- --nocapture

# FROST artist payout plane.
itl-frost-test:
    cargo test --manifest-path {{orch}} --test itl_frost_e2e -- --nocapture

# FROST spawn e2e against real containers.
itl-frost-spawn-e2e:
    bash {{itl}}/itl-docs/harness/frost-spawn-e2e.sh

# Cash App .eml nullifier, exercised without a wasmer host.
itl-cashapp-test:
    cargo test --manifest-path {{orch}} --test itl_cashapp_e2e -- --nocapture

# Payment ingest and Halo2 membership consumption.
itl-ingress-test:
    cargo test --manifest-path {{orch}} --test itl_ingress_e2e -- --nocapture

# WASIX parse guest: host only, no net. rustc 1.95.
itl-wasix-test:
    cargo test --manifest-path {{itl}}/cashapp/wasix/guest/Cargo.toml -- --nocapture
    cargo test --manifest-path {{itl}}/cashapp/wasix/host/Cargo.toml -- --nocapture

# Full local ITL e2e.
itl-harness:
    bash {{itl}}/itl-docs/harness/local-e2e.sh
