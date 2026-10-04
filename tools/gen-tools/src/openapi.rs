//! OpenAPI/Swagger specification generation from CosmWasm schemas.
//!
//! Generates an OpenAPI 3.0 specification from contract schema JSON files,
//! enabling Swagger UI and code-first API documentation.

use crate::config::{contract_api_dir, GenerationContext};
use crate::resolver::SourceResolver;
use crate::{GenerationResult, Generator};
use serde::Serialize;
use std::collections::BTreeMap;

pub struct OpenApiGenerator;

impl Generator for OpenApiGenerator {
    fn name(&self) -> &'static str {
        "openapi"
    }

    fn enabled_by_default(&self) -> bool {
        true
    }

    fn generate(&self, ctx: &GenerationContext) -> anyhow::Result<GenerationResult> {
        std::fs::create_dir_all(&ctx.openapi_out)?;

        let mut total_files = 0;

        let (_label, groups) = SourceResolver::resolve_grouped(ctx);
        if groups.is_empty() {
            log::warn!("[openapi] No type sources found");
            return Ok(GenerationResult {
                name: "openapi",
                success: true,
                files_generated: 0,
                output_dir: Some(ctx.openapi_out.to_string_lossy().to_string()),
                message: Some("No type sources found, skipping".into()),
            });
        }

        for (contract_name, schemas) in &groups {
            let contract_dir = contract_api_dir(&ctx.openapi_out, contract_name);
            std::fs::create_dir_all(&contract_dir)?;
            let spec = generate_openapi_spec(schemas, contract_name)?;
            let spec_path = contract_dir.join(format!("{}.openapi.json", contract_name));
            let spec_content = serde_json::to_string_pretty(&spec)?;
            std::fs::write(&spec_path, &spec_content)?;
            total_files += 1;
            log::info!("[openapi] Wrote {:?}", spec_path);

            let yaml_path = contract_dir.join(format!("{}.openapi.yaml", contract_name));
            let yaml_content = serde_yaml::to_string(&spec)?;
            std::fs::write(&yaml_path, &yaml_content)?;
            total_files += 1;
        }

        if total_files > 0 {
            let combined = generate_combined_openapi_spec(&groups)?;
            let combined_json = serde_json::to_string_pretty(&combined)?;
            std::fs::write(ctx.openapi_out.join("openapi.json"), &combined_json)?;
            total_files += 1;
        }

        Ok(GenerationResult {
            name: "openapi",
            success: true,
            files_generated: total_files,
            output_dir: Some(ctx.openapi_out.to_string_lossy().to_string()),
            message: Some(format!("{} OpenAPI files generated", total_files)),
        })
    }
}

#[derive(Serialize)]
struct OpenApiSpec {
    openapi: String,
    info: Info,
    paths: BTreeMap<String, BTreeMap<String, Operation>>,
    components: Components,
}

#[derive(Serialize)]
struct Info {
    title: String,
    version: String,
    description: String,
}

#[derive(Serialize)]
struct Operation {
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_body: Option<RequestBody>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    parameters: Vec<Parameter>,
    responses: BTreeMap<String, Response>,
}

#[derive(Serialize)]
struct RequestBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    required: bool,
    content: BTreeMap<String, MediaType>,
}

#[derive(Serialize)]
struct MediaType {
    schema: SchemaRef,
}

#[derive(Serialize)]
struct Parameter {
    name: String,
    #[serde(rename = "in")]
    location: String,
    required: bool,
    schema: SchemaRef,
}

#[derive(Serialize)]
struct Response {
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<BTreeMap<String, MediaType>>,
}

#[derive(Serialize)]
struct Components {
    schemas: BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize)]
struct SchemaRef {
    #[serde(rename = "$ref", skip_serializing_if = "Option::is_none")]
    ref_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    type_field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    items: Option<Box<SchemaRef>>,
}

