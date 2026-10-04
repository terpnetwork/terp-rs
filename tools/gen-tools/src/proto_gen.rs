//! Proto type definition generation from CosmWasm schemas.
//!
//! Converts CosmWasm JSON schema types into .proto definitions,
//! enabling cross-language protobuf serialization of contract types.

use crate::config::{contract_api_dir, GenerationContext};
use crate::resolver::SourceResolver;
use crate::{GenerationResult, Generator};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub struct ProtoGenGenerator;

impl Generator for ProtoGenGenerator {
    fn name(&self) -> &'static str {
        "proto"
    }

    fn enabled_by_default(&self) -> bool {
        true
    }

    fn generate(&self, ctx: &GenerationContext) -> anyhow::Result<GenerationResult> {
        std::fs::create_dir_all(&ctx.proto_out)?;

        let mut total_files = 0;

        let (_label, grouped) = SourceResolver::resolve_grouped(ctx);
        for (contract_name, schemas) in &grouped {
            let proto_content = generate_proto_from_schemas(schemas, contract_name)?;
            if !proto_content.is_empty() {
                let contract_dir = contract_api_dir(&ctx.proto_out, contract_name);
                std::fs::create_dir_all(&contract_dir)?;
                let proto_path = contract_dir.join(format!("{}.proto", contract_name));
                std::fs::write(&proto_path, &proto_content)?;
                total_files += 1;
                log::info!("[proto] Wrote {:?}", proto_path);
                let registry = generate_type_url_registry(&contract_dir)?;
                std::fs::write(contract_dir.join("type_url_registry.rs"), &registry)?;
                total_files += 1;
            }
        }

        // Chain modules live in the workspace proto/ tree. Vendor them (plus
        // gogoproto/google, which contract files import) so protoc can resolve
        // includes from api/proto/terp-proto.
        let vendored = vendor_chain_protos(&ctx.proto_out, &ctx.workspace_root)?;
        total_files += vendored;

        Ok(GenerationResult {
            name: "proto",
            success: true,
            files_generated: total_files,
            output_dir: Some(ctx.proto_out.to_string_lossy().to_string()),
            message: Some(format!(
                "{} contract proto files, {} vendored chain protos",
                total_files - vendored,
                vendored
            )),
        })
    }
}

/// Generate proto definitions from JSON schema files.
///
/// CosmWasm contract schemas are externally tagged enums (`oneOf` of
/// single-property objects) plus named `definitions`. Each of those becomes
/// a message or enum. Unions are `oneof` fields, not `google.protobuf.Any`.
fn generate_proto_from_schemas(
    schemas: &BTreeMap<String, serde_json::Value>,
    package_name: &str,
) -> anyhow::Result<String> {
    if schemas.is_empty() {
        return Ok(String::new());
    }

    let mut builder = ProtoBuilder::default();
    for schema in schemas.values() {
        collect_defs(schema, &mut builder.defs);
    }
    for schema in schemas.values() {
        builder.emit_root(schema);
    }
    // Named definitions that no field walked still need a message.
    let def_names: Vec<String> = builder.defs.keys().cloned().collect();
    for name in def_names {
        builder.ensure_ref(&name);
    }

    Ok(builder.render_file(package_name))
}

#[derive(Default)]
struct ProtoBuilder {
    defs: BTreeMap<String, Value>,
    messages: BTreeMap<String, String>,
    enums: BTreeMap<String, String>,
    done: BTreeSet<String>,
    needs_unit: bool,
    needs_proto_unit: bool,
    depth: u32,
}

struct ProtoType {
    ty: String,
    repeated: bool,
    optional: bool,
}

impl ProtoBuilder {
    fn emit_root(&mut self, schema: &Value) {
        let merged = ["instantiate", "execute", "query", "migrate", "sudo", "responses"]
            .iter()
            .any(|key| schema.get(*key).is_some());
        if merged {
            for key in ["instantiate", "execute", "query", "migrate", "sudo"] {
                if let Some(msg) = schema.get(key) {
                    if msg.is_null() {
                        continue;
                    }
                    let name = msg
                        .get("title")
                        .and_then(|t| t.as_str())
                        .unwrap_or(key);
                    self.ensure(name, msg);
                }
            }
            if let Some(responses) = schema.get("responses").and_then(|v| v.as_object()) {
                for (key, resp) in responses {
                    if resp.is_null() {
                        continue;
                    }
                    let name = resp
                        .get("title")
                        .and_then(|t| t.as_str())
                        .unwrap_or(key.as_str());
                    self.ensure(name, resp);
                }
            }
            return;
        }

        let name = schema
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("Msg");
        self.ensure(name, schema);
    }

