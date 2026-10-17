#!/bin/bash

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

echo "Updating schema for cw-ave..."
cargo run -p cw-ave --bin schema
mkdir -p contracts/ave/cw-ave/schema contracts/ave/cw-ave-factory/schema
if [[ -f schema/cw-ave.json ]]; then
  mv schema/cw-ave.json contracts/ave/cw-ave/schema/
fi

echo "Updating schema for cw-ave-factory..."
cargo run -p cw-ave-factory --bin schema
if [[ -f schema/cw-ave-factory.json ]]; then
  mv schema/cw-ave-factory.json contracts/ave/cw-ave-factory/schema/
fi

echo "Schema update completed."

cd "$ROOT/tools/cw-ave/ts" && yarn codegen