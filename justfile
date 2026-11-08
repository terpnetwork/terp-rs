# terp-rs task index.
#
# Recipes are modules under scripts/just/. `just -l` lists modules.
# `just <module> -l` lists that module. `just core test-rs` runs one recipe.
#
#   * Host runs are the merge gate. The act runners under scripts/ci/ prove the
#     GHA YAML parses; they do not replace a green host run.
#   * `just test-rs` is the default local gate. `just ci-local` adds fmt, clippy
#     and a full workspace compile.
#
# Run `just --list` for everything, or `just recipes-by-concern` to see which
# file owns which recipe. Add a recipe to the file that owns its concern.

mod core 'scripts/just/core.just'
mod ibc 'scripts/just/ibc-scripts.just'
mod ci 'scripts/just/ci-harness.just'
mod proto 'scripts/just/proto.just'
mod hashmarket 'scripts/just/hash-market.just'
mod onboard 'scripts/just/onboard.just'
mod itl 'scripts/just/itl.just'
mod bridge 'scripts/just/private-bridge.just'
mod foundation 'scripts/just/foundation.just'
mod docs 'scripts/just/docs.just'
mod web 'scripts/just/web.just'
mod gen 'scripts/just/gen.just'
mod sft 'scripts/just/sft.just'
mod skills 'scripts/just/skills.just'
mod supply 'scripts/just/supply-chain.just'

# Which module file owns which recipe.
recipes-by-concern:
    #!/usr/bin/env bash
    set -euo pipefail
    # --summary prints one space-separated line; split it for grep -x -F.
    real="$(just --summary | tr ' ' '\n')"
    for f in $(grep -oE "^mod [^ ]+ '[^']+'" justfile | sed "s/.*'\\(.*\\)'/\\1/"); do
      echo "-- ${f}"
      # Col-0 identifiers in file order, colon stripped, filtered to real
      # recipes. Comment and body lines drop out at the -xF match.
      awk '{print $1}' "$f" | sed 's/:$//' \
        | grep -xF "$real" | awk '!seen[$0]++' | sed 's/^/    just /'
    done

# Validate the changelog fragment this PR owns. See skills/pr-changelog/SKILL.md.
check-changelog:
    ./scripts/ci/check-changelog.sh

# Write the fragment skeleton for a PR number, then edit it.
changelog-scaffold pr:
    ./scripts/ci/check-changelog.sh --scaffold {{pr}}

# Hidden spellings of the names workflows and docs already call.
# `just -l` shows the modules; these stay callable.

[private]
check:
    just core check

[private]
check-libs:
    just core check-libs

[private]
test-rs:
    just core test-rs

[private]
fmt:
    just core fmt

[private]
fmt-check:
    just core fmt-check

[private]
clippy:
    just core clippy

[private]
ci-local:
    just core ci-local

[private]
ci-core:
    just core ci-core

[private]
ci-extended:
    just core ci-extended

[private]
ci-heavy:
    just core ci-heavy

[private]
scripts-ibc-harness:
    just core scripts-ibc-harness

[private]
optimize:
    just core optimize

[private]
audit:
    just supply audit

[private]
vet:
    just supply vet

[private]
vet-regenerate:
    just supply vet-regenerate

[private]
scripts-ibc-preflight mode="offline":
    just ibc scripts-ibc-preflight {{mode}}

[private]
scripts-ibc-unit:
    just ibc scripts-ibc-unit

[private]
scripts-ibc-offline:
    just ibc scripts-ibc-offline

[private]
scripts-ibc-validate:
    just ibc scripts-ibc-validate

# Rust dependency graph JSON (shortcut for `just gen deps`). Relative paths are
# from the terp-rs root.
gen-deps out="target/gen-deps/rust-crates.json" *args="":
    just gen deps {{out}} {{args}}