    fn ensure(&mut self, raw_name: &str, schema: &Value) {
        let name = sanitize_type_name(raw_name);
        if !self.done.insert(name.clone()) {
            return;
        }
        if let Some(body) = enum_body(&name, schema) {
            self.enums.insert(name, body);
            return;
        }
        let body = self.render_message(&name, schema);
        self.messages.insert(name, body);
    }

    fn ensure_ref(&mut self, raw_name: &str) {
        let name = sanitize_type_name(raw_name);
        if self.done.contains(&name) {
            return;
        }
        if let Some(schema) = self.defs.get(raw_name).cloned().or_else(|| {
            self.defs
                .iter()
                .find(|(k, _)| sanitize_type_name(k) == name)
                .map(|(_, v)| v.clone())
        }) {
            self.ensure(&name, &schema);
        } else if !self.messages.contains_key(&name) && !self.enums.contains_key(&name) {
            self.done.insert(name.clone());
            self.messages
                .insert(name.clone(), format!("message {name} {{\n}}\n"));
        }
    }

    fn render_message(&mut self, name: &str, schema: &Value) -> String {
        let mut out = String::new();
        if let Some(comment) = proto_comment(schema.get("description").and_then(|v| v.as_str())) {
            out.push_str(&comment);
        }

        if let Some(variants) = non_null_union(schema) {
            out.push_str(&format!("message {name} {{\n"));
            out.push_str("  oneof sum {\n");
            let mut used = BTreeSet::new();
            for (idx, variant) in variants.iter().enumerate() {
                let (field, ty) = self.variant_field(name, variant, idx);
                let field = unique_ident(&field, &mut used);
                let ty = if ty.repeated {
                    let wrap = pascal_ident(&format!("{name}_{field}_list"));
                    let item = ty.ty.clone();
                    self.messages.insert(
                        wrap.clone(),
                        format!("message {wrap} {{\n  repeated {item} items = 1;\n}}\n"),
                    );
                    self.done.insert(wrap.clone());
                    wrap
                } else {
                    ty.ty
                };
                out.push_str(&format!("    {ty} {field} = {};\n", idx + 1));
            }
            out.push_str("  }\n}\n");
            return out;
        }

        if let Some(allof) = schema.get("allOf").and_then(|v| v.as_array()) {
            if allof.len() == 1 {
                let inner = self.field_type(&allof[0], name, "value");
                out.push_str(&format!("message {name} {{\n"));
                let card = if inner.repeated { "repeated " } else { "" };
                out.push_str(&format!("  {card}{} value = 1;\n}}\n", inner.ty));
                return out;
            }
        }

        out.push_str(&format!("message {name} {{\n"));
        if let Some(props) = schema.get("properties").and_then(|v| v.as_object()) {
            let required: Vec<&str> = schema
                .get("required")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|r| r.as_str()).collect())
                .unwrap_or_default();
            let mut used = BTreeSet::new();
            for (idx, (field_name, field_schema)) in props.iter().enumerate() {
                let ty = self.field_type(field_schema, name, field_name);
                let field = unique_ident(&sanitize_field_name(field_name), &mut used);
                let label = if ty.repeated {
                    "repeated "
                } else if ty.optional || !required.contains(&field_name.as_str()) {
                    "optional "
                } else {
                    ""
                };
                out.push_str(&format!(
                    "  {label}{} {field} = {};\n",
                    ty.ty,
                    idx + 1
                ));
            }
        } else if let Some(scalar) = scalar_newtype(schema) {
            out.push_str(&format!("  {scalar} value = 1;\n"));
        }
        out.push_str("}\n");
        out
    }

    fn variant_field(&mut self, parent: &str, variant: &Value, idx: usize) -> (String, ProtoType) {
        if let Some(tag) = unit_variant_name(variant) {
            return (
                sanitize_field_name(&tag),
                ProtoType {
                    ty: self.unit_type(),
                    repeated: false,
                    optional: false,
                },
            );
        }
        if let Some(props) = variant.get("properties").and_then(|v| v.as_object()) {
            if props.len() == 1 {
                let (field, field_schema) = props.iter().next().unwrap();
                let ty = self.field_type(field_schema, parent, field);
                return (sanitize_field_name(field), ty);
            }
        }
        let field = format!("variant_{}", idx + 1);
        let ty = self.field_type(variant, parent, &field);
        (field, ty)
    }

    fn field_type(&mut self, schema: &Value, parent: &str, field: &str) -> ProtoType {
        self.depth += 1;
        if self.depth > 64 {
            self.depth -= 1;
            return ProtoType {
                ty: "string".to_string(),
                repeated: false,
                optional: false,
            };
        }
        let ty = self.field_type_inner(schema, parent, field);
        self.depth -= 1;
        ty
    }

    fn field_type_inner(&mut self, schema: &Value, parent: &str, field: &str) -> ProtoType {
        if let Some(inner) = unwrap_nullable(schema) {
            let mut ty = self.field_type(inner, parent, field);
            ty.optional = true;
            ty.repeated = false;
            return ty;
        }
        if let Some(allof) = schema.get("allOf").and_then(|v| v.as_array()) {
            if allof.len() == 1 {
                return self.field_type(&allof[0], parent, field);
            }
        }
        if let Some(ref_path) = schema.get("$ref").and_then(|v| v.as_str()) {
            let ref_name = ref_path.rsplit('/').next().unwrap_or(ref_path);
            self.ensure_ref(ref_name);
            return ProtoType {
                ty: sanitize_type_name(ref_name),
                repeated: false,
                optional: false,
            };
        }
        if non_null_union(schema).is_some() {
            let nested = pascal_ident(&format!("{parent}_{field}"));
            self.ensure(&nested, schema);
            return ProtoType {
                ty: nested,
                repeated: false,
                optional: false,
            };
        }
        if let Some(types) = schema.get("type").and_then(|v| v.as_array()) {
            if let Some(t) = types
            .iter()
            .find_map(|t| t.as_str().filter(|s| *s != "null"))
        {
            let mut single = schema.clone();
            if let Some(obj) = single.as_object_mut() {
                obj.insert("type".to_string(), Value::String(t.to_string()));
            }
            let mut ty = self.field_type(&single, parent, field);
            if types.iter().any(|t| t.as_str() == Some("null")) && !ty.repeated {
                ty.optional = true;
            }
            return ty;
        }
        }

        match schema.get("type").and_then(|v| v.as_str()) {
            Some("string") => {
                // A single-value enum is a unit tag, not a new message. Wrapping
                // it again called ensure() on the same schema under a longer name.
                if schema
                    .get("enum")
                    .and_then(|v| v.as_array())
                    .is_some_and(|variants| variants.len() > 1)
                {
                    let nested = pascal_ident(&format!("{parent}_{field}"));
                    self.ensure(&nested, schema);
                    return ProtoType {
                        ty: nested,
                        repeated: false,
                        optional: false,
                    };
                }
                let ty = if schema.get("format").and_then(|v| v.as_str()) == Some("byte") {
                    "bytes"
                } else {
                    "string"
                };
                ProtoType {
                    ty: ty.to_string(),
                    repeated: false,
                    optional: false,
                }
            }
            Some("integer") => ProtoType {
                ty: match schema.get("format").and_then(|v| v.as_str()) {
                    Some("int64" | "int32") => "int64",
                    Some("uint32") => "uint32",
                    _ => "uint64",
                }
                .to_string(),
                repeated: false,
                optional: false,
            },
            Some("number") => ProtoType {
                ty: "string".to_string(),
                repeated: false,
                optional: false,
            },
            Some("boolean") => ProtoType {
                ty: "bool".to_string(),
                repeated: false,
                optional: false,
            },
            Some("array") => {
                let items = schema.get("items").cloned().unwrap_or(Value::Null);
                let inner = if items.is_null() {
                    ProtoType {
                        ty: "string".to_string(),
                        repeated: false,
                        optional: false,
                    }
                } else {
                    self.field_type(&items, parent, field)
                };
                ProtoType {
                    ty: inner.ty,
                    repeated: true,
                    optional: false,
                }
            }
            Some("object") => {
                let empty = schema
                    .get("properties")
                    .and_then(|v| v.as_object())
                    .map(|p| p.is_empty())
                    .unwrap_or(true);
                if empty {
                    return ProtoType {
                        ty: self.unit_type(),
                        repeated: false,
                        optional: false,
                    };
                }
                let nested = pascal_ident(&format!("{parent}_{field}"));
                self.ensure(&nested, schema);
                ProtoType {
                    ty: nested,
                    repeated: false,
                    optional: false,
                }
            }
            _ => ProtoType {
                ty: "string".to_string(),
                repeated: false,
                optional: false,
            },
        }
    }

    fn unit_type(&mut self) -> String {
        if self.defs.contains_key("Unit") {
            self.needs_proto_unit = true;
            "ProtoUnit".to_string()
        } else {
            self.needs_unit = true;
            "Unit".to_string()
        }
    }

    fn render_file(&mut self, package_name: &str) -> String {
        if self.needs_unit && !self.messages.contains_key("Unit") {
            self.messages
                .insert("Unit".to_string(), "message Unit {\n}\n".to_string());
        }
        if self.needs_proto_unit && !self.messages.contains_key("ProtoUnit") {
            self.messages.insert(
                "ProtoUnit".to_string(),
                "message ProtoUnit {\n}\n".to_string(),
            );
        }

        let clean_package = package_name.replace('-', "_").replace('.', "_");
        let mut output = String::new();
        output.push_str("syntax = \"proto3\";\n");
        output.push_str(&format!("package {clean_package}.contract.v1;\n\n"));
        output.push_str("// Include api/proto/terp-proto on the protoc path.\n");
        output.push_str("// That directory vendors the terp-core workspace proto module\n");
        output.push_str("// (terp, osmosis, gaia) plus gogoproto and google.protobuf.\n");
        output.push_str("import \"gogoproto/gogo.proto\";\n\n");
        output.push_str("option (gogoproto.goproto_getters_all) = false;\n\n");

        for body in self.enums.values() {
            output.push_str(body);
            output.push('\n');
        }
        for body in self.messages.values() {
            output.push_str(body);
            output.push('\n');
        }
        output
    }
}

