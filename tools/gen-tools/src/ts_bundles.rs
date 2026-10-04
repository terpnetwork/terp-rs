//! TypeScript/JS bundle generation — co-located Zod schemas, lightweight JS bundles,
//! transact query clients, websocket filter tables, and Argus indexer formulas.
//!
//! Generates per-contract files alongside the existing ts-codegen output:
//!   *.zod.ts            — Zod validation schemas (co-located with TS types)
//!   *.bundle.mjs        — Lightweight ESM JS bundle (no bundler deps)
//!   *.client.ts         — Enhanced transact+query client class (overwrites ts-codegen)
//!   *.filters.ts        — Websocket subscription filter formula tables
//!   *.argus.ts          — Argus indexer formula definitions

use crate::config::{ts_variant_dir, GenerationContext};
use crate::resolver::SourceResolver;
use crate::ts_codegen;
use crate::{GenerationResult, Generator};
use serde_json::Value;
use std::collections::HashMap;

pub struct TsBundlesGenerator;

impl Generator for TsBundlesGenerator {
    fn name(&self) -> &'static str {
        "ts-bundles"
    }
    fn enabled_by_default(&self) -> bool {
        true
    }

    fn generate(&self, ctx: &GenerationContext) -> anyhow::Result<GenerationResult> {
        std::fs::create_dir_all(&ctx.ts_out)?;
        let mut total_files = 0;

        let (_source_label, grouped) = SourceResolver::resolve_grouped(ctx);
        if grouped.is_empty() {
            return Ok(GenerationResult {
                name: "ts-bundles",
                success: true,
                files_generated: 0,
                output_dir: Some(ctx.ts_out.to_string_lossy().to_string()),
                message: Some("No contracts found".to_string()),
            });
        }

        for (contract_name, schemas) in &grouped {
            let mut all_defs: HashMap<String, Value> = HashMap::new();
            let mut message_types: Vec<(String, Value)> = Vec::new();
            let mut response_types: Vec<(String, Value)> = Vec::new();

            for (_type_key, schema_value) in schemas {
                ts_codegen::collect_types_and_defs(
                    schema_value,
                    &mut message_types,
                    &mut response_types,
                    &mut all_defs,
                );
            }

            let cname = pascal_case(contract_name);
            let zod_dir = ts_variant_dir(&ctx.ts_out, contract_name, "zod");
            let bundle_dir = ts_variant_dir(&ctx.ts_out, contract_name, "esbundles");
            let client_dir = ts_variant_dir(&ctx.ts_out, contract_name, "client");
            let filters_dir = ts_variant_dir(&ctx.ts_out, contract_name, "filters");
            let argus_dir = ts_variant_dir(&ctx.ts_out, contract_name, "argus");

            // Skip non-contract entries (e.g. raw response types that don't have Execute/Query)
            let has_exec = message_types.iter().any(|(t, _)| t == "ExecuteMsg");
            let has_query = message_types.iter().any(|(t, _)| t == "QueryMsg");
            if !has_exec && !has_query {
                std::fs::create_dir_all(&zod_dir)?;
                let zod = generate_co_located_zod(&cname, &all_defs);
                std::fs::write(zod_dir.join(format!("{}.zod.ts", cname)), &zod)?;
                total_files += 1;
                continue;
            }

            std::fs::create_dir_all(&zod_dir)?;
            std::fs::create_dir_all(&bundle_dir)?;
            std::fs::create_dir_all(&client_dir)?;
            std::fs::create_dir_all(&filters_dir)?;
            std::fs::create_dir_all(&argus_dir)?;

            let zod = generate_co_located_zod(&cname, &all_defs);
            std::fs::write(zod_dir.join(format!("{}.zod.ts", cname)), &zod)?;
            total_files += 1;

            let bundle = generate_js_bundle(&cname, &message_types);
            std::fs::write(bundle_dir.join(format!("{}.bundle.mjs", cname)), &bundle)?;
            total_files += 1;

            let client = generate_transact_client(&cname, &message_types, &response_types);
            std::fs::write(client_dir.join(format!("{}.client.ts", cname)), &client)?;
            total_files += 1;

            let filters = generate_filter_table(&cname, &message_types);
            std::fs::write(filters_dir.join(format!("{}.filters.ts", cname)), &filters)?;
            total_files += 1;

            let argus = generate_argus_indexer(&cname);
            std::fs::write(argus_dir.join(format!("{}.argus.ts", cname)), &argus)?;
            total_files += 1;
        }

        Ok(GenerationResult {
            name: "ts-bundles",
            success: true,
            files_generated: total_files,
            output_dir: Some(ctx.ts_out.to_string_lossy().to_string()),
            message: Some(format!("{} bundle files generated", total_files)),
        })
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 1) Co-located Zod schemas
// ═══════════════════════════════════════════════════════════════════════════

