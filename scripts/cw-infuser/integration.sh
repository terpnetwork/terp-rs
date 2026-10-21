#!/bin/sh
# Generate InstantiateMsg JSON (init.json) for each collection under
# scripts/svgs/interchain using the prepare_svg binary.
#
# Per-collection defaults (non-interactive):
#   name   — capitalized folder name + " SVG"  (e.g. "Osmosis SVG")
#   symbol — first 6 chars of folder name uppercased (e.g. "OSMOSI")
#   total  — 10000
#   seed   — folder name (reproducible; blake3-hashed by prepare_svg)
#
# Variable definitions are loaded from spec.json ("vars" field) if present,
# otherwise from the first *.dict.json found in the folder.
# Folders with neither are skipped.

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE="$(cd "$SCRIPT_DIR/../.." && pwd)"
SVG_BASE="$WORKSPACE/scripts/svgs/interchain"
# Price tiers (all collections share the same schedule):
#   mints 1-1000    : 71 THIOL each
#   mints 1001-5000 : 710 THIOL each
#   mints 5001-10000: 1420 THIOL each
PRICE_TIERS='[{"until_count":100,"price":{"denom":"uthiol","amount":"71000000"}},{"until_count":1000,"price":{"denom":"uthiol","amount":"710000000"}},{"until_count":10000,"price":{"denom":"uthiol","amount":"1420000000"}}]'
printf 'Building binaries...\n'
cargo build -p cw-infuser-scripts --bin prepare_svg --bin gen_merkle --manifest-path "$WORKSPACE/Cargo.toml"

BIN="$WORKSPACE/target/debug/prepare_svg"
GEN_MERKLE="$WORKSPACE/target/debug/gen_merkle"
printf 'Generating terp-warriors merkle tree + whitelist init msg...\n'
"$GEN_MERKLE" \
    --csv "$WORKSPACE/data/whitelist.csv" \
    --output "$WORKSPACE/data/terp-warriors.json" \
    --instantiate-output "$WORKSPACE/data/terp-warriors-wl-init.json"
printf 'Whitelist init msg -> data/terp-warriors-wl-init.json\n\n'

for dir in "$SVG_BASE"/*/; do
    collection="$(basename "$dir")"
    template="$dir/template.svg"

    if [ ! -f "$template" ]; then
        printf '[%s] skipping — no template.svg\n' "$collection"
        continue
    fi

    if [ -f "$dir/spec.json" ]; then
        dict="$dir/spec.json"
    else
        dict="$(ls "$dir"/*.dict.json 2>/dev/null | head -1)"
        if [ -z "$dict" ]; then
            printf '[%s] skipping — no spec.json or *.dict.json\n' "$collection"
            continue
        fi
    fi

    # name: capitalize first letter + " SVG"
    first="$(printf '%s' "$collection" | cut -c1 | tr '[:lower:]' '[:upper:]')"
    rest="$(printf '%s' "$collection" | cut -c2-)"
    name="${first}${rest} SVG"

    # symbol: first 6 chars uppercased
    symbol="$(printf '%s' "$collection" | tr '[:lower:]' '[:upper:]' | cut -c1-6)"

    printf '[%s] generating init.json...\n' "$collection"

    "$BIN" \
        --svg "$template" \
        --dict "$dict" \
        --name "$name" \
        --symbol "$symbol" \
        --total 10000 \
        --seed "$collection" \
        --price-tiers "$PRICE_TIERS" \
        --output "$dir/init.json"

    printf '[%s] -> %sinit.json\n' "$collection" "$dir"
done

printf '\nAll collections processed.\n'
