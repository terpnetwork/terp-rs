//! Build ERGORS proto files. This build script uses the local proto files
//! in the ergors/ directory to build the required proto types for the ERGORS system.
//! This is adapted from the proto-compiler code in github.com/informalsystems/ibc-rs

// push to correct path: was ./src/gen/* from package root, needs to go ../crates/sdk/gen/* for the rust and ../crates/sdk/py/* for the py gen output
// must
use std::path::PathBuf;

const SERDE_JSON: &str = "#[derive(serde::Serialize, serde::Deserialize)]";
fn main() -> anyhow::Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    println!("root: {}", root.display());

    // Output directory for generated Rust source files.
    let target_dir = root.join("../crates").join("sdk").join("src").join("gen");
    std::fs::create_dir_all(&target_dir)?;
    let target_dir = target_dir.canonicalize()?;
    println!("target_dir: {}", target_dir.display());

    // Vendor directory: root/vendor/ (co-located with terp/, gaia/, osmosis/)
    let vendor_dir = root.join("vendor");

    // Proto source directories to compile: terp/, gaia/, osmosis/ (all non-vendor subdirs)
    let excluded_dirs = ["vendor", "src", "target", ".git"];
    let chain_proto_files: Vec<PathBuf> = walkdir::WalkDir::new(&root)
        .min_depth(1)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 1 && e.file_type().is_dir() {
                let name = e.file_name().to_string_lossy();
                return !excluded_dirs.iter().any(|ex| name == *ex);
            }
            true
        })
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "proto"))
        .map(|e| e.path().to_path_buf())
        .collect();
    // Selectively collect ONLY the vendor protos you need generated types for.
    // Skip problematic ones with missing transitive deps (e.g., google/api/servicemanagement)
    let vendor_include_dirs = [
        "cosmos/auth",
        "cosmos/authz",
        "cosmos/bank",
        "cosmos/base/query/v1beta1",
        "cosmos/base/v1beta1",
        "cosmos/feegrant",
        "cosmos/gov",
        "cosmos/staking",
        "cosmos/tx",
        "cosmos/upgrade",
        "cosmos/params",
        "cosmos/crypto",
        "cosmos/msg",
        "ibc/applications",
        "ibc/core",
        "ibc/lightclients",
        "tendermint/abci",
        "tendermint/types",
        "tendermint/p2p",
        "tendermint/crypto",
        "tendermint/version",
        "cosmwasm/wasm",
        "amino",
        "cosmos_proto",
    ];

    let vendor_proto_files: Vec<PathBuf> = walkdir::WalkDir::new(&vendor_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().extension().is_some_and(|ext| ext == "proto")
                && vendor_include_dirs.iter().any(|dir| {
                    e.path()
                        .strip_prefix(&vendor_dir)
                        .map(|p| p.starts_with(dir))
                        .unwrap_or(false)
                })
        })
        .map(|e| e.path().to_path_buf())
        .collect();

    let proto_files: Vec<PathBuf> = chain_proto_files
        .into_iter()
        .chain(vendor_proto_files)
        .collect();

    if proto_files.is_empty() {
        anyhow::bail!(
            "No .proto files found under {}.\nRun `just fetch-protos` first.",
            root.display()
        );
    }

    println!("Compiling {} proto file(s)…", proto_files.len());
    for f in &proto_files {
        println!("  {}", f.display());
    }

    // Include paths:
    //   1. vendor/   — resolves gogoproto/*, cosmos/*, amino/*, google/*, cosmos_proto/*
    //   2. root      — resolves terp/*, gaia/*, osmosis/* (imports use full package path)
    let mut include_paths: Vec<PathBuf> = vec![];
    if vendor_dir.exists() {
        include_paths.push(vendor_dir.clone());
    }
    include_paths.push(root.clone());
    // prost_build::Config isn't Clone, so we need to make two.
    let mut config = prost_build::Config::new();
    // DOWNLOAD AND ENSURE DEFAULT CHAIN PROTOS EXIST IN EXPECTED PATH
    // As recommended in pbjson_types docs.
    config.extern_path(".google.protobuf", "crate");
    config.compile_well_known_types();
    config.type_attribute(".", SERDE_JSON);

    config
        .out_dir(&target_dir)
        // .file_descriptor_set_path(&target_dir.join(descriptor_file_name))
        .enable_type_names();

    let rpc_doc_attr = r#"#[cfg(feature = "rpc")]"#;

    tonic_prost_build::configure()
        .out_dir(&target_dir)
        .emit_rerun_if_changed(false)
        // Feature-gate generated RPC client/server code.
        .server_mod_attribute(".", rpc_doc_attr)
        .client_mod_attribute(".", rpc_doc_attr)
        .compile_with_config(config, &proto_files, &include_paths)?;

    pbjson_build::Builder::new()
        // .register_descriptors(&descriptor_set)?
        .ignore_unknown_fields()
        .out_dir(&target_dir)
        .build(&["."])?;

    // Post-process generated files to remove serde and problematic derives from types
    use std::fs;

    // Types that have serde derives mixed with other derives in the same #[derive(...)]
    // The post-processor will remove serde traits from these derives
    let types_to_remove_serde_from_mixed_derives = [
        "QueryCertificatesRequest",
        "QueryCertificatesResponse",
        "QueryDeploymentsResponse",
        "QueryDeploymentsRequest",
        "Params",
        "ResourceUnit",
        "MsgCreateDeployment",
        "MsgDepositDeployment",
        "GroupSpec",
        "Account",
        "FractionalPayment",
        "Order",
        "MsgCreateBid",
        "Bid",
        "Lease",
        "QueryOrdersRequest",
        "QueryOrdersResponse",
        "QueryBidsRequest",
        "QueryBidsResponse",
        "QueryLeasesRequest",
        "QueryLeasesResponse",
    ];

    // Types that have a separate #[derive(serde::Serialize, serde::Deserialize)] line
    // The post-processor will remove the entire separate serde derive line
    let types_to_remove_separate_serde_derive = ["SctFrontierResponse"];

    for entry in walkdir::WalkDir::new(&target_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.path().extension().is_some_and(|ext| ext == "rs") {
            let content = fs::read_to_string(entry.path())?;
            let lines: Vec<&str> = content.lines().collect();
            let mut new_lines: Vec<String> = Vec::new();
            let mut i = 0;

            while i < lines.len() {
                let line = lines[i];

                // Check if this is a type with serde in mixed derives (same #[derive(...)] line)
                let mut is_mixed_derive_type = false;
                for type_name in &types_to_remove_serde_from_mixed_derives {
                    if line.contains(&format!("struct {}", type_name))
                        || line.contains(&format!("enum {}", type_name))
                    {
                        is_mixed_derive_type = true;
                        break;
                    }
                }

                // Check if this is a type with separate serde derive line
                let mut is_separate_derive_type = false;
                for type_name in &types_to_remove_separate_serde_derive {
                    if line.contains(&format!("struct {}", type_name))
                        || line.contains(&format!("enum {}", type_name))
                    {
                        is_separate_derive_type = true;
                        break;
                    }
                }

                // Handle types with serde mixed in the same derive line
                if is_mixed_derive_type && i > 0 {
                    let prev_line = lines[i - 1];
                    if prev_line.contains("#[derive(") && prev_line.contains("serde::Serialize") {
                        // Remove serde from the derive
                        let new_derive = prev_line
                            .replace(", serde::Serialize", "")
                            .replace("serde::Serialize, ", "")
                            .replace(", serde::Deserialize", "")
                            .replace("serde::Deserialize, ", "")
                            .replace("serde::Serialize", "")
                            .replace("serde::Deserialize", "");

                        if new_derive.contains("#[derive()]") || new_derive == "#[derive" {
                            // Remove the derive line entirely if empty
                            new_lines.pop();
                        } else {
                            // Replace the last added derive line with the cleaned version
                            new_lines.pop();
                            new_lines.push(new_derive);
                        }
                    }
                }

                // Handle types with separate serde derive line
                // Example:
                //   #[derive(serde::Serialize, serde::Deserialize)]  <- Remove this entire line
                //   #[derive(Clone, PartialEq, ::prost::Message)]    <- Keep this
                //   pub struct SctFrontierResponse {
                if is_separate_derive_type && i > 1 {
                    let prev_line = lines[i - 1];
                    let prev_prev_line = lines[i - 2];

                    // Check if i-2 is a serde-only derive and i-1 is a non-serde derive
                    if prev_prev_line.contains("#[derive(")
                        && (prev_prev_line.contains("serde::Serialize")
                            || prev_prev_line.contains("serde::Deserialize"))
                        && prev_line.contains("#[derive(")
                        && !prev_line.contains("serde::")
                    {
                        // Remove the serde derive line (second-to-last in new_lines)
                        if new_lines.len() >= 2 {
                            let last = new_lines.pop().unwrap(); // Pop non-serde derive
                            new_lines.pop(); // Pop serde derive (discard)
                            new_lines.push(last); // Push non-serde derive back
                        }
                    }
                }

                // Check for types containing ibc_proto::cosmos::base::v1beta1::Coin
                // and remove Hash, Eq derives
                if line.contains("#[derive(") && line.contains("Hash") && i < lines.len() - 1 {
                    // Look ahead to find the struct/enum and check if it contains Coin
                    let mut j = i + 1;
                    let mut found_struct = false;
                    let mut struct_start = 0;
                    let mut struct_end = 0;

                    while j < lines.len() {
                        let next_line = lines[j];
                        if next_line.contains("struct ") || next_line.contains("enum ") {
                            found_struct = true;
                            struct_start = j;
                            break;
                        }
                        j += 1;
                    }

                    if found_struct {
                        // Find the end of the struct
                        let mut brace_count = 0;
                        let mut k = struct_start;
                        while k < lines.len() {
                            let struct_line = lines[k];
                            if struct_line.contains("{") {
                                brace_count += struct_line.matches("{").count();
                            }
                            if struct_line.contains("}") {
                                brace_count -= struct_line.matches("}").count();
                            }
                            if brace_count == 0 && k > struct_start {
                                struct_end = k;
                                break;
                            }
                            k += 1;
                        }

                        // Check if the struct contains problematic cosmos types
                        let mut has_problematic_type = false;
                        for l in &lines[struct_start..=struct_end] {
                            if l.contains("ibc_proto::cosmos::base::v1beta1::Coin")
                                || l.contains("ibc_proto::cosmos::base::v1beta1::DecCoin")
                                || l.contains("ibc::core::commitment::v1::MerkleProof")
                                || l.contains("cosmos::base::query::v1beta1::PageRequest")
                                || l.contains("cosmos::base::query::v1beta1::PageResponse")
                                || l.contains("Option<ConsensusState>")
                                || l.contains("IdentifiedClientState")
                            {
                                has_problematic_type = true;
                                break;
                            }
                        }

                        if has_problematic_type && line.contains("Hash") {
                            // Remove Hash and Eq from the derive
                            let new_derive = line
                                .replace(", Hash", "")
                                .replace("Hash, ", "")
                                .replace(" Eq,", "")
                                .replace("Hash", "");

                            new_lines.push(new_derive);
                            i += 1;
                            continue;
                        }
                    }
                }

                new_lines.push(line.to_string());
                i += 1;
            }

            let new_content = new_lines.join("\n");
            if new_content != content {
                fs::write(entry.path(), new_content)?;
            } else {
                fs::write(entry.path(), content)?;
            }
        }
    }

    Ok(())
}
