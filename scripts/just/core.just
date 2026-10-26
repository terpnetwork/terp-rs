# scripts/just/core.just
# Workspace compile, lint, and the CI gate tiers.
#
# Workflows call these as `just core <recipe>` or `just ibc <recipe>`.

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
ci-core:
    just ibc scripts-ibc-preflight
    just ibc scripts-ibc-offline
    just ibc scripts-ibc-validate
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

# Workspace optimizer. Writes artifacts/ and artifacts/checksums.txt.
# Same mount as docs/COMPILING.md. arm64 uses the arm64 image tag.
optimize:
    #!/usr/bin/env bash
    set -euo pipefail
    # Local image: tools/optimizer, `make build-arm64-local` (or build-amd64-local).
    # Guest rustc 1.91.0; bob only builds members with a cdylib lib target.
    image=terpnetwork/optimizer:0.17.0-rust1.91
    case "$(uname -m)" in
      arm64|aarch64) image=terpnetwork/optimizer-arm64:0.17.0-rust1.91 ;;
    esac
    # justfile_directory() is the root justfile (terp-rs), even inside this module.
    # Mount crates/, the parent of terp-rs, so ../cosmwasm and other sibling path
    # dependencies resolve.
    cd "{{justfile_directory()}}/.."
    test -f terp-rs/Cargo.toml || { echo "optimize: expected terp-rs/ under $(pwd)" >&2; exit 1; }
    docker run --rm \
      -v "$(pwd)":/workspace \
      -e PROJECT_DIR=/workspace/terp-rs \
      --mount type=volume,source="${OPTIMIZE_TARGET_VOLUME:-terp-rs_cache}",target=/target \
      --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry \
      "$image"
