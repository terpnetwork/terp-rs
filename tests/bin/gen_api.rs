// /// Generate a full markdown API reference from the suite manifest + schema files.
// ///
// /// `workspace_root`: path to the repo root containing `contracts/` and `schema/`.
// pub fn generate_api_markdown(workspace_root: &std::path::Path) -> String {
//     let manifest = contract_manifest();
//     let mut md = String::new();

//     // Header
//     md.push_str("# DAO DAO Suite — API Reference\n\n");
//     md.push_str("> Auto-generated from contract schemas and `DaoDaoSuite` registry.\n");
//     md.push_str(
//         "> Regenerate: `cargo test -p dao-testing generate_suite_api_docs -- --ignored`\n\n",
//     );

//     // Collect unique categories in declaration order
//     let categories: Vec<&str> = {
//         let mut cats = Vec::new();
//         for doc in &manifest {
//             if !cats.contains(&doc.category) {
//                 cats.push(doc.category);
//             }
//         }
//         cats
//     };

//     // TOC
//     md.push_str("## Table of Contents\n\n");
//     for cat in &categories {
//         md.push_str(&format!(
//             "- [{}](#{})\n",
//             cat,
//             cat.to_lowercase().replace(' ', "-")
//         ));
//     }
//     md.push('\n');

//     // Summary table
//     md.push_str("## Contract Summary\n\n");
//     md.push_str("| Key | Name | Category | Description |\n");
//     md.push_str("|-----|------|----------|-------------|\n");
//     for doc in &manifest {
//         md.push_str(&format!(
//             "| `{}` | {} | {} | {} |\n",
//             doc.key, doc.name, doc.category, doc.description
//         ));
//     }
//     md.push('\n');

//     // Per-category detailed sections
//     for cat in &categories {
//         md.push_str(&format!("---\n\n## {}\n\n", cat));

//         for doc in manifest.iter().filter(|d| d.category == *cat) {
//             md.push_str(&format!("### {} (`{}`)\n\n", doc.name, doc.key));
//             md.push_str(&format!("> {}\n\n", doc.description));

//             if doc.schema_path.is_empty() {
//                 md.push_str("*No schema available.*\n\n");
//                 continue;
//             }

//             let schema: Option<serde_json::Value> =
//                 std::fs::read_to_string(workspace_root.join(doc.schema_path))
//                     .ok()
//                     .and_then(|s| serde_json::from_str(&s).ok());

//             let schema = match schema {
//                 Some(s) => s,
//                 None => {
//                     md.push_str(&format!("*Schema not found: `{}`*\n\n", doc.schema_path));
//                     continue;
//                 }
//             };

//             if let Some(ver) = schema.get("contract_version").and_then(|v| v.as_str()) {
//                 md.push_str(&format!("**Version**: `{}`\n\n", ver));
//             }

//             // InstantiateMsg
//             if let Some(inst) = schema.get("instantiate") {
//                 let fields = extract_instantiate_fields(inst);
//                 if !fields.is_empty() {
//                     md.push_str("#### InstantiateMsg\n\n");
//                     md.push_str("| Field | Required | Description |\n");
//                     md.push_str("|-------|----------|-------------|\n");
//                     for (name, desc, req) in &fields {
//                         md.push_str(&format!(
//                             "| `{}` | {} | {} |\n",
//                             name,
//                             if *req { "yes" } else { "no" },
//                             truncate(desc, 120)
//                         ));
//                     }
//                     md.push('\n');
//                 }
//             }

//             // ExecuteMsg
//             if let Some(exec) = schema.get("execute") {
//                 let variants = extract_variants(exec);
//                 if !variants.is_empty() {
//                     md.push_str("#### ExecuteMsg\n\n");
//                     md.push_str("| Variant | Description |\n");
//                     md.push_str("|---------|-------------|\n");
//                     for (name, desc) in &variants {
//                         md.push_str(&format!(
//                             "| `{}` | {} |\n",
//                             to_pascal(name),
//                             truncate(desc, 140)
//                         ));
//                     }
//                     md.push('\n');
//                 }
//             }

//             // QueryMsg
//             if let Some(query) = schema.get("query") {
//                 let variants = extract_variants(query);
//                 if !variants.is_empty() {
//                     md.push_str("#### QueryMsg\n\n");
//                     md.push_str("| Variant | Description |\n");
//                     md.push_str("|---------|-------------|\n");
//                     for (name, desc) in &variants {
//                         md.push_str(&format!(
//                             "| `{}` | {} |\n",
//                             to_pascal(name),
//                             truncate(desc, 140)
//                         ));
//                     }
//                     md.push('\n');
//                 }
//             }

//             // MigrateMsg
//             if let Some(mig) = schema.get("migrate") {
//                 if !mig.is_null() {
//                     let variants = extract_variants(mig);
//                     if !variants.is_empty() {
//                         md.push_str("#### MigrateMsg\n\n");
//                         md.push_str("| Variant | Description |\n");
//                         md.push_str("|---------|-------------|\n");
//                         for (name, desc) in &variants {
//                             md.push_str(&format!("| `{}` | {} |\n", to_pascal(name), desc));
//                         }
//                         md.push('\n');
//                     }
//                 }
//             }
//         }
//     }

//     md.push_str("---\n\n");
//     md.push_str(&format!(
//         "*{} contracts across {} categories.*\n",
//         manifest.len(),
//         categories.len()
//     ));
//     md
// }
