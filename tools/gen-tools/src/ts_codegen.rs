//! TypeScript code generation (cw-infuser design).
//!
//! Reads CosmWasm **merged schema JSON** files and generates:
//!   - `*.types.ts` — TypeScript type definitions for all message types,
//!     response types, and shared definitions
//!   - `*.client.ts` — CosmJS query/execute client classes
//!   - `*.message-composer.ts` — Message composer helpers
//!
//! Merged schemas from `cosmwasm-schema` have the structure:
//!   { contract_name, contract_version, idl_version,
//!     instantiate: { title, type, oneOf/properties, definitions },
//!     execute: { title, oneOf, definitions },
//!     query: { title, oneOf, definitions },
//!     migrate: { ... } | null,
//!     sudo: { ... } | null,
//!     responses: { query_name: { title, type, properties, definitions } }
//!   }

use crate::config::{ts_variant_dir, GenerationContext};
use crate::resolver::SourceResolver;
use crate::{GenerationResult, Generator};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub struct TsCodegenGenerator;

impl Generator for TsCodegenGenerator {
    fn name(&self) -> &'static str {
        "ts-codegen"
    }

    fn enabled_by_default(&self) -> bool {
        true
    }

    fn generate(&self, ctx: &GenerationContext) -> anyhow::Result<GenerationResult> {
        std::fs::create_dir_all(&ctx.ts_out)?;

        let mut total_files = 0;
        let mut contracts_processed = 0;

        let (_source_label, grouped) = SourceResolver::resolve_grouped(ctx);
        if grouped.is_empty() {
            log::warn!("[ts-codegen] No type sources found");
            return Ok(GenerationResult {
                name: "ts-codegen",
                success: true,
                files_generated: 0,
                output_dir: Some(ctx.ts_out.to_string_lossy().to_string()),
                message: Some("No contracts found to process".to_string()),
            });
        }

        for (contract_name, schemas) in &grouped {
            // Each contract has one entry in the BTreeMap: the full merged schema JSON
            // We need to collect ALL definitions across all message/response types
            let mut all_defs: HashMap<String, Value> = HashMap::new();
            let mut message_types: Vec<(String, Value)> = Vec::new();
            let mut response_types: Vec<(String, Value)> = Vec::new();

            for (_type_key, schema_value) in schemas {
                // schema_value is the full merged schema JSON for this contract
                collect_types_and_defs(schema_value, &mut message_types, &mut response_types, &mut all_defs);
            }

            // Merge definitions found across all message and response schemas
            // This ensures $ref resolution works globally

            let cname = pascal_case(contract_name);
            let types_dir = ts_variant_dir(&ctx.ts_out, contract_name, "types");
            let client_dir = ts_variant_dir(&ctx.ts_out, contract_name, "client");
            let composer_dir = ts_variant_dir(&ctx.ts_out, contract_name, "message-composer");
            std::fs::create_dir_all(&types_dir)?;
            std::fs::create_dir_all(&client_dir)?;
            std::fs::create_dir_all(&composer_dir)?;

            let types_ts = generate_types_file(&cname, &message_types, &response_types, &all_defs)?;
            std::fs::write(types_dir.join(format!("{}.types.ts", cname)), &types_ts)?;
            total_files += 1;

            let client_ts = generate_client_file(&cname, &message_types, &response_types)?;
            std::fs::write(client_dir.join(format!("{}.client.ts", cname)), &client_ts)?;
            total_files += 1;

            let composer_ts = generate_message_composer_file(&cname, &message_types)?;
            std::fs::write(
                composer_dir.join(format!("{}.message-composer.ts", cname)),
                &composer_ts,
            )?;
            total_files += 1;

            let barrel_ts = format!(
                "export * from './types/{cname}.types';\n\
                 export * from './client/{cname}.client';\n\
                 export * from './message-composer/{cname}.message-composer';\n\
                 export * from './zod/{cname}.zod';\n\
                 export * from './esbundles/{cname}.bundle.mjs';\n\
                 export * from './filters/{cname}.filters';\n\
                 export * from './argus/{cname}.argus';\n",
            );
            std::fs::write(ctx.ts_out.join(contract_name).join("index.ts"), &barrel_ts)?;
            total_files += 1;

            contracts_processed += 1;
        }

        Ok(GenerationResult {
            name: "ts-codegen",
            success: true,
            files_generated: total_files,
            output_dir: Some(ctx.ts_out.to_string_lossy().to_string()),
            message: Some(format!(
                "{} contracts processed, {} files generated",
                contracts_processed, total_files
            )),
        })
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Schema parsing — extract message types, response types, and definitions
// from a CosmWasm merged schema JSON value.
// ═══════════════════════════════════════════════════════════════════════════

/// Known message-type top-level keys in a merged schema.
const MSG_KEYS: &[&str] = &["instantiate", "execute", "query", "migrate", "sudo"];

/// Metadata keys to skip.
const _SKIP_KEYS: &[&str] = &["contract_name", "contract_version", "idl_version", "$schema"];

pub fn collect_types_and_defs(
    merged: &Value,
    message_types: &mut Vec<(String, Value)>,
    response_types: &mut Vec<(String, Value)>,
    all_defs: &mut HashMap<String, Value>,
) {
    // Collect message types (instantiate, execute, query, etc.)
    for key in MSG_KEYS {
        if let Some(msg_val) = merged.get(*key) {
            if msg_val.is_null() {
                continue;
            }
            let title = msg_val
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| key)
                .to_string();
            message_types.push((title.clone(), msg_val.clone()));

            // Collect definitions embedded in this message type
            collect_definitions(msg_val, all_defs);
        }
    }

    // Collect response types
    if let Some(responses) = merged.get("responses").and_then(|v| v.as_object()) {
        for (query_name, resp_schema) in responses {
            let title = resp_schema
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| query_name)
                .to_string();
            response_types.push((title, resp_schema.clone()));

            // Collect definitions embedded in this response type
            collect_definitions(resp_schema, all_defs);
        }
    }
}

