# scripts/just/supply-chain.just
# cargo-audit (RustSec) and cargo-vet (https://github.com/mozilla/cargo-vet).
# Roots are listed in scripts/supply-chain/roots.txt:
# this workspace, tools/hash-market, ../cosmwasm, and ../headstash (airdrop).
#
# Module recipes run from scripts/just/. Paths below are from the workspace root.
set working-directory := '../..'

# RustSec advisories. Non-zero when a vulnerability is reported.
audit:
    ./scripts/supply-chain/audit.sh

# Third-party audit gate. Fails when a crate is neither audited nor exempted.
vet:
    ./scripts/supply-chain/vet.sh

# Rewrite imports.lock and exemptions after a Cargo.lock change, then vet.
vet-regenerate:
    ./scripts/supply-chain/vet.sh --regenerate

# Advisory database and the vet gate.
check: audit vet
