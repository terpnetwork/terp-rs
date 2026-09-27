# terp-rs task index.
#
# Recipes are grouped by concern under scripts/just/. This file is only the
# import list plus the two things worth knowing before you pick a recipe:
#
#   * Host runs are the merge gate. The act runners under scripts/ci/ prove the
#     GHA YAML parses; they do not replace a green host run.
#   * `just test-rs` is the default local gate. `just ci-local` adds fmt, clippy
#     and a full workspace compile.
#
# Run `just --list` for everything, or `just recipes-by-concern` to see which
# file owns which recipe. Add a recipe to the file that owns its concern.

import 'scripts/just/core.just'
import 'scripts/just/ibc-scripts.just'
import 'scripts/just/ci-harness.just'
import 'scripts/just/proto.just'
import 'scripts/just/hash-market.just'
import 'scripts/just/onboard.just'
import 'scripts/just/itl.just'
import 'scripts/just/private-bridge.just'
import 'scripts/just/foundation.just'

# Show which concern file owns which recipe.
#
# `just --list` flattens the imports into one alphabetical list, which hides
# where a recipe lives. This walks the import list rather than the directory,
# so unimported scripts/just/*.just files stay out of it.
#
# Recipe names are intersected against `just --summary` on this merged justfile
# instead of being parsed out of each file. That matters twice over: no single
# concern file is independently parseable (core.just's ci-core depends on
# ibc-scripts.just recipes), and it keeps settings like `ibc := "..."` and
# recipes with defaulted params from being miscounted.
recipes-by-concern:
    #!/usr/bin/env bash
    set -euo pipefail
    # --summary prints one space-separated line; split it for grep -x -F.
    real="$(just --summary | tr ' ' '\n')"
    for f in $(grep -oE "^import '[^']+'" justfile | cut -d"'" -f2); do
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
