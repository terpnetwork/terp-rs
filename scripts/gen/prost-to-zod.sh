#!/usr/bin/env bash
# prost-to-zod.sh — Parse prost-generated .rs files and emit Zod schemas + TS types.
# Usage: ./scripts/gen/prost-to-zod.sh [--modules terp,osmosis] [--out-dir api/ts/terp-ts/zod]
#
# Reads src/gen/*.rs, extracts #[prost::Message] structs, maps prost field
# annotations to Zod validators, and writes one .ts file per proto package.
#
# Prost field patterns handled:
#   string                → z.string()
#   bool                  → z.boolean()
#   uint64 / int64        → z.string()        (bigint-as-string, Cosmos convention)
#   uint32 / int32        → z.number().int()
#   bytes = "vec"         → z.string()        (base64-encoded)
#   message, optional     → <Ref>Schema.optional()
#   message, repeated     → z.array(<Ref>Schema)
#   string, repeated      → z.array(z.string())
#   enumeration(...)      → z.number().int()  (proto enum wire value)
set -euo pipefail

# ── Config ────────────────────────────────────────────────────────────

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
GEN_DIR="$ROOT/proto/src/gen"
OUT_DIR="${OUT_DIR:-$ROOT/api/ts/terp-ts/zod}"
MODULES="${MODULES:-terp,osmosis}"

# CLI overrides.
while [[ $# -gt 0 ]]; do
  case "$1" in
    --modules)  MODULES="$2"; shift 2 ;;
    --out-dir)  OUT_DIR="$2"; shift 2 ;;
    -h|--help)
      echo "Usage: $0 [--modules terp,osmosis] [--out-dir api/ts/terp-ts/zod]"
      exit 0 ;;
    *) echo "Unknown arg: $1"; exit 1 ;;
  esac
done

mkdir -p "$OUT_DIR"

# ── Prost type → Zod mapping ─────────────────────────────────────────

prost_to_zod() {
  local ptype="$1"
  case "$ptype" in
    string)                echo "z.string()" ;;
    bool)                  echo "z.boolean()" ;;
    uint64|int64|sint64|fixed64|sfixed64)
                           echo "z.string()" ;; # Cosmos: bigints as string
    uint32|int32|sint32|fixed32|sfixed32)
                           echo "z.number().int()" ;;
    float|double)          echo "z.number()" ;;
    'bytes = "vec"')       echo "z.string()" ;; # base64
    *)                     echo "z.unknown()" ;;
  esac
}

# ── Parse a single .rs file → zod .ts ────────────────────────────────