/// Recursively collect `definitions` from a nested schema object.
fn collect_definitions(schema: &Value, defs: &mut HashMap<String, Value>) {
    if let Some(defs_obj) = schema.get("definitions").and_then(|v| v.as_object()) {
        for (name, val) in defs_obj {
            defs.entry(name.clone()).or_insert_with(|| val.clone());
        }
    }
    // Also look for nested definitions inside $defs (JSON Schema 2020-12 style)
    if let Some(defs_obj) = schema.get("$defs").and_then(|v| v.as_object()) {
        for (name, val) in defs_obj {
            defs.entry(name.clone()).or_insert_with(|| val.clone());
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TypeScript type file generation
// ═══════════════════════════════════════════════════════════════════════════

fn generate_types_file(
    cname: &str,
    message_types: &[(String, Value)],
    response_types: &[(String, Value)],
    all_defs: &HashMap<String, Value>,
) -> anyhow::Result<String> {
    let mut output = String::new();
    output.push_str("// Auto-generated by gen-tools.\n// Do not edit manually.\n\n");

    // Track names we've already emitted to avoid duplicates
    let mut emitted = HashSet::new();

    // 1. Emit all shared definitions as top-level types
    let mut def_names: Vec<&String> = all_defs.keys().collect();
    def_names.sort();
    for name in def_names {
        if emitted.contains(name.as_str()) {
            continue;
        }
        if let Some(ts_type) = def_to_ts_type(name, &all_defs[name], all_defs) {
            output.push_str(&ts_type);
            output.push('\n');
            emitted.insert(name.clone());
        }
    }

    // 2. Emit message types
    for (title, schema) in message_types {
        if emitted.contains(title.as_str()) {
            continue;
        }
        if let Some(ts_type) = schema_to_union_type(title, schema, all_defs, &emitted) {
            output.push_str(&ts_type);
            output.push('\n');
            emitted.insert(title.clone());
        }
    }

    // 3. Emit response types
    for (title, schema) in response_types {
        if emitted.contains(title.as_str()) {
            continue;
        }
        if let Some(ts_type) = schema_to_interface(title, schema, all_defs, &emitted) {
            output.push_str(&ts_type);
            output.push('\n');
            emitted.insert(title.clone());
        }
    }

    // 4. Emit top-level contract type as union of all message types
    if !message_types.is_empty() {
        let msg_type_names: Vec<&str> = message_types.iter().map(|(t, _)| t.as_str()).collect();
        output.push_str(&format!("export type {} = {};\n", cname, msg_type_names.join(" | ")));
    }

    Ok(output)
}

/// Convert a top-level definition into a TS type, interface, or enum.
fn def_to_ts_type(name: &str, val: &Value, all_defs: &HashMap<String, Value>) -> Option<String> {
    let pname = pascal_case(name);
    let mut out = String::new();

    // Doc comment
    if let Some(desc) = val.get("description").and_then(|v| v.as_str()) {
        out.push_str(&format!("/** {} */\n", desc));
    }

    match val.get("type").and_then(|v| v.as_str()) {
        Some("object") => {
            out.push_str(&format!("export interface {} {{\n", pname));
            if let Some(props) = val.get("properties").and_then(|v| v.as_object()) {
                let required = val
                    .get("required")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|r| r.as_str()).collect::<HashSet<_>>())
                    .unwrap_or_default();
                for (field_name, field_schema) in props {
                    let opt = if !required.contains(field_name.as_str()) { "?" } else { "" };
                    let field_doc = field_schema.get("description").and_then(|v| v.as_str());
                    if let Some(doc) = field_doc {
                        out.push_str(&format!("  /** {} */\n", doc));
                    }
                    let ft = json_schema_to_ts(field_schema, all_defs);
                    out.push_str(&format!("  {}{}: {};\n", field_name, opt, ft));
                }
            }
            out.push_str("}\n");
            Some(out)
        }
        Some("string") => {
            if let Some(variants) = val.get("enum").and_then(|v| v.as_array()) {
                // String enum
                out.push_str(&format!("export enum {} {{\n", pname));
                for v in variants {
                    if let Some(s) = v.as_str() {
                        let key = s.to_uppercase().replace('-', "_");
                        out.push_str(&format!("  {} = '{}',\n", key, s));
                    }
                }
                out.push_str("}\n");
            } else {
                // Type alias for branded/constrained string
                out.push_str(&format!("export type {} = string;\n", pname));
            }
            Some(out)
        }
        Some("integer") | Some("number") => {
            out.push_str(&format!("export type {} = number;\n", pname));
            Some(out)
        }
        Some("boolean") => {
            out.push_str(&format!("export type {} = boolean;\n", pname));
            Some(out)
        }
        Some("array") => {
            let inner = val.get("items").map(|i| json_schema_to_ts(i, all_defs)).unwrap_or_else(|| "unknown".to_string());
            out.push_str(&format!("export type {} = {}[];\n", pname, inner));
            Some(out)
        }
        Some(t) => {
            out.push_str(&format!("export type {} = {};\n", pname, t));
            Some(out)
        }
        None => {
            // oneOf/anyOf or allOf
            if val.get("oneOf").is_some() || val.get("anyOf").is_some() {
                out.push_str(&format!("export type {} =\n", pname));
                let arr = val.get("oneOf").or_else(|| val.get("anyOf")).and_then(|v| v.as_array()).unwrap();
                for (i, v) in arr.iter().enumerate() {
                    let t = json_schema_to_ts(v, all_defs);
                    let d = if i < arr.len() - 1 { "" } else { ";" };
                    out.push_str(&format!("  | {}{}\n", t, d));
                }
                Some(out)
            } else {
                // Unknown structure; emit as unknown
                out.push_str(&format!("export type {} = unknown;\n", pname));
                Some(out)
            }
        }
    }
}

/// Convert a message-type schema (oneOf/variants) into a discriminated union.
fn schema_to_union_type(
    title: &str,
    schema: &Value,
    all_defs: &HashMap<String, Value>,
    _emitted: &HashSet<String>,
) -> Option<String> {
    let pname = pascal_case(title);
    let mut out = String::new();

    if let Some(desc) = schema.get("description").and_then(|v| v.as_str()) {
        out.push_str(&format!("/** {} */\n", desc));
    }

    // OneOf variants → discriminated union
    if let Some(variants) = schema.get("oneOf").and_then(|v| v.as_array()) {
        out.push_str(&format!("export type {} =\n", pname));
        for (i, variant) in variants.iter().enumerate() {
            let variant_type = variant_oneof_to_type(variant, all_defs);
            let delim = if i < variants.len() - 1 { "" } else { ";" };
            out.push_str(&format!("  | {}{}\n", variant_type, delim));
        }
        return Some(out);
    }

    // Simple object → interface
    if schema.get("type").and_then(|v| v.as_str()) == Some("object") {
        if let Some(iface) = schema_to_interface(title, schema, all_defs, _emitted) {
            return Some(iface);
        }
    }

    // AllOf → intersection type
    if let Some(all_of) = schema.get("allOf").and_then(|v| v.as_array()) {
        out.push_str(&format!("export type {} = ", pname));
        for (i, item) in all_of.iter().enumerate() {
            let t = json_schema_to_ts(item, all_defs);
            out.push_str(&t);
            if i < all_of.len() - 1 {
                out.push_str(" & ");
            }
        }
        out.push_str(";\n");
        return Some(out);
    }

    None
}

/// Convert one variant of a oneOf into a type expression.
/// For "{ type: 'object', required: ['create_infusion'], properties: { ... } }",
/// this generates `{ create_infusion: CreateInfusion }` inline or uses the named type.
fn variant_oneof_to_type(variant: &Value, all_defs: &HashMap<String, Value>) -> String {
    // Single-key object: extract the key as the variant discriminator
    if let Some(props) = variant.get("properties").and_then(|v| v.as_object()) {
        if props.len() == 1 {
            if let Some((field_name, field_schema)) = props.iter().next() {
                let field_type = json_schema_to_ts(field_schema, all_defs);
                return format!("{{ {}: {} }}", field_name, field_type);
            }
        }
        // Multi-property variant
        let required = variant
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|r| r.as_str()).collect::<HashSet<_>>())
            .unwrap_or_default();
        let fields: Vec<String> = props
            .iter()
            .map(|(k, v)| {
                let _opt = if !required.contains(k.as_str()) { "?" } else { "" };
                format!("{}: {}", k, json_schema_to_ts(v, all_defs))
            })
            .collect();
        if fields.len() == 1 && !fields[0].contains(": ") {
            return fields[0].clone();
        }
        return format!("{{ {} }}", fields.join("; "));
    }

    // $ref based variant
    if let Some(ref_path) = variant.get("$ref").and_then(|v| v.as_str()) {
        let ref_name = ref_path.rsplit('/').next().unwrap_or(ref_path);
        return pascal_case(ref_name);
    }

    json_schema_to_ts(variant, all_defs)
}

