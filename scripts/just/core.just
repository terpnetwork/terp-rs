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

# Docker-only IBC multihop harness. Ignored tests; see the heavy jobs in ci.yml.
scripts-ibc-harness:
    cargo test -p terp-scripts --test ibc_multihop_harness -- --ignored --nocapture

# Workspace optimizer. Writes artifacts/ and artifacts/checksums.txt.
# arm64 uses the arm64 image tag.
optimize:
    #!/usr/bin/env bash
    set -euo pipefail
    # Local image: tools/optimizer, `make build-arm64-local` (or build-amd64-local).
    # Guest rustc 1.97.1. bob only builds members with a cdylib lib target.
    #
    # Mount path (required, baked into the wasm).
    # rustc embeds absolute source paths in the wasm. The published hash was
    # built with the sources mounted at
    #
    #   /Users/returniflost/abstract/terp-core/crates/terp-rs-consolidate
    #   /Users/returniflost/abstract/terp-core/crates/<sibling>
    #
    # The crates directory is mounted at that prefix, and this checkout is
    # mounted again at terp-rs-consolidate so a tree named terp-rs still
    # compiles under the published path. A /workspace mount changes the hash.
    image=terpnetwork/optimizer:0.17.0-rust1.97
    case "$(uname -m)" in
      arm64|aarch64) image=terpnetwork/optimizer-arm64:0.17.0-rust1.97 ;;
    esac
    src="{{justfile_directory()}}"
    crates_dir="$(dirname "$src")"
    canon_crates=/Users/returniflost/abstract/terp-core/crates
    canon_ws=$canon_crates/terp-rs-consolidate
    test -f "$src/Cargo.toml" || { echo "optimize: no Cargo.toml at $src" >&2; exit 1; }
    docker run --rm \
      -v "$crates_dir:$canon_crates" \
      -v "$src:$canon_ws" \
      -e PROJECT_DIR="$canon_ws" \
      -e CANONICAL_PROJECT_DIR="$canon_ws" \
      -w "$canon_ws" \
      --mount type=volume,source="${OPTIMIZE_TARGET_VOLUME:-terp-rs_cache}",target=/target \
      --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry \
      "$image"