parse_file() {
  local rs_file="$1"
  local basename
  basename="$(basename "$rs_file" .rs)"

  # Derive TS module name: terp.clock.v1 → terp-clock-v1
  local ts_name="${basename//./-}"
  local ts_file="$OUT_DIR/${ts_name}.ts"

  # Extract package from first ::prost::Name impl.
  local package
  package=$(grep -m1 'const PACKAGE' "$rs_file" | sed 's/.*"\(.*\)".*/\1/' || echo "$basename")

  local structs=()
  local current_struct=""
  local current_doc=""
  local in_struct=0
  local fields=""
  local struct_docs=""

  # Collect all struct names for forward-ref resolution.
  local all_structs
  all_structs=$(grep -E '^pub struct [A-Z]\w*' "$rs_file" | sed 's/pub struct \([A-Za-z0-9_]*\).*/\1/')

  # Begin output.
  {
    echo "// Auto-generated from $basename — do not edit."
    echo "// Source: terp-rs/src/gen/$basename.rs"
    echo "// Package: $package"
    echo "import { z } from 'zod';"
    echo ""

    # Track which schemas we emit for the barrel export.
    local schema_names=()

    # State machine: read line by line.
    while IFS= read -r line; do
      # Accumulate doc comments.
      if [[ "$line" =~ ^[[:space:]]*///[[:space:]]*(.*) ]]; then
        local doc_line="${BASH_REMATCH[1]}"
        if [[ -n "$current_doc" ]]; then
          current_doc="$current_doc $doc_line"
        else
          current_doc="$doc_line"
        fi
        continue
      fi

      # Detect struct start: pub struct Foo { or pub struct Foo {}
      if [[ "$line" =~ ^pub[[:space:]]+struct[[:space:]]+([A-Za-z0-9_]+)[[:space:]]*\{ ]]; then
        current_struct="${BASH_REMATCH[1]}"
        struct_docs="$current_doc"
        current_doc=""
        fields=""
        in_struct=1

        # Empty struct: pub struct Foo {}
        if [[ "$line" =~ \{\} ]]; then
          if [[ -n "$struct_docs" ]]; then
            echo "/** $struct_docs */"
          fi
          echo "export const ${current_struct}Schema = z.object({});"
          echo "export type $current_struct = z.infer<typeof ${current_struct}Schema>;"
          echo ""
          schema_names+=("$current_struct")
          in_struct=0
          current_struct=""
          struct_docs=""
        fi
        continue
      fi

      # Inside a struct — parse fields.
      if [[ $in_struct -eq 1 ]]; then
        # End of struct.
        if [[ "$line" =~ ^\} ]]; then
          if [[ -n "$struct_docs" ]]; then
            echo "/** $struct_docs */"
          fi
          echo "export const ${current_struct}Schema = z.object({"
          echo -e "$fields"
          echo "});"
          echo "export type $current_struct = z.infer<typeof ${current_struct}Schema>;"
          echo ""
          schema_names+=("$current_struct")
          in_struct=0
          current_struct=""
          struct_docs=""
          fields=""
          continue
        fi

        # Field doc comment.
        if [[ "$line" =~ ^[[:space:]]*///[[:space:]]*(.*) ]]; then
          current_doc="${BASH_REMATCH[1]}"
          continue
        fi

        # Parse #[prost(...)] + pub field_name: Type
        local prost_re='#\[prost\(([^)]+)\)\]'
        if [[ "$line" =~ $prost_re ]]; then
          local prost_attr="${BASH_REMATCH[1]}"
          # Read next line for the field name.
          IFS= read -r field_line
          if [[ "$field_line" =~ pub[[:space:]]+([a-z_][a-z0-9_]*): ]]; then
            local field_name="${BASH_REMATCH[1]}"
            local zod_type=""

            # Parse prost attribute.
            local is_optional=0
            local is_repeated=0
            local is_message=0
            local is_enum=0
            local prost_type=""

            if [[ "$prost_attr" == *"optional"* ]]; then is_optional=1; fi
            if [[ "$prost_attr" == *"repeated"* ]]; then is_repeated=1; fi
            if [[ "$prost_attr" == *"message"* ]]; then is_message=1; fi
            if [[ "$prost_attr" == *"enumeration"* ]]; then is_enum=1; fi

            # Extract base type.
            if [[ $is_enum -eq 1 ]]; then
              zod_type="z.number().int()"
            elif [[ $is_message -eq 1 ]]; then
              # Resolve reference type from the Rust type on the field line.
              local ref_type=""
              if [[ "$field_line" =~ Option\<([A-Za-z0-9_:]+)\> ]]; then
                ref_type="${BASH_REMATCH[1]}"
              elif [[ "$field_line" =~ Vec\<[[:space:]]*([A-Za-z0-9_:]+) ]]; then
                ref_type="${BASH_REMATCH[1]}"
              fi
              # Strip module path, keep final type name.
              ref_type="${ref_type##*::}"
              # If it's a known local struct, reference its schema.
              # Otherwise use z.unknown() as a cross-module ref.
              if echo "$all_structs" | grep -qx "$ref_type" 2>/dev/null; then
                zod_type="z.lazy(() => ${ref_type}Schema)"
              else
                # Cross-module type — emit as z.unknown() with comment.
                zod_type="z.unknown() /* ${ref_type} */"
              fi
            else
              # Scalar type — extract from prost attr.
              # Handle bytes = "vec" specially.
              if [[ "$prost_attr" == *'bytes = "vec"'* ]]; then
                prost_type='bytes = "vec"'
              else
                # First token in prost attr is the type.
                prost_type=$(echo "$prost_attr" | sed 's/,.*//' | tr -d ' ')
              fi
              zod_type=$(prost_to_zod "$prost_type")
            fi

            # Apply repeated/optional wrappers.
            if [[ $is_repeated -eq 1 ]]; then
              zod_type="z.array($zod_type)"
            elif [[ $is_optional -eq 1 ]]; then
              zod_type="${zod_type}.optional()"
            fi

            # Emit field with doc comment.
            local field_entry="  $field_name: $zod_type,"
            if [[ -n "$current_doc" ]]; then
              field_entry="  /** $current_doc */\n$field_entry"
            fi
            if [[ -n "$fields" ]]; then
              fields="$fields\n$field_entry"
            else
              fields="$field_entry"
            fi
            current_doc=""
          fi
        fi
      fi

      # Reset doc if line is not a doc comment or struct/field.
      if [[ ! "$line" =~ ^[[:space:]]*(///|#\[|pub[[:space:]]) ]]; then
        current_doc=""
      fi
    done < "$rs_file"

    # Barrel export at bottom.
    echo ""
    echo "// ── Package metadata ──────────────────────────────────────────"
    echo "export const PACKAGE = '$package' as const;"
    echo ""
    echo "export const schemas = {"
    for s in "${schema_names[@]}"; do
      echo "  $s: ${s}Schema,"
    done
    echo "} as const;"
  } > "$ts_file"

  echo "  $(printf '%-40s' "$basename") → $ts_name.ts  (${#schema_names[@]} schemas)"
}

# ── Main ──────────────────────────────────────────────────────────────

echo "prost-to-zod: generating Zod schemas from prost types"
echo "  gen dir:  $GEN_DIR"
echo "  out dir:  $OUT_DIR"
echo "  modules:  $MODULES"
echo ""

IFS=',' read -ra MODULE_LIST <<< "$MODULES"
file_count=0
total_schemas=0

for rs_file in "$GEN_DIR"/*.rs; do
  basename="$(basename "$rs_file" .rs)"
  # Filter to requested modules.
  match=0
  for mod in "${MODULE_LIST[@]}"; do
    if [[ "$basename" == "$mod"* ]]; then match=1; break; fi
  done
  [[ $match -eq 0 ]] && continue

  parse_file "$rs_file"
  file_count=$((file_count + 1))
done

# Generate barrel index.ts.
{
  echo "// Auto-generated barrel export — do not edit."
  for rs_file in "$GEN_DIR"/*.rs; do
    basename="$(basename "$rs_file" .rs)"
    match=0
    for mod in "${MODULE_LIST[@]}"; do
      if [[ "$basename" == "$mod"* ]]; then match=1; break; fi
    done
    [[ $match -eq 0 ]] && continue
    ts_name="${basename//./-}"
    echo "export * from './${ts_name}.js';"
  done
} > "$OUT_DIR/index.ts"

echo ""
echo "Done — $file_count files processed → $OUT_DIR/"
echo "  index.ts barrel export written"