fn generate_openapi_spec(
    schemas: &BTreeMap<String, serde_json::Value>,
    contract_name: &str,
) -> anyhow::Result<OpenApiSpec> {
    let mut paths = BTreeMap::new();

    // Instantiate path: POST /instantiate
    if let Some(instantiate) = find_schema_by_name(&schemas, "InstantiateMsg") {
        let mut ops = BTreeMap::new();
        ops.insert(
            "post".to_string(),
            Operation {
                summary: Some("Instantiate contract".into()),
                description: instantiate
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                request_body: Some(RequestBody {
                    description: Some("Instantiate message".into()),
                    required: true,
                    content: {
                        let mut c = BTreeMap::new();
                        c.insert(
                            "application/json".into(),
                            MediaType {
                                schema: SchemaRef {
                                    ref_path: Some("#/components/schemas/InstantiateMsg".into()),
                                    type_field: None,
                                    items: None,
                                },
                            },
                        );
                        c
                    },
                }),
                parameters: vec![],
                responses: {
                    let mut r = BTreeMap::new();
                    r.insert(
                        "200".into(),
                        Response {
                            description: "Contract instantiated successfully".into(),
                            content: Some({
                                let mut c = BTreeMap::new();
                                c.insert(
                                    "application/json".into(),
                                    MediaType {
                                        schema: SchemaRef {
                                            ref_path: Some("#/components/schemas/InstantiateMsg".into()),
                                            type_field: None,
                                            items: None,
                                        },
                                    },
                                );
                                c
                            }),
                        },
                    );
                    r
                },
            },
        );
        paths.insert("/instantiate".into(), ops);
    }

    // Execute path: POST /execute
    if let Some(execute) = find_schema_by_name(&schemas, "ExecuteMsg") {
        let mut ops = BTreeMap::new();
        ops.insert(
            "post".to_string(),
            Operation {
                summary: Some("Execute contract method".into()),
                description: execute
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                request_body: Some(RequestBody {
                    description: Some("Execute message".into()),
                    required: true,
                    content: {
                        let mut c = BTreeMap::new();
                        c.insert(
                            "application/json".into(),
                            MediaType {
                                schema: SchemaRef {
                                    ref_path: Some("#/components/schemas/ExecuteMsg".into()),
                                    type_field: None,
                                    items: None,
                                },
                            },
                        );
                        c
                    },
                }),
                parameters: vec![],
                responses: {
                    let mut r = BTreeMap::new();
                    r.insert(
                        "200".into(),
                        Response {
                            description: "Execution successful".into(),
                            content: None,
                        },
                    );
                    r
                },
            },
        );
        paths.insert("/execute".into(), ops);
    }

    // Query paths: GET /query/{query_name}
    if let Some(query) = find_schema_by_name(&schemas, "QueryMsg") {
        if let Some(oneof) = query.get("oneOf").and_then(|v| v.as_array()) {
            for variant in oneof {
                if let Some(props) = variant.get("properties").and_then(|v| v.as_object()) {
                    for (field_name, field_schema) in props {
                        let path = format!("/query/{}", field_name);
                        let mut ops = BTreeMap::new();
                        ops.insert(
                            "get".to_string(),
                            Operation {
                                summary: Some(format!("Query {}", field_name)),
                                description: field_schema
                                    .get("description")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string()),
                                request_body: None,
                                parameters: vec![Parameter {
                                    name: field_name.clone(),
                                    location: "query".into(),
                                    required: true,
                                    schema: schema_value_to_ref(field_schema),
                                }],
                                responses: {
                                    let mut r = BTreeMap::new();
                                    r.insert(
                                        "200".into(),
                                        Response {
                                            description: format!("{} response", pascal_case(field_name)),
                                            content: Some({
                                                let mut c = BTreeMap::new();
                                                c.insert(
                                                    "application/json".into(),
                                                    MediaType {
                                                        schema: SchemaRef {
                                                            ref_path: Some(format!(
                                                                "#/components/schemas/{}Response",
                                                                pascal_case(field_name)
                                                            )),
                                                            type_field: None,
                                                            items: None,
                                                        },
                                                    },
                                                );
                                                c
                                            }),
                                        },
                                    );
                                    r
                                },
                            },
                        );
                        paths.insert(path, ops);
                    }
                }
            }
        }
    }

    // Components: re-export all schema types
    let mut component_schemas = BTreeMap::new();
    for (name, schema) in schemas {
        let clean_name = pascal_case(name);
        component_schemas.insert(clean_name, schema.clone());
    }

    Ok(OpenApiSpec {
        openapi: "3.0.3".into(),
        info: Info {
            title: format!("{} API", contract_name),
            version: "1.0.0".into(),
            description: format!("Auto-generated OpenAPI spec for {}", contract_name),
        },
        paths,
        components: Components {
            schemas: component_schemas,
        },
    })
}

fn generate_combined_openapi_spec(
    groups: &[(String, BTreeMap<String, serde_json::Value>)],
) -> anyhow::Result<OpenApiSpec> {
    let mut all_paths = BTreeMap::new();
    let mut all_schemas = BTreeMap::new();

    for (contract_name, schemas) in groups {
        let spec = generate_openapi_spec(schemas, contract_name)?;
        all_paths.extend(spec.paths);
        all_schemas.extend(spec.components.schemas);
    }

    Ok(OpenApiSpec {
        openapi: "3.0.3".into(),
        info: Info {
            title: "Terp Network Contract API".into(),
            version: "1.0.0".into(),
            description: "Combined API for all contracts".into(),
        },
        paths: all_paths,
        components: Components {
            schemas: all_schemas,
        },
    })
}

fn find_schema_by_name<'a>(
    schemas: &'a BTreeMap<String, serde_json::Value>,
    name: &str,
) -> Option<&'a serde_json::Value> {
    schemas
        .iter()
        .find(|(k, _)| k.contains(name))
        .map(|(_, v)| v)
}

fn schema_value_to_ref(schema: &serde_json::Value) -> SchemaRef {
    if let Some(ref_path) = schema.get("$ref").and_then(|v| v.as_str()) {
        let ref_name = ref_path.rsplit('/').next().unwrap_or(ref_path);
        return SchemaRef {
            ref_path: Some(format!("#/components/schemas/{}", pascal_case(ref_name))),
            type_field: None,
            items: None,
        };
    }

    match schema.get("type").and_then(|v| v.as_str()) {
        Some("string") => SchemaRef {
            ref_path: None,
            type_field: Some("string".into()),
            items: None,
        },
        Some("integer") => SchemaRef {
            ref_path: None,
            type_field: Some("integer".into()),
            items: None,
        },
        Some("boolean") => SchemaRef {
            ref_path: None,
            type_field: Some("boolean".into()),
            items: None,
        },
        Some("array") => SchemaRef {
            ref_path: None,
            type_field: Some("array".into()),
            items: schema.get("items").map(|items| {
                Box::new(schema_value_to_ref(items))
            }),
        },
        _ => SchemaRef {
            ref_path: None,
            type_field: Some("object".into()),
            items: None,
        },
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