/// Convert a response / object schema into a TS interface.
fn schema_to_interface(
    title: &str,
    schema: &Value,
    all_defs: &HashMap<String, Value>,
    _emitted: &HashSet<String>,
) -> Option<String> {
    let pname = pascal_case(title);
    let mut out = String::new();

    // Doc comment
    if let Some(desc) = schema.get("description").and_then(|v| v.as_str()) {
        out.push_str(&format!("/** {} */\n", desc));
    }

    // type: 'object' with properties → interface
    if schema.get("type").and_then(|v| v.as_str()) == Some("object") {
        if let Some(props) = schema.get("properties").and_then(|v| v.as_object()) {
            let required = schema
                .get("required")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|r| r.as_str()).collect::<HashSet<_>>())
                .unwrap_or_default();

            out.push_str(&format!("export interface {} {{\n", pname));
            for (field_name, field_schema) in props {
                let opt = if !required.contains(field_name.as_str()) { "?" } else { "" };
                let field_doc = field_schema.get("description").and_then(|v| v.as_str());
                if let Some(doc) = field_doc {
                    out.push_str(&format!("  /** {} */\n", doc));
                }
                let ft = json_schema_to_ts(field_schema, all_defs);
                out.push_str(&format!("  {}{}: {};\n", field_name, opt, ft));
            }
            out.push_str("}\n");
            return Some(out);
        }
    }

    // No properties but enum → emit as type alias
    if schema.get("type").and_then(|v| v.as_str()) == Some("string") {
        if let Some(variants) = schema.get("enum").and_then(|v| v.as_array()) {
            let vals: Vec<String> = variants
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| format!("'{}'", s))
                .collect();
            out.push_str(&format!("export type {} = {};\n", pname, vals.join(" | ")));
            return Some(out);
        }
    }

    None
}

