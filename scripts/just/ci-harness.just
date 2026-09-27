# scripts/just/ci-harness.just
# Local CI harness: the act tiers and the pinned ict-rs binary fetcher.
#
# Host runs are the merge gate. act only proves the GHA YAML parses and runs on
# a real image. Do not merge on act green alone if host is red.
# See scripts/ci/README.md and docs/ci.md.

bins := "/tmp/ict-rs-bins"

# --- act tiers (require `act` + a running Docker) ----------------------------

# Tier 0 host gate, timed, writing runs/latest-host-core.md.
act-host-core:
    ./scripts/ci/run-host-core.sh

# Tier 1. Parse ci-*.yml and list jobs. Seconds, no compile.
act-wire:
    ./scripts/ci/run-wire.sh

# Tier 3. Full act run of ci-core.yml jobs. Long and fragile on Docker Desktop.
act-core:
    ./scripts/ci/run-core.sh

# Same for ci-extended.yml.
act-extended:
    ./scripts/ci/run-extended.sh

# Containers + volumes + prune old run dirs. Safe to run any time.
act-teardown:
    ./scripts/ci/teardown.sh

# The daily loop: clear lag, gate on host, then prove the YAML.
act-exercise: act-teardown act-host-core act-wire

# --- ecosystem combo --------------------------------------------------------

# One ict-rs runtime, many crate surfaces. Offline + in-process.
combo:
    bash scripts/ci/ci-combo.sh

# Adds live LCD DAO code-id checks (ICT_LCD).
combo-live:
    bash scripts/ci/ci-combo.sh --live

# Pull the pinned ict-ci tarball, then spawn chains.
combo-spawn:
    ICT_SPAWN=1 bash scripts/ci/ci-combo.sh --live

# --- pinned ict-rs binaries -------------------------------------------------

# Download the pinned prebuilt ict-rs tarball. No compile. Fails on sha256 mismatch.
fetch-ict-rs dest=bins:
    bash scripts/ci/fetch-ict-rs-bins.sh {{dest}}

# Verify the tarball actually contains every suite in ict-rs-suites.txt.
ci-verify-ict-pin dest=bins:
    bash scripts/ci/verify-ict-rs-tarball.sh {{dest}}

# --- dense pack inspection --------------------------------------------------

# Print pack names and membership, to see what the matrix will run.
ci-dense-list:
    ./scripts/ci/dense-packs.sh list

# Run one named pack (a single GHA matrix cell), e.g. core-libs-a.
ci-core-matrix pack:
    CI_DENSE_MODE=matrix ./scripts/ci/dense-packs.sh pack {{pack}}
