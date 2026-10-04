#!/bin/sh
# ------------------------------------------------------------------
# Collect one JSON schema per contract into the workspace ./schema dir
# ------------------------------------------------------------------

set -eu                 # abort on error or use of an unset variable
IFS=$(printf ' \n\t')   # default POSIX IFS (space, newline, tab)

SCHEMA_OUTPUT_DIR=schema
mkdir -p "$SCHEMA_OUTPUT_DIR"

# ------------------------------------------------------------------
# Helper: move the first *.json found in $pwd/schema/ to $dest
# ------------------------------------------------------------------
move_schema() {
    dest=$1               # e.g. ./schema/terp721-account.json
    # Find the first JSON file (if any) – stop after the first match
    json_file=$(find schema -maxdepth 1 -type f -name '*.json' -print -quit)

    if [ -z "$json_file" ]; then
        echo "⚠️  No JSON file found in $(pwd)/schema"
        return 1
    fi

    mv "$json_file" "$dest"
    echo "✅  Moved $json_file → $dest"
}

# ------------------------------------------------------------------
# 1️⃣ Specific contracts listed explicitly
# ------------------------------------------------------------------
for contract in terp721-account terp721-account-manifold; do
    echo "=== contracts/$contract ==="
    (
        cargo schema -p $contract --all-features
        move_schema "contracts/$contract/schema/$contract.json"
    )
done

# ------------------------------------------------------------------
# 2️⃣ All contracts under contracts/smart-accounts/
# ------------------------------------------------------------------
for dir in contracts/smart-accounts/*; do
    [ -d "$dir" ] || continue               # skip non‑directories
    name=$(basename "$dir")
    echo "=== $dir ==="
    (
        cd "$dir" || exit 1
        cargo schema
        move_schema "../../../$SCHEMA_OUTPUT_DIR/${name}.json"
    )
done

echo "🎉 All schemas have been collected in ./$SCHEMA_OUTPUT_DIR/"