// ═══════════════════════════════════════════════════════════════════════════
// JSON Schema → TypeScript type conversion
// ═══════════════════════════════════════════════════════════════════════════

/// Convert a JSON schema node into a TS type expression string.
fn json_schema_to_ts(schema: &Value, all_defs: &HashMap<String, Value>) -> String {
    // $ref resolution
    if let Some(ref_path) = schema.get("$ref").and_then(|v| v.as_str()) {
        let ref_name = ref_path.rsplit('/').next().unwrap_or(ref_path);
        let pname = pascal_case(ref_name);
        // Check if this definition exists in our collected defs
        if all_defs.contains_key(ref_name) || all_defs.contains_key(&pname) {
            return pname;
        }
        return pname;
    }

    // const values
    if let Some(const_val) = schema.get("const") {
        if let Some(s) = const_val.as_str() {
            return format!("'{}'", s);
        }
        return format!("{}", const_val);
    }

    // oneOf / anyOf → union
    if let Some(arr) = schema.get("oneOf").or_else(|| schema.get("anyOf")).and_then(|v| v.as_array()) {
        let types: Vec<String> = arr.iter().map(|v| json_schema_to_ts(v, all_defs)).collect();
        return types.join(" | ");
    }

    // allOf → intersection
    if let Some(arr) = schema.get("allOf").and_then(|v| v.as_array()) {
        let types: Vec<String> = arr.iter().map(|v| json_schema_to_ts(v, all_defs)).collect();
        return types.join(" & ");
    }

    match schema.get("type").and_then(|v| v.as_str()) {
        Some("string") => {
            // String enum → union of literals
            if let Some(variants) = schema.get("enum").and_then(|v| v.as_array()) {
                let vals: Vec<String> = variants
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| format!("'{}'", s))
                    .collect();
                return vals.join(" | ");
            }
            // Nullable string
            "string".to_string()
        }
        Some("integer") | Some("number") => "number".to_string(),
        Some("boolean") => "boolean".to_string(),
        Some("null") => "null".to_string(),
        Some("array") => {
            let inner = schema
                .get("items")
                .map(|i| json_schema_to_ts(i, all_defs))
                .unwrap_or_else(|| "unknown".to_string());
            format!("{}[]", inner)
        }
        Some("object") => {
            // Inline object type
            if let Some(props) = schema.get("properties").and_then(|v| v.as_object()) {
                let required = schema
                    .get("required")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|r| r.as_str()).collect::<HashSet<_>>())
                    .unwrap_or_default();
                let fields: Vec<String> = props
                    .iter()
                    .map(|(k, v)| {
                        let _opt = if !required.contains(k.as_str()) { "?" } else { "" };
                        format!("{}: {}", k, json_schema_to_ts(v, all_defs))
                    })
                    .collect();
                format!("{{ {} }}", fields.join("; "))
            } else {
                "Record<string, unknown>".to_string()
            }
        }
        // Type array: ["string", "null"] → nullable
        Some(_) => "string".to_string(),
        None => {
            // No explicit type — check for nested structure
            if schema.get("properties").is_some() {
                return json_schema_to_ts_type_obj(schema, all_defs);
            }
            "unknown".to_string()
        }
    }
}

