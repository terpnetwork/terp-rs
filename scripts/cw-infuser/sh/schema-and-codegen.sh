#!/bin/bash
# Must stay #!/bin/bash (we use a couple of bash features for safety)

set -eu

echo "🔄 Updating schemas for ALL contracts under ./contracts/..."

SCHEMA_OUTPUT_DIR=schema
mkdir -p "$SCHEMA_OUTPUT_DIR"

# ------------------------------------------------------------------
# Helper: move the generated JSON schema
# ------------------------------------------------------------------
move_schema() {
    local contract_dir=$1

    json_file=$(find "$SCHEMA_OUTPUT_DIR" -maxdepth 1 -type f -name '*.json' -print -quit)

    if [ -z "$json_file" ]; then
        echo "⚠️  No JSON schema found after generating for $contract_dir"
        return 1
    fi

    mkdir -p "${contract_dir}/schema"
    mv "$json_file" "${contract_dir}/schema/"
    echo "✅  Moved $(basename "$json_file") → ${contract_dir}/schema/"
}

# ------------------------------------------------------------------
# Auto-discover ALL contracts (POSIX-compatible loop – no more syntax error!)
# ------------------------------------------------------------------
echo "Discovering contracts..."

find contracts -name "Cargo.toml" -type f -print0 | while IFS= read -r -d '' cargo_toml; do
    contract_dir=$(dirname "$cargo_toml")

    # Extract package name from Cargo.toml
    package_name=$(awk -F'"' '/^[[:space:]]*name[[:space:]]*=/ {print $2; exit}' "$cargo_toml")

    if [ -z "$package_name" ]; then
        echo "⚠️  Could not parse package name from $cargo_toml – skipping"
        continue
    fi

    echo "=== Processing $contract_dir (package: $package_name) ==="

    # Generate schema
    cargo schema -p "$package_name" --all-features

    # Move the generated JSON
    move_schema "$contract_dir"
done

echo "🎉 All schemas have been updated and moved to their contract folders."

# ------------------------------------------------------------------
# Generate dynamic CONTRACTS config for TypeScript
# ------------------------------------------------------------------
echo "📝 Generating dynamic CONTRACTS list for TypeScript codegen..."
node scripts/ts/gen-contracts-config.js

# ------------------------------------------------------------------
# Generate TypeScript bindings
# ------------------------------------------------------------------
echo "Generating TypeScript code..."
cd scripts/ts && yarn codegen

echo "✅ Schema update + codegen completed successfully!"

# Vocs MDX + query-first OpenAPI (every IDL under contracts/, vendor/, schema/)
echo "schema-docs (Vocs + OpenAPI)..."
cargo run --release --manifest-path tools/schema-docs/Cargo.toml -- --root "$(pwd)"