fn collect_defs(value: &Value, out: &mut BTreeMap<String, Value>) {
    match value {
        Value::Object(map) => {
            for key in ["definitions", "$defs"] {
                if let Some(defs) = map.get(key).and_then(|v| v.as_object()) {
                    for (name, schema) in defs {
                        out.entry(name.clone()).or_insert_with(|| schema.clone());
                        collect_defs(schema, out);
                    }
                }
            }
            for (key, child) in map {
                if key != "definitions" && key != "$defs" {
                    collect_defs(child, out);
                }
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_defs(child, out);
            }
        }
        _ => {}
    }
}

fn non_null_union(schema: &Value) -> Option<Vec<Value>> {
    let is_oneof = schema.get("oneOf").is_some();
    let variants = schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .and_then(|v| v.as_array())?;
    let kept: Vec<Value> = variants
        .iter()
        .filter(|v| v.get("type").and_then(|t| t.as_str()) != Some("null"))
        .cloned()
        .collect();
    // A single `oneOf` arm is still an externally tagged variant. A single
    // non-null `anyOf` arm is the nullable pattern and is unwrapped instead.
    if kept.is_empty() || (!is_oneof && kept.len() < 2) {
        None
    } else {
        Some(kept)
    }
}

fn unwrap_nullable(schema: &Value) -> Option<&Value> {
    let variants = schema.get("anyOf").or_else(|| schema.get("oneOf"))?;
    let variants = variants.as_array()?;
    let non_null: Vec<&Value> = variants
        .iter()
        .filter(|v| v.get("type").and_then(|t| t.as_str()) != Some("null"))
        .collect();
    if non_null.len() == 1
        && variants
            .iter()
            .any(|v| v.get("type").and_then(|t| t.as_str()) == Some("null"))
    {
        return Some(non_null[0]);
    }
    None
}