/// Handle type: <array> cases — e.g., `"type": ["string", "null"]`
fn json_schema_to_ts_type_obj(schema: &Value, _all_defs: &HashMap<String, Value>) -> String {
    // Type array like ["string", "null"] → string | null
    if let Some(types) = schema.get("type").and_then(|v| v.as_array()) {
        let ts_types: Vec<String> = types
            .iter()
            .filter_map(|t| t.as_str())
            .map(|t| match t {
                "string" => "string",
                "integer" | "number" => "number",
                "boolean" => "boolean",
                "null" => "null",
                "object" => "Record<string, unknown>",
                "array" => "unknown[]",
                _ => "unknown",
            })
            .map(String::from)
            .collect();
        if !ts_types.is_empty() {
            return ts_types.join(" | ");
        }
    }

    "unknown".to_string()
}

// ═══════════════════════════════════════════════════════════════════════════
// Client generation — query and execute client interfaces
// ═══════════════════════════════════════════════════════════════════════════

fn generate_client_file(
    cname: &str,
    message_types: &[(String, Value)],
    response_types: &[(String, Value)],
) -> anyhow::Result<String> {
    let mut out = String::new();
    out.push_str("// Auto-generated by gen-tools.\n");
    out.push_str("import type { ");
    out.push_str(cname);
    out.push_str(" as ");
    out.push_str(cname);
    out.push_str("Type } from './");
    out.push_str(cname);
    out.push_str(".types';\n");
    out.push_str("import type { CosmWasmClient, SigningCosmWasmClient } from '@cosmjs/cosmwasm-stargate';\n\n");

    // Find execute and query message schemas
    let execute_schema = message_types.iter().find(|(t, _)| t.contains("Execute"));
    let query_schema = message_types.iter().find(|(t, _)| t.contains("Query"));

    // Extract execute variant names
    let exec_variants: Vec<String> = execute_schema
        .and_then(|(_, s)| extract_oneof_variant_names(s))
        .unwrap_or_default();
    let query_variants: Vec<String> = query_schema
        .and_then(|(_, s)| extract_oneof_variant_names(s))
        .unwrap_or_default();

    // Extract response type names for query methods
    let query_response_map: HashMap<String, String> = response_types
        .iter()
        .map(|(name, _)| (camel_case(name), pascal_case(name)))
        .collect();

    // Query client
    out.push_str(&format!("export interface {}QueryClient {{\n", cname));
    out.push_str("  readonly client: CosmWasmClient;\n");
    out.push_str("  readonly contractAddress: string;\n\n");

    for variant in &query_variants {
        let method = camel_case(variant);
        let response_type = query_response_map
            .get(&method)
            .map(|s| s.clone())
            .unwrap_or_else(|| "unknown".to_string());
        let param_type = format!("Extract<{}Type, {{ {}: unknown }}>['{}']", cname, variant, variant);
        out.push_str(&format!(
            "  {}(params: {}): Promise<{}>;\n",
            method, param_type, response_type
        ));
    }
    out.push_str("}\n\n");

    // Execute client
    out.push_str(&format!("export interface {}ExecuteClient {{\n", cname));
    out.push_str("  readonly client: SigningCosmWasmClient;\n");
    out.push_str("  readonly contractAddress: string;\n");
    out.push_str("  readonly sender: string;\n\n");

    for variant in &exec_variants {
        let method = camel_case(variant);
        let param_type = format!("Extract<{}Type, {{ {}: unknown }}>['{}']", cname, variant, variant);
        out.push_str(&format!(
            "  {}(params: {}): Promise<string>;\n",
            method, param_type
        ));
    }
    out.push_str("}\n");

    Ok(out)
}

