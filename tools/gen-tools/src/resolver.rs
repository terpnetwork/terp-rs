//! Source-aware type resolution for the gen-tools pipeline.
//!
//! The [`SourceResolver`] discovers available type definitions across a workspace
//! and resolves them into a canonical `HashMap<String, serde_json::Value>` format
//! that all generators already understand.
//!
//! # Supported source types
//!
//! | Source      | Priority | Description                                  |
//! |-------------|----------|----------------------------------------------|
//! | Schemas     | 1 (best) | CosmWasm `cargo schema` JSON output          |
//! | Protos      | 2        | `.proto` definition files                    |
//! | Generated   | 3        | Previously generated types in output dirs    |
//!
//! Each generator calls `resolve()` instead of independently scanning for
//! `schema/` directories. This allows the pipeline to work from ANY
//! supported type definition starting point — not just CosmWasm schemas.

use crate::config::GenerationContext;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Type source detection
// ---------------------------------------------------------------------------

/// Available type sources for a workspace, ordered by priority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeSource {
    /// CosmWasm JSON schemas from `cargo schema`.
    /// Each entry is (contract_name, schema_dir_path).
    Schemas(Vec<(String, PathBuf)>),
    /// Protobuf `.proto` definition files.
    /// Each entry is (package_name, proto_dir_path).
    Protos(Vec<(String, PathBuf)>),
    /// Previously generated types (in output directories).
    /// Each entry is (source_name, dir_path).
    Generated(Vec<(String, PathBuf)>),
}

impl TypeSource {
    /// Human-readable label for diagnostics.
    pub fn label(&self) -> &'static str {
        match self {
            TypeSource::Schemas(_) => "cosmwasm-schema",
            TypeSource::Protos(_) => "protobuf",
            TypeSource::Generated(_) => "pre-generated",
        }
    }

    /// Number of source entries.
    pub fn count(&self) -> usize {
        match self {
            TypeSource::Schemas(v) => v.len(),
            TypeSource::Protos(v) => v.len(),
            TypeSource::Generated(v) => v.len(),
        }
    }
}

// ---------------------------------------------------------------------------
// Source resolver
// ---------------------------------------------------------------------------

/// Discovers available type definitions and resolves them into a unified format.
/// One contract's name and its resolved type map.
pub type ContractTypes = (String, BTreeMap<String, serde_json::Value>);

pub struct SourceResolver;

impl SourceResolver {
    /// Discover all available type sources for a workspace.
    ///
    /// Returns sources in priority order (best first). Each source has its
    /// own conversion logic to produce the canonical schema format.
    pub fn discover(ctx: &GenerationContext) -> Vec<TypeSource> {
        let mut sources: Vec<TypeSource> = Vec::new();

        // 1. Check for CosmWasm JSON schemas (highest priority)
        let mut schema_dirs = find_schema_dirs(&ctx.workspace_root);

        // Also incorporate known contract schema dirs from workspace detection,
        // which go deeper than the shallow find_schema_dirs scan.
        for contract in &ctx.contracts {
            if contract.schema_dir.exists() && !schema_dirs.contains(&contract.schema_dir) {
                schema_dirs.push(contract.schema_dir.clone());
            }
        }

        if !schema_dirs.is_empty() {
            let entries: Vec<_> = schema_dirs
                .iter()
                .map(|d| {
                    let name = d
                        .parent()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "contract".to_string());
                    (name, d.clone())
                })
                .collect();
            sources.push(TypeSource::Schemas(entries));
        }