fn unit_variant_name(schema: &Value) -> Option<String> {
    if schema.get("type").and_then(|v| v.as_str()) != Some("string") {
        return None;
    }
    let variants = schema.get("enum")?.as_array()?;
    if variants.len() == 1 {
        variants[0].as_str().map(|s| s.to_string())
    } else {
        None
    }
}

fn scalar_newtype(schema: &Value) -> Option<&'static str> {
    if schema.get("enum").and_then(|v| v.as_array()).is_some_and(|v| v.len() > 1) {
        return None;
    }
    if schema.get("oneOf").is_some() || schema.get("anyOf").is_some() || schema.get("properties").is_some()
    {
        return None;
    }
    match schema.get("type").and_then(|v| v.as_str()) {
        Some("string") => Some(if schema.get("format").and_then(|v| v.as_str()) == Some("byte") {
            "bytes"
        } else {
            "string"
        }),
        Some("integer") => Some(match schema.get("format").and_then(|v| v.as_str()) {
            Some("int64" | "int32") => "int64",
            Some("uint32") => "uint32",
            _ => "uint64",
        }),
        Some("number") => Some("string"),
        Some("boolean") => Some("bool"),
        _ => None,
    }
}

fn enum_body(name: &str, schema: &Value) -> Option<String> {
    if schema.get("oneOf").is_some() || schema.get("anyOf").is_some() {
        return None;
    }
    if schema.get("type").and_then(|v| v.as_str()) != Some("string") {
        return None;
    }
    let variants = schema.get("enum")?.as_array()?;
    if variants.len() <= 1 {
        return None;
    }
    let prefix = name.to_ascii_uppercase();
    let mut out = String::new();
    if let Some(comment) = proto_comment(schema.get("description").and_then(|v| v.as_str())) {
        out.push_str(&comment);
    }
    out.push_str(&format!("enum {name} {{\n"));
    out.push_str(&format!("  {prefix}_UNSPECIFIED = 0;\n"));
    let mut used = BTreeSet::new();
    used.insert(format!("{prefix}_UNSPECIFIED"));
    for (idx, variant) in variants.iter().enumerate() {
        let raw = variant.as_str().unwrap_or("UNKNOWN");
        let value = unique_ident(
            &format!("{prefix}_{}", sanitize_enum_value(raw)),
            &mut used,
        );
        out.push_str(&format!("  {value} = {};\n", idx + 1));
    }
    out.push_str("}\n");
    Some(out)
}

