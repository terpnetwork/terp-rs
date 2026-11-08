# Contract page. The server root is this workspace, so /api/ts bundles resolve.
# State files are the mapped set in scripts/web/contracts.py, starting at
# ~/.cw-orchestrator/state.json.

set working-directory := '../..'

# Confirm every catalog bundle exists and exports createApi.
check:
    python3 scripts/web/contracts.py --check

# Serve the page at http://127.0.0.1:4173/web/
serve port="4173":
    python3 scripts/web/contracts.py --serve --port {{port}}