// ═══════════════════════════════════════════════════════════════════════════
// Message composer generation
// ═══════════════════════════════════════════════════════════════════════════

fn generate_message_composer_file(
    cname: &str,
    message_types: &[(String, Value)],
) -> anyhow::Result<String> {
    let mut out = String::new();
    out.push_str("// Auto-generated by gen-tools.\n");
    out.push_str(&format!("import type {{ {} }} from './{}.types';\n", cname, cname));
    out.push_str("import type { Coin } from '@cosmjs/amino';\n");
    out.push_str("import type { MsgExecuteContract } from 'cosmjs-types/cosmwasm/wasm/v1/tx';\n\n");

    out.push_str(&format!("export class {}MsgComposer {{\n", cname));
    out.push_str("  readonly contractAddress: string;\n\n");
    out.push_str("  constructor(contractAddress: string) {\n");
    out.push_str("    this.contractAddress = contractAddress;\n");
    out.push_str("  }\n\n");

    let execute_schema = message_types.iter().find(|(t, _)| t.contains("Execute"));
    let exec_variants: Vec<String> = execute_schema
        .and_then(|(_, s)| extract_oneof_variant_names(s))
        .unwrap_or_default();

    for variant in &exec_variants {
        let method = camel_case(variant);
        out.push_str(&format!(
            "  {}(msg: Extract<{}Type, {{ {}: unknown }}>): MsgExecuteContract {{\n",
            method, cname, variant
        ));
        out.push_str("    return {\n");
        out.push_str("      typeUrl: '/cosmwasm.wasm.v1.MsgExecuteContract',\n");
        out.push_str("      value: {\n");
        out.push_str("        sender: '',\n");
        out.push_str("        contract: this.contractAddress,\n");
        out.push_str("        msg: Buffer.from(JSON.stringify(msg)),\n");
        out.push_str("        funds: [] as Coin[],\n");
        out.push_str("      },\n");
        out.push_str("    } as MsgExecuteContract;\n");
        out.push_str("  }\n\n");
    }

    out.push_str("}\n");
    Ok(out)
}