fn proto_comment(text: Option<&str>) -> Option<String> {
    let text = text?.split('\n').next()?.trim();
    if text.is_empty() {
        return None;
    }
    let trimmed: String = text.chars().take(160).collect();
    Some(format!("// {}\n", trimmed.replace("*/", "")))
}

fn pascal_ident(raw: &str) -> String {
    let parts: String = raw
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut chars = s.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    let mut word = first.to_ascii_uppercase().to_string();
                    word.push_str(chars.as_str());
                    word
                }
            }
        })
        .collect();
    sanitize_type_name(&parts)
}

fn sanitize_type_name(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        }
    }
    let out = out.trim_matches('_').to_string();
    let out = if out.is_empty() {
        "Msg".to_string()
    } else {
        out
    };
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("M{out}")
    } else {
        out
    }
}

fn sanitize_enum_value(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
        } else {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        "UNKNOWN".to_string()
    } else if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("V{out}")
    } else {
        out
    }
}

fn sanitize_field_name(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c.to_ascii_lowercase());
        } else {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_string();
    let out = if out.is_empty() {
        "field".to_string()
    } else {
        out
    };
    const RESERVED: &[&str] = &[
        "syntax", "import", "package", "option", "message", "enum", "service", "rpc", "repeated",
        "optional", "required", "oneof", "map", "reserved", "returns", "stream", "to", "true",
        "false", "public", "weak", "extensions", "extend", "group",
    ];
    if RESERVED.contains(&out.as_str()) || out.chars().next().is_some_and(|c| c.is_ascii_digit())
    {
        format!("{out}_field")
    } else {
        out
    }
}

