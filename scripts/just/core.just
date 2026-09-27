# scripts/just/core.just
# Workspace compile, lint, and the CI gate tiers.
#
# These are the recipes the GHA workflows call by name, so a rename here is a
# CI break. Keep names in sync with .github/workflows/.

# --- compile / lint ---------------------------------------------------------

# Compile every workspace target including tests. Heaviest local gate.
check:
    cargo check --workspace --tests

# Fast compile pass over workspace libs only.
check-libs:
    cargo check --workspace --lib

# Tests that need neither Docker nor a live node. The default local gate.
test-rs:
    cargo test --workspace --lib
    cargo test -p terp-scripts --lib --tests

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Full local gate: format, lint, compile, test.
ci-local: fmt-check clippy check test-rs

# --- CI gate tiers ----------------------------------------------------------
# Host-native mirrors of .github/workflows/ci-{core,extended,heavy}.yml. These
# are the merge-authoritative gate; the act runners under scripts/ci/ only
# prove YAML fidelity on top of a green host run.

# Tier 0. Offline IBC pipeline + dense internal lib packs. No Docker.
ci-core: scripts-ibc-preflight scripts-ibc-offline scripts-ibc-validate
    ./scripts/ci/dense-packs.sh core-libs

# Tier 1. Contract lib packs, wasm target checks, non-ignored terp-scripts tests.
ci-extended:
    ./scripts/ci/dense-packs.sh contracts-core
    ./scripts/ci/dense-packs.sh contracts-wasm
    cargo test -p terp-scripts --lib --tests

# Tier 2. Full workspace compile plus the zk contract libs. Maintainer tier.
ci-heavy: check
    ./scripts/ci/dense-packs.sh contracts-heavy

# Docker-only IBC multihop harness. Ignored tests; see ci-heavy.yml.
scripts-ibc-harness:
    cargo test -p terp-scripts --test ibc_multihop_harness -- --ignored --nocapture
