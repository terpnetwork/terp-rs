//! gen-tools: Unified Rust runtime for CosmWasm contract type generation.
//!
//! Orchestrates a configurable pipeline of code generators:
//!   schema → ts-codegen → proto → python → zod → go → trailmark → readme → openapi
//!
//! Each step is a separate module with a shared `Generator` trait.
//! Steps can be run individually or as a composable pipeline.

pub mod config;
pub mod deps;
pub mod go_gen;
pub mod llm;
pub mod openapi;
pub mod pipeline;
pub mod placeholders;
pub mod proto_gen;
pub mod python_gen;
pub mod readme_api;
pub mod resolver;
pub mod rust_gen;
pub mod schema;
pub mod tensorzero;
pub mod trailmark;
pub mod ts_bundles;
pub mod ts_codegen;
pub mod vocs_docs;
pub mod zod_gen;

// ---------------------------------------------------------------------------
// Generator trait: each pipeline step implements this.
// ---------------------------------------------------------------------------

/// Result of running a single generator step.
#[derive(Debug, Clone)]
pub struct GenerationResult {
    /// Name of the generator (e.g. "schema", "ts-codegen").
    pub name: &'static str,
    /// Whether the step succeeded.
    pub success: bool,
    /// Number of files generated (if applicable).
    pub files_generated: usize,
    /// Output directory (relative or absolute).
    pub output_dir: Option<String>,
    /// Any error or warning message.
    pub message: Option<String>,
}

/// Shared interface for all code generator steps.
pub trait Generator {
    /// Unique name for this generator (used in CLI flag --steps).
    fn name(&self) -> &'static str;

    /// Run the generator. Returns a result describing what happened.
    fn generate(&self, ctx: &config::GenerationContext) -> anyhow::Result<GenerationResult>;

    /// Whether this step is enabled by default.
    fn enabled_by_default(&self) -> bool {
        true
    }
}

/// Convenience: run a slice of generators in order, collecting results.
pub fn run_pipeline(
    generators: &[Box<dyn Generator>],
    ctx: &config::GenerationContext,
) -> Vec<anyhow::Result<GenerationResult>> {
    generators
        .iter()
        .map(|g| {
            log::info!("[pipeline] Running generator: {}", g.name());
            let result = g.generate(ctx);
            match &result {
                Ok(r) => log::info!("  ✓ {} — {} files generated", r.name, r.files_generated),
                Err(e) => log::error!("  ✗ {} — {}", g.name(), e),
            }
            result
        })
        .collect()
}
