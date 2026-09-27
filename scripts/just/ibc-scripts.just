# scripts/just/ibc-scripts.just
# The `terp-scripts` IBC info pipeline (package `terp-scripts`, path `tests/`).
#
# These four recipes are called by name from .github/workflows/ci-core.yml and
# ci-heavy.yml. Their contract — exit codes, RunReport JSON, capability modes —
# is documented in skills/tests.md; keep the two in sync.

ibc := "cargo run -p terp-scripts --bin terp-ibc --"

# `offline` needs no env. `live-tx` needs MAIN_MNEMONIC and exits 2 with a JSON
# `missing_env` finding rather than panicking.
#
# Capability preflight for the selected mode.
scripts-ibc-preflight mode="offline":
    {{ibc}} preflight --mode {{mode}} --format json

# The goldens are labelled audited-vs-synthetic; do not update them silently.
#
# Unit + golden tests for the IBC pipeline.
scripts-ibc-unit:
    cargo test -p terp-scripts --lib --test ibc_unit --test ibc_golden

# Serial on purpose: rebuild asserts invariants before writing anything.
#
# Unit/golden, then rebuild the public/ artifacts from existing ibc-data.
scripts-ibc-offline: scripts-ibc-unit
    {{ibc}} rebuild-from-public --format json

# Validate public/ against the hard invariants. Exits non-zero on error findings.
scripts-ibc-validate:
    {{ibc}} validate --format json