        // 2. Check for protobuf definitions
        let proto_dirs = find_proto_dirs(&ctx.workspace_root);
        if !proto_dirs.is_empty() {
            let entries: Vec<_> = proto_dirs
                .iter()
                .map(|d| {
                    let name = d
                        .parent()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "proto".to_string());
                    (name, d.clone())
                })
                .collect();
            sources.push(TypeSource::Protos(entries));
        }

        // 3. Check for pre-existing generated types (in output dirs)
        let generated_dirs = find_generated_dirs(&ctx.workspace_root);
        if !generated_dirs.is_empty() {
            let entries: Vec<_> = generated_dirs
                .iter()
                .map(|d| {
                    let name = d
                        .parent()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "generated".to_string());
                    (name, d.clone())
                })
                .collect();
            sources.push(TypeSource::Generated(entries));
        }

        sources
    }

    /// Resolve the best available source into a unified schema map.
    ///
    /// The returned map has the same shape that all generators expect:
    /// `{ type_name → JSON Schema object }`.
    ///
    /// Returns `(source_label, schemas)` where `source_label` is a human-readable
    /// description of which source was used (for diagnostics).
    pub fn resolve(ctx: &GenerationContext) -> (String, BTreeMap<String, serde_json::Value>) {
        let (label, grouped) = Self::resolve_grouped(ctx);
        // Merge all grouped maps into one flat map
        let mut merged = BTreeMap::new();
        for (_name, schemas) in &grouped {
            merged.extend(schemas.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        (label, merged)
    }

    /// Resolve types with per-contract grouping preserved.
    ///
    /// Returns `(source_label, Vec<(contract_name, type_map)>)` — one entry per
    /// discovered source directory. Generators that emit per-contract files
    /// (TS codegen, OpenAPI) should use this. Generators that process all types
    /// together (Go, Python, Zod) can use `resolve()` and get a flat map.
    pub fn resolve_grouped(ctx: &GenerationContext) -> (String, Vec<ContractTypes>) {
        let sources = Self::discover(ctx);

        // Try each source in priority order — use the first one with data
        for source in &sources {
            let _grouped = match Self::convert_source_grouped(source) {
                Some(v) if !v.is_empty() => {
                    let total: usize = v.iter().map(|(_, m)| m.len()).sum();
                    log::info!(
                        "[resolver] Resolved {} types from {} ({}) across {} groups",
                        total,
                        source.label(),
                        source.count(),
                        v.len()
                    );
                    return (source.label().to_string(), v);
                }
                _ => continue,
            };
        }

        log::warn!(
            "[resolver] No type sources found for {:?}",
            ctx.workspace_root
        );
        (String::new(), Vec::new())
    }

    /// Convert a single TypeSource into per-entry grouped schema maps.
    /// Returns `Vec<(entry_name, type_map)>` preserving the original grouping.
    fn convert_source_grouped(
        source: &TypeSource,
    ) -> Option<Vec<(String, BTreeMap<String, serde_json::Value>)>> {
        let mut result: Vec<(String, BTreeMap<String, serde_json::Value>)> = Vec::new();

        match source {
            TypeSource::Schemas(entries) => {
                // Load each schema directory, creating one entry per .json file
                // rather than aggregating all files into one flat map. This
                // ensures contracts in the same schema dir (e.g. cw-infuser
                // and cw-infusion-minter) each get their own output files.
                for (_dir_name, dir) in entries {
                    let per_file = load_schema_files_per_file(dir);
                    result.extend(per_file);
                }
            }
            TypeSource::Protos(entries) => {
                for (name, dir) in entries {
                    let schemas = proto_dir_to_schemas(dir);
                    if !schemas.is_empty() {
                        result.push((name.clone(), schemas));
                    }
                }
            }
            TypeSource::Generated(entries) => {
                for (name, dir) in entries {
                    let schemas = load_schema_jsons(dir);
                    if !schemas.is_empty() {
                        result.push((name.clone(), schemas));
                    }
                }
            }
        }

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    /// Quick-check whether any type sources are available.
    /// Useful for generators that want to skip early without resolving all types.
    pub fn has_sources(ctx: &GenerationContext) -> bool {
        let sources = Self::discover(ctx);
        !sources.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Proto → JSON Schema conversion
// ---------------------------------------------------------------------------

/// Convert all `.proto` files in a directory into JSON Schema representations.
fn proto_dir_to_schemas(proto_dir: &Path) -> BTreeMap<String, serde_json::Value> {
    let mut schemas = BTreeMap::new();

    let proto_files = match std::fs::read_dir(proto_dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "proto"))
            .collect::<Vec<_>>(),
        Err(_) => return schemas,
    };

    for entry in &proto_files {
        let path = entry.path();
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let file_schemas = parse_proto_content(&content, &path);
        schemas.extend(file_schemas);
    }

    schemas
}

/// Parse a `.proto` file content and emit JSON Schema representations.
///
/// This is a heuristic parser — it doesn't use a full protobuf compiler.
/// It handles the common patterns: message definitions, enum definitions,
/// field types, nested messages, oneof, and imports.
fn parse_proto_content(content: &str, _path: &Path) -> BTreeMap<String, serde_json::Value> {
    let mut schemas = BTreeMap::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        // Parse message blocks
        if line.starts_with("message ") && line.ends_with('{') {
            let msg_name = line
                .strip_prefix("message ")
                .and_then(|s| s.strip_suffix('{'))
                .map(|s| s.trim())
                .unwrap_or("")
                .to_string();

            if msg_name.is_empty() {
                i += 1;
                continue;
            }

            // Collect fields until closing brace
            let mut properties = serde_json::Map::new();
            let mut required = Vec::new();
            let mut field_num = 1;
            i += 1;

            while i < lines.len() && !lines[i].trim().eq("}") {
                let field_line = lines[i].trim();

                // Handle nested messages
                if field_line.starts_with("message ") && field_line.ends_with('{') {
                    // Recurse into nested message
                    let nested_content = extract_block(&lines, i);
                    let nested_schemas = parse_proto_content(&nested_content, _path);
                    schemas.extend(nested_schemas);
                    // Skip to end of nested block
                    i += count_block_lines(&lines, i);
                    continue;
                }

                // Skip comments, options, and package statements
                if field_line.starts_with("//")
                    || field_line.starts_with('#')
                    || field_line.starts_with("option")
                    || field_line.starts_with("import")
                    || field_line.starts_with("package")
                    || field_line.starts_with("syntax")
                    || field_line.starts_with("//")
                {
                    i += 1;
                    continue;
                }

                if let Some(prop) = parse_proto_field(field_line, &mut field_num) {
                    let (field_name, field_schema, is_required) = prop;
                    properties.insert(field_name.clone(), field_schema);
                    if is_required {
                        required.push(field_name);
                    }
                    field_num += 1;
                }

                i += 1;
            }

            let mut msg_schema = serde_json::json!({
                "type": "object",
                "title": msg_name,
            });

            if !properties.is_empty() {
                msg_schema
                    .as_object_mut()
                    .unwrap()
                    .insert("properties".into(), serde_json::Value::Object(properties));
            }

            if !required.is_empty() {
                msg_schema.as_object_mut().unwrap().insert(
                    "required".into(),
                    serde_json::Value::Array(
                        required
                            .into_iter()
                            .map(serde_json::Value::String)
                            .collect(),
                    ),
                );
            }

            schemas.insert(msg_name, msg_schema);
        }

        // Parse enum blocks
        if line.starts_with("enum ") && line.ends_with('{') {
            let enum_name = line
                .strip_prefix("enum ")
                .and_then(|s| s.strip_suffix('{'))
                .map(|s| s.trim())
                .unwrap_or("")
                .to_string();

            if !enum_name.is_empty() {
                let mut variants = Vec::new();
                i += 1;

                while i < lines.len() && !lines[i].trim().eq("}") {
                    let enum_line = lines[i].trim();
                    // Parse "VARIANT_NAME = N;" pattern
                    if let Some(equals_pos) = enum_line.find('=') {
                        let variant_name = enum_line[..equals_pos].trim();
                        if !variant_name.is_empty()
                            && !variant_name.starts_with('/')
                            && !variant_name.starts_with("option")
                            && !variant_name.starts_with("reserved")
                        {
                            // Skip numeric-only variants (proto reserved enum values)
                            if !variant_name.chars().all(|c| c.is_ascii_digit() || c == '_') {
                                variants.push(serde_json::Value::String(
                                    variant_name.to_lowercase().to_string(),
                                ));
                            }
                        }
                    }
                    i += 1;
                }

                schemas.insert(
                    enum_name.clone(),
                    serde_json::json!({
                        "type": "string",
                        "title": enum_name,
                        "enum": variants,
                    }),
                );
            }
        }

        i += 1;
    }

    schemas
}

/// Parse a single proto field line into a JSON Schema property.
/// Returns (field_name, schema_value, is_required).
fn parse_proto_field(line: &str, field_num: &mut u32) -> Option<(String, serde_json::Value, bool)> {
    let line = line.trim().trim_end_matches(';');

    // Skip empty lines, comments, and control statements
    if line.is_empty()
        || line.starts_with("//")
        || line.starts_with("option ")
        || line.starts_with("reserved ")
    {
        return None;
    }

    // Skip nested message/enum declarations
    if line.starts_with("message ") || line.starts_with("enum ") || line.starts_with("oneof ") {
        return None;
    }

    // Extract field number at end: "type name = N;"
    let eq_pos = line.rfind('=');
    eq_pos?;
    let eq_pos = eq_pos.unwrap();

    // Field spec is everything before the = sign
    let field_spec = line[..eq_pos].trim();

    // Parse: [repeated|optional] type field_name
    let parts: Vec<&str> = field_spec.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }

    let is_repeated = parts[0] == "repeated";
    let is_optional = parts[0] == "optional";
    let is_map = parts.len() >= 3 && parts[0] == "map";

    let (proto_type, field_name) = if is_repeated || is_optional {
        // repeated type field_name
        (parts[1], parts[2])
    } else if is_map {
        // map<key_type, value_type> field_name
        // Join the map type back
        let map_end = field_spec.rfind('>')?;
        let map_type = &field_spec[..map_end + 1];
        let after_map = field_spec[map_end + 1..].trim();
        if after_map.is_empty() {
            return None;
        }
        return Some((
            after_map.to_string(),
            proto_map_to_json_schema(map_type),
            true,
        ));
    } else {
        // type field_name
        (parts[0], parts[1])
    };

    let base_schema = proto_type_to_json_schema(proto_type);
    let field_schema = if is_repeated {
        serde_json::json!({
            "type": "array",
            "items": base_schema,
            "description": format!("repeated {}", proto_type),
        })
    } else {
        base_schema
    };

    // Update field number based on the parsed position
    if let Some(num_str) = line[eq_pos + 1..].split_whitespace().next()
        && let Ok(n) = num_str.parse::<u32>()
    {
        *field_num = n;
    }

    Some((
        field_name.to_string(),
        field_schema,
        !is_optional && !is_repeated,
    ))
}

