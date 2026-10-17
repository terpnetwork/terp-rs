//! Build ERGORS proto files. This build script uses the local proto files
//! in the ergors/ directory to build the required proto types for the ERGORS system.
//! This is adapted from the proto-compiler code in github.com/informalsystems/ibc-rs

use std::path::PathBuf;

const SERDE_JSON: &str = "#[derive(serde::Serialize, serde::Deserialize)]";
fn main() -> anyhow::Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    println!("root: {}", root.display());

    let target_dir = root
        .join("../")
        .join("zk-crates/zk-headstash/src")
        .join("gen");

    println!("target_dir: {}", target_dir.display());

    // prost_build::Config isn't Clone, so we need to make two.
    let mut config = prost_build::Config::new();

    // As recommended in pbjson_types docs.
    config.extern_path(".google.protobuf", "::pbjson_types");
    // NOTE: we need this because the rust module that defines the IBC types is external, and not
    // part of this crate.
    // See https://docs.rs/prost-build/0.5.0/prost_build/struct.Config.html#method.extern_path
    config.extern_path(".ibc", "::ibc_proto::ibc");

    config.extern_path(".ics23", "::ics23");
    config.extern_path(".cosmos.ics23", "::ics23");

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
        // Only in Tonic 0.10
        //.generate_default_stubs(true)
        // We need to feature-gate the RPCs.
        .server_mod_attribute(".", rpc_doc_attr)
        .client_mod_attribute(".", rpc_doc_attr)
        .compile_with_config(
            config,
            &[
                "./headstash/extendo/v1/extendo.proto",
                "./headstash/snp/v1/snp.proto",
                "./headstash/actions/v1/actions.proto",
                "./headstash/types/v1/types.proto",
            ],
            &[],
        )?;

    // Finally, build pbjson Serialize, Deserialize impls:
    // let descriptor_set = std::fs::read(target_dir.join(descriptor_file_name))?;

    pbjson_build::Builder::new()
        // .register_descriptors(&descriptor_set)?
        .ignore_unknown_fields()
        .out_dir(&target_dir)
        .build(&["."])?;

    Ok(())
}