// ═══════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Extract the variant discriminator names from a oneOf schema.
/// e.g., [{ required: ["create_infusion"], ... }, { required: ["wavs_entry_point"], ... }]
/// → ["create_infusion", "wavs_entry_point"]
fn extract_oneof_variant_names(schema: &Value) -> Option<Vec<String>> {
    let variants = schema.get("oneOf").and_then(|v| v.as_array())?;
    let names: Vec<String> = variants
        .iter()
        .filter_map(|v| {
            // Get the first required field — that's the discriminator
            let required = v.get("required").and_then(|v| v.as_array())?;
            required.first().and_then(|r| r.as_str()).map(|s| s.to_string())
        })
        .collect();
    if names.is_empty() { None } else { Some(names) }
}

fn pascal_case(s: &str) -> String {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().to_string() + c.as_str(),
            }
        })
        .collect()
}

fn camel_case(s: &str) -> String {
    let pascal = pascal_case(s);
    let mut c = pascal.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_lowercase().to_string() + c.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pascal_case() {
        assert_eq!(pascal_case("cw-infuser"), "CwInfuser");
        assert_eq!(pascal_case("dao_calendar"), "DaoCalendar");
        assert_eq!(pascal_case("hello_world"), "HelloWorld");
        assert_eq!(pascal_case("create_infusion"), "CreateInfusion");
    }

    #[test]
    fn test_camel_case() {
        assert_eq!(camel_case("cw-infuser"), "cwInfuser");
        assert_eq!(camel_case("HelloWorld"), "helloWorld");
        assert_eq!(camel_case("create_infusion"), "createInfusion");
    }

    #[test]
    fn test_extract_oneof_variant_names() {
        let schema = serde_json::json!({
            "oneOf": [
                { "required": ["create_infusion"], "properties": {} },
                { "required": ["wavs_entry_point"], "properties": {} },
            ]
        });
        let names = extract_oneof_variant_names(&schema).unwrap();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"create_infusion".to_string()));
        assert!(names.contains(&"wavs_entry_point".to_string()));
    }

    #[test]
    fn test_collect_defs() {
        let schema = serde_json::json!({
            "title": "ExecuteMsg",
            "oneOf": [],
            "definitions": {
                "Coin": { "type": "object", "properties": {} },
                "Decimal": { "type": "string" },
            }
        });
        let mut defs = HashMap::new();
        collect_definitions(&schema, &mut defs);
        assert!(defs.contains_key("Coin"));
        assert!(defs.contains_key("Decimal"));
        assert_eq!(defs.len(), 2);
    }

    #[test]
    fn test_collect_types_and_defs() {
        let merged = serde_json::json!({
            "contract_name": "test",
            "contract_version": "1.0",
            "idl_version": "1",
            "instantiate": {
                "title": "InstantiateMsg",
                "type": "object",
                "properties": { "owner": { "type": "string" } },
                "definitions": { "Coin": { "type": "object", "properties": { "denom": {"type": "string"} } } }
            },
            "execute": {
                "title": "ExecuteMsg",
                "oneOf": [
                    { "required": ["do_something"], "properties": { "do_something": { "type": "object" } } }
                ],
                "definitions": {}
            },
            "query": {
                "title": "QueryMsg",
                "oneOf": [
                    { "required": ["get_info"], "properties": { "get_info": { "type": "object" } } }
                ]
            },
            "migrate": null,
            "sudo": null,
            "responses": {
                "get_info": {
                    "title": "InfoResponse",
                    "type": "object",
                    "properties": { "data": { "type": "string" } }
                }
            }
        });

        let mut message_types = Vec::new();
        let mut response_types = Vec::new();
        let mut all_defs = HashMap::new();

        collect_types_and_defs(&merged, &mut message_types, &mut response_types, &mut all_defs);

        assert_eq!(message_types.len(), 3); // instantiate, execute, query
        assert_eq!(response_types.len(), 1);
        assert!(all_defs.contains_key("Coin"));
        assert!(message_types.iter().any(|(t, _)| t == "InstantiateMsg"));
        assert!(message_types.iter().any(|(t, _)| t == "ExecuteMsg"));
    }
}