# scripts/just/foundation.just
# Foundation DAO stake planning and Infisical-backed secrets.

# Dry-run redelegation plan. Emits JSON, broadcasts nothing.
foundation-stake-plan lcd="https://api.terp.network" delegator="" rank="chaintools,sergo,blockpro,apollo,alxvoy,blackhox" cap="500":
    #!/usr/bin/env bash
    set -euo pipefail
    extra=()
    if [[ -n "{{delegator}}" ]]; then extra+=(--delegator "{{delegator}}"); fi
    cargo run --manifest-path tools/foundation-stake/Cargo.toml -- \
      plan --lcd "{{lcd}}" --rank "{{rank}}" --cap-bps "{{cap}}" "${extra[@]}"

# Infisical-backed mnemonic/SSH helper. No host IP lives in the crate.
infisical-test:
    cargo test -p terp-infisical