fn generate_co_located_zod(cname: &str, all_defs: &HashMap<String, Value>) -> String {
    let mut out = String::new();
    out.push_str(&format!("// Zod schemas for {}\n", cname));
    out.push_str("import { z } from 'zod';\n\n");

    let mut def_names: Vec<&String> = all_defs.keys().collect();
    def_names.sort();
    for name in &def_names {
        let val = &all_defs[*name];
        let schema_name = format!("{}Schema", pascal_case(name));
        out.push_str(&format!("export const {} = ", schema_name));
        out.push_str(&schema_to_zod(val, all_defs, 0));
        out.push_str(";\n");
    }

    out.push_str(&format!("\nexport const {}Schemas = {{\n", cname));
    for name in &def_names {
        let p = pascal_case(name);
        out.push_str(&format!("  {}: {}Schema,\n", name, p));
    }
    out.push_str("};\n");
    out
}

fn schema_to_zod(val: &Value, defs: &HashMap<String, Value>, depth: usize) -> String {
    if let Some(r) = val.get("$ref").and_then(|v| v.as_str()) {
        let n = r.rsplit('/').next().unwrap_or(r);
        return format!("{}Schema", pascal_case(n));
    }
    if let Some(arr) = val
        .get("oneOf")
        .or_else(|| val.get("anyOf"))
        .and_then(|v| v.as_array())
    {
        if arr.is_empty() {
            return "z.never()".to_string();
        }
        let items: Vec<String> = arr.iter().map(|v| schema_to_zod(v, defs, depth)).collect();
        return items.join(".or(");
    }
    if let Some(arr) = val.get("allOf").and_then(|v| v.as_array()) {
        if arr.is_empty() {
            return "z.unknown()".to_string();
        }
        let items: Vec<String> = arr.iter().map(|v| schema_to_zod(v, defs, depth)).collect();
        return items.join(".and(");
    }
    match val.get("type").and_then(|v| v.as_str()) {
        Some("string") => "z.string()".into(),
        Some("integer") => "z.number().int()".into(),
        Some("number") => "z.number()".into(),
        Some("boolean") => "z.boolean()".into(),
        Some("null") => "z.null()".into(),
        Some("array") => {
            let inner = val
                .get("items")
                .map(|i| schema_to_zod(i, defs, depth))
                .unwrap_or_else(|| "z.unknown()".into());
            format!("z.array({})", inner)
        }
        Some("object") => {
            if let Some(props) = val.get("properties").and_then(|v| v.as_object()) {
                let indent = "  ".repeat(depth + 1);
                let indent_inner = "  ".repeat(depth);
                let fields: Vec<String> = props
                    .iter()
                    .map(|(k, v)| {
                        format!("{}  {}: {},", indent, k, schema_to_zod(v, defs, depth + 1))
                    })
                    .collect();
                format!("z.object({{\n{}\n{}}})", fields.join("\n"), indent_inner)
            } else {
                "z.record(z.string(), z.unknown())".into()
            }
        }
        _ => {
            if let Some(types) = val.get("type").and_then(|v| v.as_array()) {
                let items: Vec<String> = types
                    .iter()
                    .filter_map(|t| t.as_str())
                    .map(|t| match t {
                        "string" => "z.string()",
                        "integer" | "number" => "z.number()",
                        "boolean" => "z.boolean()",
                        "null" => "z.null()",
                        _ => "z.unknown()",
                    })
                    .map(String::from)
                    .collect();
                return items.join(".or(");
            }
            "z.unknown()".into()
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 2) Lightweight ESM JS bundle
// ═══════════════════════════════════════════════════════════════════════════

fn generate_js_bundle(cname: &str, message_types: &[(String, Value)]) -> String {
    let mut out = String::new();
    out.push_str("// Lightweight ESM JS bundle — no bundler deps\n");
    out.push_str(&format!(
        "// Usage: import {{ createApi }} from './{}.bundle.mjs';\n\n",
        cname
    ));

    out.push_str(&format!("export const CONTRACT = {{\n  name: '{}',\n  typeUrl: '/cosmwasm.wasm.v1.MsgExecuteContract',\n}};\n\n", cname));

    let exec_variants = message_types
        .iter()
        .find(|(t, _)| t.contains("Execute"))
        .and_then(|(_, s)| extract_variant_names(s))
        .unwrap_or_default();
    let query_variants = message_types
        .iter()
        .find(|(t, _)| t.contains("Query"))
        .and_then(|(_, s)| extract_variant_names(s))
        .unwrap_or_default();

    out.push_str("export const Execute = {\n");
    for v in &exec_variants {
        out.push_str(&format!("  {}: '{}',\n", camel_case(v), v));
    }
    out.push_str("};\n\n");

    out.push_str("export const Query = {\n");
    for v in &query_variants {
        out.push_str(&format!("  {}: '{}',\n", camel_case(v), v));
    }
    out.push_str("};\n\n");

    out.push_str("export function encodeMsg(msg) {\n");
    out.push_str("  const encoder = new TextEncoder();\n");
    out.push_str("  const json = JSON.stringify(msg);\n");
    out.push_str("  const bytes = encoder.encode(json);\n");
    out.push_str("  return Array.from(bytes);\n");
    out.push_str("}\n\n");

    out.push_str("export function createApi(client, contractAddress) {\n");
    out.push_str("  return {\n    client,\n    contractAddress,\n");
    out.push_str("    query: {\n");
    for v in &query_variants {
        let m = camel_case(v);
        out.push_str(&format!("      async {}(p) {{ return client.queryContractSmart(contractAddress, {{ {}: p }}); }},\n", m, v));
    }
    out.push_str("    },\n    execute: {\n");
    for v in &exec_variants {
        let m = camel_case(v);
        out.push_str(&format!("      async {}(s, p, f) {{ return client.execute(s, contractAddress, {{ {}: p }}, f || []); }},\n", m, v));
    }
    out.push_str("    },\n  };\n}\n");
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// 3) Enhanced transact+query client class
// ═══════════════════════════════════════════════════════════════════════════

fn generate_transact_client(
    cname: &str,
    message_types: &[(String, Value)],
    response_types: &[(String, Value)],
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "import type {{ ExecuteMsg, QueryMsg }} from './{}.types';\n",
        cname
    ));
    out.push_str(
        "import type { CosmWasmClient, SigningCosmWasmClient } from '@cosmjs/cosmwasm-stargate';\n",
    );
    out.push_str("import type { Coin } from '@cosmjs/amino';\n\n");

    let exec_variants = message_types
        .iter()
        .find(|(t, _)| t.contains("Execute"))
        .and_then(|(_, s)| extract_variant_names(s))
        .unwrap_or_default();
    let query_variants = message_types
        .iter()
        .find(|(t, _)| t.contains("Query"))
        .and_then(|(_, s)| extract_variant_names(s))
        .unwrap_or_default();

    let qrm: HashMap<String, String> = response_types
        .iter()
        .map(|(n, _)| (camel_case(n), pascal_case(n)))
        .collect();

    out.push_str(&format!("export class {}Client {{\n", cname));
    out.push_str("  constructor(\n");
    out.push_str("    public readonly client: SigningCosmWasmClient,\n");
    out.push_str("    public readonly contractAddress: string,\n");
    out.push_str("    public readonly sender?: string,\n");
    out.push_str("  ) {}\n\n");

    for v in &query_variants {
        let m = camel_case(v);
        let rt = qrm.get(&m).cloned().unwrap_or_else(|| "unknown".into());
        out.push_str(&format!(
            "  async {}(params: QueryMsg[keyof QueryMsg]): Promise<{}> {{\n",
            m, rt
        ));
        out.push_str("  }\n\n");
    }

    for v in &exec_variants {
        let m = camel_case(v);
        out.push_str(&format!(
            "  async {}Tx(msg: ExecuteMsg[keyof ExecuteMsg], funds?: Coin[]): Promise<string> {{\n",
            m
        ));
        out.push_str("    if (!this.sender) throw new Error('sender required');\n");
        out.push_str(&format!(
            "    const r = await this.client.execute(this.sender, this.contractAddress, {{ {}: msg }}, funds || []);\n", v));
        out.push_str("    return r.transactionHash;\n  }\n\n");
    }

    out.push_str(
        "  static connect(client: SigningCosmWasmClient, addr: string, sender?: string) {\n",
    );
    out.push_str(&format!(
        "    return new {}Client(client, addr, sender);\n",
        cname
    ));
    out.push_str("  }\n}\n");
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// 4) Websocket subscription filter tables
// ═══════════════════════════════════════════════════════════════════════════

fn generate_filter_table(cname: &str, message_types: &[(String, Value)]) -> String {
    let mut out = String::new();
    out.push_str("// Websocket subscription filter tables\n");
    out.push_str("// Use with CosmJS WebsocketClient or StargateClient\n\n");

    out.push_str("export type EventFilter = {\n");
    out.push_str("  type: string;\n");
    out.push_str("  attributes?: Record<string, string>;\n");
    out.push_str("};\n\n");

    let exec_variants = message_types
        .iter()
        .find(|(t, _)| t.contains("Execute"))
        .and_then(|(_, s)| extract_variant_names(s))
        .unwrap_or_default();
    let query_variants = message_types
        .iter()
        .find(|(t, _)| t.contains("Query"))
        .and_then(|(_, s)| extract_variant_names(s))
        .unwrap_or_default();

    out.push_str("export function contractFilter(addr: string): EventFilter {\n");
    out.push_str("  return { type: 'wasm', attributes: { contract_address: addr } };\n");
    out.push_str("}\n\n");

    out.push_str("export const ExecuteFilters = {\n");
    for v in &exec_variants {
        let m = camel_case(v);
        out.push_str(&format!(
            "  {}: (addr: string): EventFilter => ({{ type: 'wasm', attributes: {{ contract_address: addr, action: '{}' }} }}),\n", m, v));
    }
    out.push_str("};\n\n");

    out.push_str("export const QueryFilters = {\n");
    for v in &query_variants {
        let m = camel_case(v);
        out.push_str(&format!(
            "  {}: (addr: string): EventFilter => ({{ type: 'wasm', attributes: {{ contract_address: addr, method: '{}' }} }}),\n", m, v));
    }
    out.push_str("};\n\n");

    out.push_str("export const FilterTable = {\n");
    out.push_str("  wasm: {\n");
    out.push_str("    contract: contractFilter,\n");
    if !exec_variants.is_empty() {
        out.push_str("    execute: ExecuteFilters,\n");
    }
    if !query_variants.is_empty() {
        out.push_str("    query: QueryFilters,\n");
    }
    out.push_str("  },\n} as const;\n");
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// 5) Argus indexer formula definitions
// ═══════════════════════════════════════════════════════════════════════════

fn generate_argus_indexer(cname: &str) -> String {
    let mut out = String::new();
    out.push_str("// Argus indexer formula definitions\n");
    out.push_str("// https://github.com/terpnetwork/argus\n\n");

    out.push_str("export const Entities = {\n");
    out.push_str(&format!("  {}: {{\n", cname));
    out.push_str("    id: 'string',\n    contractAddress: 'string',\n");
    out.push_str("    lastUpdated: 'number',\n    blockHeight: 'number',\n");
    out.push_str("  },\n};\n\n");

    out.push_str("export const Formulas = {\n");
    out.push_str("  instantiate: {\n");
    out.push_str("    trigger: { type: 'wasm', action: 'instantiate' },\n");
    out.push_str(&format!("    entities: ['{}'],\n", cname));
    out.push_str("    fn: 'createEntity',\n  },\n");
    out.push_str("  execute: {\n");
    out.push_str("    trigger: { type: 'wasm', action: 'execute' },\n");
    out.push_str(&format!("    entities: ['{}'],\n", cname));
    out.push_str("    fn: 'updateEntity',\n  },\n};\n\n");

    out.push_str("export const QueryFormulas = {\n");
    out.push_str(&format!(
        "  byContract: 'SELECT * FROM {} WHERE contractAddress = $1',\n",
        cname
    ));
    out.push_str(&format!("  all: 'SELECT * FROM {}',\n", cname));
    out.push_str("};\n");
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════

fn extract_variant_names(schema: &Value) -> Option<Vec<String>> {
    let variants = schema.get("oneOf").and_then(|v| v.as_array())?;
    let names: Vec<String> = variants
        .iter()
        .filter_map(|v| {
            v.get("required")
                .and_then(|a| a.as_array())?
                .first()?
                .as_str()
                .map(|s| s.to_string())
        })
        .collect();
    if names.is_empty() {
        None
    } else {
        Some(names)
    }
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
    let p = pascal_case(s);
    let mut c = p.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_lowercase().to_string() + c.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_zod_defs() {
        let mut defs = HashMap::new();
        defs.insert("Addr".into(), json!({"type": "string"}));
        defs.insert("Uint128".into(), json!({"type": "string"}));
        let zod = generate_co_located_zod("Test", &defs);
        assert!(zod.contains("AddrSchema"));
        assert!(zod.contains("z.string()"));
        assert!(zod.contains("TestSchemas"));
    }

    #[test]
    fn test_js_bundle() {
        let msg = vec![(
            "ExecuteMsg".into(),
            json!({"oneOf": [{"required": ["do_it"], "properties": {}}]}),
        )];
        let b = generate_js_bundle("T", &msg);
        assert!(b.contains("encodeMsg"));
        assert!(b.contains("createApi"));
        assert!(b.contains("doIt"));
    }

    #[test]
    fn test_filter_table() {
        let msg = vec![
            (
                "ExecuteMsg".into(),
                json!({"oneOf": [{"required": ["act"], "properties": {}}]}),
            ),
            (
                "QueryMsg".into(),
                json!({"oneOf": [{"required": ["info"], "properties": {}}]}),
            ),
        ];
        let f = generate_filter_table("T", &msg);
        assert!(f.contains("EventFilter"));
        assert!(f.contains("ExecuteFilters"));
        assert!(f.contains("FilterTable"));
    }

    #[test]
    fn test_transact_client() {
        let msg = vec![
            (
                "ExecuteMsg".into(),
                json!({"oneOf": [{"required": ["go"], "properties": {}}]}),
            ),
            (
                "QueryMsg".into(),
                json!({"oneOf": [{"required": ["peek"], "properties": {}}]}),
            ),
        ];
        let res = vec![("PeekResponse".into(), json!({"title": "PeekResponse"}))];
        let c = generate_transact_client("T", &msg, &res);
        assert!(c.contains("TClient"));
        assert!(c.contains("static connect"));
        assert!(c.contains("goTx"));
        assert!(c.contains("peek"));
    }

    #[test]
    fn test_argus_indexer() {
        let a = generate_argus_indexer("MyContract");
        assert!(a.contains("Entities"));
        assert!(a.contains("Formulas"));
        assert!(a.contains("QueryFormulas"));
        assert!(a.contains("MyContract"));
    }
}