/// Convert a proto type name to a JSON Schema representation.
fn proto_type_to_json_schema(proto_type: &str) -> serde_json::Value {
    // Strip leading package qualifiers
    let base = proto_type.split('.').next_back().unwrap_or(proto_type);

    match base {
        // Scalar types
        "double" | "float" => serde_json::json!({"type": "number"}),
        "int32" | "int64" | "sint32" | "sint64" | "sfixed32" | "sfixed64" => {
            serde_json::json!({"type": "integer", "format": base})
        }
        "uint32" | "uint64" | "fixed32" | "fixed64" => {
            serde_json::json!({"type": "string", "format": base})
        } // Cosmos convention
        "bool" => serde_json::json!({"type": "boolean"}),
        "string" => serde_json::json!({"type": "string"}),
        "bytes" => serde_json::json!({"type": "string", "contentEncoding": "base64"}),

        // Well-known types
        "Any" | "google.protobuf.Any" => {
            serde_json::json!({"type": "object", "description": "google.protobuf.Any"})
        }
        "Timestamp" | "google.protobuf.Timestamp" => {
            serde_json::json!({"type": "string", "format": "date-time"})
        }
        "Duration" | "google.protobuf.Duration" => {
            serde_json::json!({"type": "string"})
        }
        "Struct" | "google.protobuf.Struct" => {
            serde_json::json!({"type": "object"})
        }
        "Value" | "google.protobuf.Value" => {
            serde_json::json!({"type": "object"})
        }
        "NullValue" | "google.protobuf.NullValue" => {
            serde_json::json!({"type": "null"})
        }
        "Empty" | "google.protobuf.Empty" => {
            serde_json::json!({"type": "object"})
        }

        // Cosmos SDK well-known types
        "Coin" => {
            serde_json::json!({
                "type": "object",
                "title": "Coin",
                "properties": {
                    "denom": {"type": "string"},
                    "amount": {"type": "string"}
                },
                "required": ["denom", "amount"]
            })
        }
        "DecCoin" => {
            serde_json::json!({
                "type": "object",
                "title": "DecCoin",
                "properties": {
                    "denom": {"type": "string"},
                    "amount": {"type": "string"}
                },
                "required": ["denom", "amount"]
            })
        }

        // Unknown types get referenced as $ref
        _ => {
            serde_json::json!({"$ref": format!("#/definitions/{}", base)})
        }
    }
}

