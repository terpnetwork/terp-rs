//! Pipeline orchestration — maps step names to generators and runs them.

use crate::config::{parse_steps, GenerationContext};
use crate::go_gen::GoGenGenerator;
use crate::rust_gen::RustGenGenerator;
use crate::openapi::OpenApiGenerator;
use crate::vocs_docs::VocsDocsGenerator;
use crate::placeholders::{IndexerGenerator, MermaidGenerator, TzEpisodesGenerator};
use crate::proto_gen::ProtoGenGenerator;
use crate::python_gen::PythonGenGenerator;
use crate::readme_api::ReadmeApiGenerator;
use crate::schema::SchemaGenerator;
use crate::tensorzero::TensorZeroGenerator;
use crate::trailmark::TrailmarkGenerator;
use crate::ts_bundles::TsBundlesGenerator;
use crate::ts_codegen::TsCodegenGenerator;
use crate::zod_gen::ZodGenGenerator;
use crate::{GenerationResult, Generator};

/// All registered generators, keyed by their name().
pub struct Pipeline {
    generators: Vec<Box<dyn Generator>>,
    enabled: Vec<String>,
    disabled: Vec<String>,
}

impl Pipeline {
    /// Build the full pipeline from a --steps string and register all generators.
    pub fn new(steps: &str) -> Self {
        let (enabled, disabled) = parse_steps(steps);

        // Register all known generators (in dependency order).
        let all: Vec<Box<dyn Generator>> = vec![
            Box::new(SchemaGenerator),
            Box::new(TsCodegenGenerator),
            Box::new(TsBundlesGenerator),
            Box::new(ProtoGenGenerator),
            Box::new(PythonGenGenerator),
            Box::new(ZodGenGenerator),
            Box::new(GoGenGenerator),
            Box::new(RustGenGenerator),
            Box::new(TrailmarkGenerator),
            Box::new(ReadmeApiGenerator),
            Box::new(OpenApiGenerator),
            Box::new(VocsDocsGenerator),
            // TensorZero — opt-in (not in default step set)
            Box::new(TensorZeroGenerator::new()),
            // Placeholders — explicitly opt-in only (not in default step set)
            Box::new(MermaidGenerator),
            Box::new(IndexerGenerator),
            Box::new(TzEpisodesGenerator),
        ];

        Pipeline {
            generators: all,
            enabled,
            disabled,
        }
    }

    /// Return the filtered list of generators that should run.
pub fn active_generators(&self) -> Vec<&dyn Generator> {
        self.generators
            .iter()
            .filter(|g| {
                let name = g.name();
                // If enabled list is non-empty, must be in it
                let in_enabled = self.enabled.is_empty() || self.enabled.contains(&name.to_string());
                // Must not be disabled
                let not_disabled = !self.disabled.contains(&name.to_string());
                in_enabled && not_disabled
            })
            .map(|b| b.as_ref())
            .collect()
    }

    /// Run all active generators and collect results.
    pub fn run(&self, ctx: &GenerationContext) -> Vec<anyhow::Result<GenerationResult>> {
        let active = self.active_generators();
        log::info!(
            "[pipeline] Running {} generator(s) on {:?}",
            active.len(),
            ctx.workspace_root
        );

        active
            .iter()
            .map(|g| {
                log::info!("  → {}", g.name());
                let result = g.generate(ctx);
                match &result {
                    Ok(r) => log::info!("  ✓ {} — {} files generated", r.name, r.files_generated),
                    Err(e) => log::error!("  ✗ {} — {}", g.name(), e),
                }
                result
            })
            .collect()
    }

    /// Print a summary of all available generators.
    pub fn list_generators(&self) {
        println!("Available generators:");
        for g in &self.generators {
            let default_label = if g.enabled_by_default() { " [default]" } else { " [opt-in]" };
            println!("  {}{}", g.name(), default_label);
        }
    }

    /// Pretty-print results from a pipeline run.
    pub fn print_results(results: &[anyhow::Result<GenerationResult>]) {
        let total = results.len();
        let successes = results.iter().filter(|r| r.is_ok()).count();
        let failures = total - successes;

        println!();
        println!("═══════════════════════════════════════");
        println!("  Pipeline complete: {successes}/{total} steps ok");
        if failures > 0 {
            println!("  Failures: {failures}");
            for r in results.iter().filter(|r| r.is_err()) {
                if let Err(e) = r {
                    println!("    ✗ {e}");
                }
            }
        }
        println!("═══════════════════════════════════════");

        // Detailed per-step breakdown
        for r in results {
            match r {
                Ok(rr) => {
                    let dir_label = rr
                        .output_dir
                        .as_deref()
                        .unwrap_or("(no output)");
                    println!(
                        "  ✓ {:20} {:>4} files  → {}",
                        rr.name, rr.files_generated, dir_label
                    );
                }
                Err(_e) => {
                    // Already printed above
                }
            }
        }
    }
}