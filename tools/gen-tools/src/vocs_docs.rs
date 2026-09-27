//! Vocs MDX + query-first OpenAPI from cw-schema IDLs.
//!
//! Runs `cw-infuser/tools/schema-docs` against **this** project's workspace
//! so dao-contracts, abstract, cw-infuser, etc. all get the same curation.

use crate::config::GenerationContext;
use crate::{GenerationResult, Generator};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct VocsDocsGenerator;

impl Generator for VocsDocsGenerator {
    fn name(&self) -> &'static str {
        "vocs-docs"
    }

    fn enabled_by_default(&self) -> bool {
        true
    }

    fn generate(&self, ctx: &GenerationContext) -> anyhow::Result<GenerationResult> {
        let Some(manifest) = find_schema_docs_manifest(ctx) else {
            log::warn!("[vocs-docs] schema-docs crate not found; skipping");
            return Ok(GenerationResult {
                name: "vocs-docs",
                success: true,
                files_generated: 0,
                output_dir: None,
                message: Some("schema-docs crate not on disk".into()),
            });
        };

        let vocs_out = ctx.workspace_root.join("docs/vocs");
        let mut cmd = Command::new("cargo");
        cmd.args([
            "run",
            "--release",
            "--quiet",
            "--manifest-path",
            &manifest.to_string_lossy(),
            "--",
            "--root",
        ])
        .arg(&ctx.workspace_root)
        .arg("--vocs-out")
        .arg(&vocs_out)
        .arg("--openapi-out")
        .arg(&ctx.openapi_out);
        let status = cmd.status()?;
        if !status.success() {
            anyhow::bail!("schema-docs exited {status}");
        }
        Ok(GenerationResult {
            name: "vocs-docs",
            success: true,
            files_generated: 1,
            output_dir: Some(vocs_out.to_string_lossy().to_string()),
            message: Some(format!("curated {} → {}", ctx.project_name, vocs_out.display())),
        })
    }
}

fn find_schema_docs_manifest(ctx: &GenerationContext) -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SCHEMA_DOCS_MANIFEST") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    let local = ctx.workspace_root.join("tools/schema-docs/Cargo.toml");
    if local.exists() {
        return Some(local);
    }
    // sibling cw-infuser in the crates/ tree
    let sibling = ctx
        .workspace_root
        .parent()
        .map(|p| p.join("cw-infuser/tools/schema-docs/Cargo.toml"));
    if let Some(p) = sibling {
        if p.exists() {
            return Some(p);
        }
    }
    // walk up a few levels from gen-tools itself
    let mut dir: Option<&Path> = Some(&ctx.workspace_root);
    for _ in 0..6 {
        let p = dir?.join("cw-infuser/tools/schema-docs/Cargo.toml");
        if p.exists() {
            return Some(p);
        }
        let p2 = dir?.join("crates/cw-infuser/tools/schema-docs/Cargo.toml");
        if p2.exists() {
            return Some(p2);
        }
        dir = dir?.parent();
    }
    None
}