/// Convert a `map<key, value>` proto declaration to a JSON Schema.
fn proto_map_to_json_schema(map_decl: &str) -> serde_json::Value {
    // Extract key and value types from "map<key_type, value_type>"
    let inner = map_decl
        .strip_prefix("map<")
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or("");

    let parts: Vec<&str> = inner.splitn(2, ',').collect();
    let _key_type = parts.first().map(|s| s.trim()).unwrap_or("string");
    let value_type = parts.get(1).map(|s| s.trim()).unwrap_or("string");

    serde_json::json!({
        "type": "object",
        "additionalProperties": proto_type_to_json_schema(value_type),
        "description": format!("map<{}, {}>", _key_type, value_type),
    })
}

/// Extract a block of text starting at index `i` (which should be a `{` line)
/// through the matching closing `}`.
fn extract_block(lines: &[&str], start: usize) -> String {
    let mut depth = 1;
    let mut result = String::new();
    let mut i = start + 1;

    // Include the opening declaration line so recursive parsing sees it
    result.push_str(lines[start]);
    result.push('\n');

    while i < lines.len() && depth > 0 {
        let line = lines[i];
        depth += line.matches('{').count();
        depth -= line.matches('}').count();
        if depth > 0 {
            result.push_str(line);
            result.push('\n');
        }
        i += 1;
    }

    result
}