fn unique_ident(base: &str, used: &mut BTreeSet<String>) -> String {
    if used.insert(base.to_string()) {
        return base.to_string();
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{base}_{n}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

/// Generate a Rust type URL registry (compatible with terp-rs proto-gen format).
fn generate_type_url_registry(proto_dir: &Path) -> anyhow::Result<String> {
    let mut output = String::new();
    output.push_str("// Auto-generated type URL registry — do not edit.\n");
    output.push_str("// Run `gen-tools` to regenerate.\n\n");

    // Scan for .proto files and extract message names
    let proto_files: Vec<_> = std::fs::read_dir(proto_dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map_or(false, |ext| ext == "proto"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    for entry in &proto_files {
        let path = entry.path();
        if let Ok(content) = std::fs::read_to_string(&path) {
            let package = content
                .lines()
                .find(|l| l.trim().starts_with("package "))
                .and_then(|l| {
                    l.trim()
                        .strip_prefix("package ")
                        .and_then(|s| s.strip_suffix(';'))
                })
                .unwrap_or("unknown");

            for line in content.lines() {
                if line.trim().starts_with("message ") {
                    let msg_name = line
                        .trim()
                        .strip_prefix("message ")
                        .and_then(|s| s.split('{').next())
                        .map(|s| s.trim())
                        .unwrap_or("");
                    if !msg_name.is_empty() {
                        let type_url = format!("/{}.{}", package, msg_name);
                        output.push_str(&format!(
                            "pub const {}_TYPE_URL: &str = \"{}\";\n",
                            msg_name.to_uppercase(),
                            type_url
                        ));
                    }
                }
            }
        }
    }

    Ok(output)
}

/// Copy chain `.proto` files into `api/proto/terp-proto/`.
///
/// Source of truth is the terp-core workspace `proto/` module (the tree that
/// contains `terp/drip`). `gogoproto` and `google/protobuf` are copied from
/// `crates/terp-rs/proto/vendor`, which is where those imports already live.
fn vendor_chain_protos(proto_out: &Path, workspace_root: &Path) -> anyhow::Result<usize> {
    let dest = proto_out.join("terp-proto");
    std::fs::create_dir_all(&dest)?;
    let mut copied = 0usize;

    if let Some(chain_proto) = find_ancestor_file(workspace_root, &["proto", "terp", "drip", "v1"])
        .as_deref()
        .and_then(|drip_v1| drip_v1.parent()?.parent()?.parent().map(|p| p.to_path_buf()))
    {
        for module in ["terp", "osmosis", "gaia"] {
            let src = chain_proto.join(module);
            if src.is_dir() {
                copy_proto_files(&src, &dest.join(module), &mut copied)?;
            }
        }
        std::fs::write(
            dest.join("SOURCE"),
            format!(
                "Vendored by gen-tools from the terp-core proto module at {}.\n\
                 gogoproto and google/protobuf come from crates/terp-rs/proto/vendor.\n\
                 protoc include path: this directory (api/proto/terp-proto).\n",
                chain_proto.display()
            ),
        )?;
    } else {
        log::warn!(
            "[proto] terp-core proto module not found from {}",
            workspace_root.display()
        );
    }

    if let Some(gogo) = find_ancestor_file(
        workspace_root,
        &["proto", "vendor", "gogoproto", "gogo.proto"],
    ) {
        if let Some(vendor) = gogo.parent().and_then(|p| p.parent()) {
            for module in ["gogoproto", "google"] {
                let src = vendor.join(module);
                if src.is_dir() {
                    copy_proto_files(&src, &dest.join(module), &mut copied)?;
                }
            }
        }
    } else {
        log::warn!(
            "[proto] gogoproto vendor not found from {}",
            workspace_root.display()
        );
    }

    log::info!(
        "[proto] vendored {} chain proto file(s) into {}",
        copied,
        dest.display()
    );
    Ok(copied)
}

fn find_ancestor_file(start: &Path, rel: &[&str]) -> Option<std::path::PathBuf> {
    let mut cur = Some(start);
    while let Some(dir) = cur {
        let mut candidate = dir.to_path_buf();
        for seg in rel {
            candidate.push(seg);
        }
        if candidate.exists() {
            return Some(candidate);
        }
        cur = dir.parent();
    }
    None
}

fn copy_proto_files(src: &Path, dst: &Path, count: &mut usize) -> anyhow::Result<()> {
    for ent in std::fs::read_dir(src)? {
        let ent = ent?;
        let path = ent.path();
        if path.is_dir() {
            copy_proto_files(&path, &dst.join(ent.file_name()), count)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("proto") {
            std::fs::create_dir_all(dst)?;
            std::fs::copy(&path, dst.join(ent.file_name()))?;
            *count += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn one_schema(value: Value) -> BTreeMap<String, Value> {
        let mut schemas = BTreeMap::new();
        schemas.insert("contract".to_string(), value);
        schemas
    }

    #[test]
    fn cosmwasm_enum_and_defs_are_messages() {
        let schema = json!({
            "contract_name": "terp-passkey",
            "instantiate": {
                "title": "InstantiateMsg",
                "type": "object",
                "required": ["registration"],
                "properties": {
                    "owner": { "type": ["string", "null"] },
                    "registration": { "$ref": "#/definitions/PasskeyRegistration" }
                },
                "definitions": {
                    "PasskeyRegistration": {
                        "type": "object",
                        "required": ["credential_id"],
                        "properties": {
                            "credential_id": { "allOf": [{ "$ref": "#/definitions/Binary" }] },
                            "origin": { "type": ["string", "null"] }
                        }
                    },
                    "Binary": { "type": "string" }
                }
            },
            "execute": {
                "title": "ExecuteMsg",
                "oneOf": [{
                    "type": "object",
                    "required": ["update_ownership"],
                    "properties": {
                        "update_ownership": { "$ref": "#/definitions/Action" }
                    }
                }],
                "definitions": {
                    "Action": {
                        "oneOf": [
                            {
                                "type": "object",
                                "required": ["transfer_ownership"],
                                "properties": {
                                    "transfer_ownership": {
                                        "type": "object",
                                        "required": ["new_owner"],
                                        "properties": {
                                            "new_owner": { "type": "string" },
                                            "expiry": {
                                                "anyOf": [
                                                    { "$ref": "#/definitions/Expiration" },
                                                    { "type": "null" }
                                                ]
                                            }
                                        }
                                    }
                                }
                            },
                            { "type": "string", "enum": ["accept_ownership"] }
                        ]
                    },
                    "Expiration": {
                        "oneOf": [
                            {
                                "type": "object",
                                "required": ["at_height"],
                                "properties": {
                                    "at_height": { "type": "integer", "format": "uint64" }
                                }
                            },
                            {
                                "type": "object",
                                "required": ["never"],
                                "properties": {
                                    "never": { "type": "object", "additionalProperties": false }
                                }
                            }
                        ]
                    }
                }
            }
        });

        let proto = generate_proto_from_schemas(&one_schema(schema), "terp-passkey").unwrap();
        assert!(proto.contains("message InstantiateMsg"), "{proto}");
        assert!(proto.contains("message PasskeyRegistration"), "{proto}");
        assert!(proto.contains("message Binary"), "{proto}");
        assert!(proto.contains("PasskeyRegistration registration = "), "{proto}");
        assert!(proto.contains("optional string owner = "), "{proto}");
        assert!(proto.contains("message ExecuteMsg"), "{proto}");
        assert!(proto.contains("Action update_ownership = "), "{proto}");
        assert!(proto.contains("message Action"), "{proto}");
        assert!(proto.contains("ActionTransferOwnership transfer_ownership = "), "{proto}");
        assert!(proto.contains("accept_ownership = "), "{proto}");
        assert!(proto.contains("message Expiration"), "{proto}");
        assert!(proto.contains("uint64 at_height = "), "{proto}");
        assert!(!proto.contains("google.protobuf.Any"), "{proto}");
    }

    #[test]
    fn passkey_schema_file_emits_sudo_requests() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../contracts/smart-accounts/terp-passkey/schema/terp-passkey.json");
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        let schema: Value = serde_json::from_str(&raw).unwrap();
        let proto = generate_proto_from_schemas(&one_schema(schema), "terp-passkey").unwrap();
        for needle in [
            "message InstantiateMsg",
            "message PasskeyRegistration",
            "message ExecuteMsg",
            "Action update_ownership = ",
            "message QueryMsg",
            "message SudoMsg",
            "message AuthenticationRequest",
            "message OnAuthenticatorAddedRequest",
            "Addr account = ",
        ] {
            assert!(proto.contains(needle), "missing {needle}\n{proto}");
        }
        assert!(!proto.contains("google.protobuf.Any"), "{proto}");
    }
}