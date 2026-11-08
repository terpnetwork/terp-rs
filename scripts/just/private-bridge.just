# scripts/just/private-bridge.just
# Private Bridge corridor lab e2e.
#
# The harness lives at tests/e2e/private-bridge/ (moved out of docs/ — it is an
# executable lab, not documentation). These recipes wrap its entry scripts.
#
# Requires Docker and the zakura node image; the private-dex settle and
# spend-circuit footers live in tests/suite/terp-orch, not here.

lab := "tests/e2e/private-bridge"

# Bring up the corridor lab and run the smoke ladder.
demo-corridor-lab:
    bash {{lab}}/corridor-lab-smoke.sh

# Host prerequisites only, no containers.
demo-corridor-preflight:
    bash {{lab}}/preflight-corridor-local.sh

# Funded variant: mints against a live faucet and observes settlement.
demo-corridor-funded:
    bash {{lab}}/corridor-ict-funded.sh

# Local zakura destination binding.
demo-corridor-zakura:
    bash {{lab}}/zakura/zakura-local.sh

# Prepare the ICT wasm bundle before a funded run.
prepare-corridor-ict-wasm:
    bash {{lab}}/prepare-corridor-ict-wasm.sh

# Full local flow on one host.
demo-corridor-full-local:
    bash {{lab}}/demo-corridor-full-local.sh