/// Count lines in a block starting at index `i`.
fn count_block_lines(lines: &[&str], start: usize) -> usize {
    let mut depth = 1;
    let mut i = start + 1;

    while i < lines.len() && depth > 0 {
        depth += lines[i].matches('{').count();
        depth -= lines[i].matches('}').count();
        i += 1;
    }

    i - start
}

// ---------------------------------------------------------------------------
// Source discovery helpers
// ---------------------------------------------------------------------------

/// Find all `schema/` directories in a workspace (CosmWasm JSON schemas).
fn find_schema_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if root.join("schema").exists() {
        dirs.push(root.join("schema"));
    }
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !path.ends_with("target") {
                let schema_path = path.join("schema");
                if schema_path.exists() {
                    dirs.push(schema_path);
                }
            }
        }
    }
    // Also walk into contracts/ subdirs (depth-3)
    if let Ok(entries) = std::fs::read_dir(root.join("contracts")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let schema_path = path.join("schema");
                if schema_path.exists() {
                    dirs.push(schema_path);
                }
            }
        }
    }
    dirs
}

/// Find all `proto/` directories in a workspace that contain `.proto` files.
fn find_proto_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.file_name().is_some_and(|n| n == "proto") {
                // Only include if it has .proto files
                if has_proto_files(&path) {
                    dirs.push(path);
                }
            }
        }
    }
    // Also check subdirectories for proto dirs (depth-2)
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !path.ends_with("target") {
                let proto_path = path.join("proto");
                if proto_path.exists() && has_proto_files(&proto_path) {
                    dirs.push(proto_path);
                }
            }
        }
    }
    dirs
}

/// Check if a directory contains any `.proto` files.
fn has_proto_files(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .any(|e| e.path().extension().is_some_and(|ext| ext == "proto"))
        })
        .unwrap_or(false)
}

/// Find pre-existing generated type directories.
fn find_generated_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    // Check common output dirs
    for candidate in &[
        "generated",
        "scripts/ts/src",
        "scripts/python",
        "ts/zod",
        "docs/openapi",
    ] {
        let path = root.join(candidate);
        if path.exists() && path.is_dir() {
            // Only include if it has JSON or TS files
            if has_type_files(&path) {
                dirs.push(path);
            }
        }
    }
    dirs
}

/// Check if a directory contains type-definition files (JSON, TS, or proto).
fn has_type_files(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries.flatten().any(|e| {
                e.path()
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| matches!(ext, "json" | "ts" | "py" | "go"))
            })
        })
        .unwrap_or(false)
}

/// Load JSON schema files from a directory into a name→value map.
pub fn load_schema_jsons(schema_dir: &Path) -> BTreeMap<String, serde_json::Value> {
    let mut schemas = BTreeMap::new();
    if !schema_dir.exists() {
        return schemas;
    }
    if let Ok(entries) = std::fs::read_dir(schema_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json")
                && let Ok(content) = std::fs::read_to_string(&path)
                && let Ok(value) = serde_json::from_str::<serde_json::Value>(&content)
            {
                let name = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                schemas.insert(name, value);
            }
        }
    }
    schemas
}

/// Load each .json schema file as its own contract entry.
///
/// Returns one `(contract_name, {contract_name: schema_value})` entry per
/// `.json` file. This ensures files like `schema/cw-infuser.json` and
/// `schema/cw-infusion-minter.json` each produce separate output files
/// rather than being merged into a single group.
fn load_schema_files_per_file(
    schema_dir: &Path,
) -> Vec<(String, BTreeMap<String, serde_json::Value>)> {
    let mut result: Vec<(String, BTreeMap<String, serde_json::Value>)> = Vec::new();
    if !schema_dir.exists() {
        return result;
    }
    if let Ok(entries) = std::fs::read_dir(schema_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json")
                && let Ok(content) = std::fs::read_to_string(&path)
                && let Ok(value) = serde_json::from_str::<serde_json::Value>(&content)
            {
                let name = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                // Skip non-contract files (e.g. raw, example, test)
                if name.is_empty() || name.starts_with('_') || name == "example" {
                    continue;
                }
                let mut schemas = BTreeMap::new();
                schemas.insert(name.clone(), value);
                result.push((name, schemas));
            }
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_proto_message() {
        let proto = r#"
syntax = "proto3";
package test.v1;

message SimpleMessage {
    string name = 1;
    int32 count = 2;
    bool active = 3;
}
"#;
        let schemas = parse_proto_content(proto, Path::new("test.proto"));
        assert!(schemas.contains_key("SimpleMessage"));
        let msg = schemas.get("SimpleMessage").unwrap();
        assert_eq!(msg["type"], "object");
        assert!(msg["properties"].is_object());

        let props = msg["properties"].as_object().unwrap();
        assert!(props.contains_key("name"));
        assert!(props.contains_key("count"));
        assert!(props.contains_key("active"));

        assert_eq!(props["name"]["type"], "string");
        assert_eq!(props["count"]["type"], "integer");
        assert_eq!(props["active"]["type"], "boolean");
    }

    #[test]
    fn test_parse_proto_enum() {
        let proto = r#"
enum Status {
    STATUS_UNSPECIFIED = 0;
    STATUS_ACTIVE = 1;
    STATUS_INACTIVE = 2;
}
"#;
        let schemas = parse_proto_content(proto, Path::new("test.proto"));
        assert!(schemas.contains_key("Status"));
        let status = schemas.get("Status").unwrap();
        assert_eq!(status["type"], "string");
        assert!(status["enum"].is_array());
    }

    #[test]
    fn test_parse_proto_repeated() {
        let proto = r#"
message WithRepeated {
    repeated string tags = 1;
    repeated int32 scores = 2;
}
"#;
        let schemas = parse_proto_content(proto, Path::new("test.proto"));
        let msg = schemas.get("WithRepeated").unwrap();
        let props = msg["properties"].as_object().unwrap();
        assert_eq!(props["tags"]["type"], "array");
        assert_eq!(props["tags"]["items"]["type"], "string");
        assert_eq!(props["scores"]["type"], "array");
    }

    #[test]
    fn test_resolve_no_sources() {
        let tmp = tempfile::tempdir().unwrap();
        // Should not panic with empty workspace
        let ctx = crate::config::GenerationContext {
            workspace_root: tmp.path().to_path_buf(),
            contracts: vec![],
            is_library: false,
            workspace_members: vec![],
            skip_schema: true,
            project_name: "test".into(),
            output_dir: tmp.path().join("generated"),
            ts_out: tmp.path().join("ts"),
            py_out: tmp.path().join("py"),
            zod_out: tmp.path().join("zod"),
            proto_out: tmp.path().join("proto"),
            go_out: tmp.path().join("go"),
            rust_out: tmp.path().join("rust"),
            openapi_out: tmp.path().join("openapi"),
            proto_modules: vec![],
            tz_out: None,
            tz_heuristics_path: None,
            tz_episodes_dir: None,
            tz_recipes_dir: None,
            tz_profiles_dir: None,
        };
        let (label, schemas) = SourceResolver::resolve(&ctx);
        assert!(label.is_empty());
        assert!(schemas.is_empty());
    }

    #[test]
    fn test_find_schema_dirs_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = find_schema_dirs(tmp.path());
        assert!(dirs.is_empty());
    }

    #[test]
    fn test_proto_scalar_types() {
        assert_eq!(proto_type_to_json_schema("string")["type"], "string");
        assert_eq!(proto_type_to_json_schema("int32")["type"], "integer");
        assert_eq!(proto_type_to_json_schema("bool")["type"], "boolean");
        assert_eq!(proto_type_to_json_schema("uint64")["format"], "uint64");
    }

    #[test]
    fn test_parse_proto_with_nested_message() {
        let proto = r#"
message Outer {
    string name = 1;
    message Inner {
        int32 value = 1;
    }
    Inner inner = 2;
}
"#;
        let schemas = parse_proto_content(proto, Path::new("test.proto"));
        assert!(schemas.contains_key("Outer"), "Outer should be parsed");
        assert!(schemas.contains_key("Inner"), "Inner should be parsed");
    }
}
