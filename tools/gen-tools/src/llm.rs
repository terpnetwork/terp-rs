//! LLM runtime interface for second-wave generators.
//!
//! This module defines the invocation API for LLM-powered code generation.
//! In the second wave, this will support:
//!   - Prompt construction from schema context
//!   - LLM provider abstraction (OpenAI, Anthropic, local)
//!   - Response parsing and code generation
//!
//! Currently, this is a stub ready for implementation.

use crate::config::GenerationContext;

/// Configuration for an LLM provider.
#[derive(Debug, Clone)]
pub struct LlmConfig {
    /// Provider name ("openai", "anthropic", "local", "tensorzero").
    pub provider: String,
    /// Model name (e.g. "gpt-4", "claude-sonnet-4").
    pub model: String,
    /// API key (loaded from env or config).
    pub api_key: Option<String>,
    /// Base URL for API calls.
    pub base_url: Option<String>,
    /// Temperature for generation.
    pub temperature: f32,
    /// Max tokens per response.
    pub max_tokens: u32,
}

/// Prompt context assembled from schema types for LLM generation.
#[derive(Debug)]
pub struct PromptContext {
    /// The type definitions from contract schemas.
    pub schema_context: String,
    /// The generator target (e.g. "mermaid", "indexer").
    pub target: String,
    /// Additional instructions.
    pub instructions: String,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: "tensorzero".into(),
            model: "default".into(),
            api_key: None,
            base_url: Some("http://localhost:3000".into()),
            temperature: 0.3,
            max_tokens: 4096,
        }
    }
}

/// Invoke the LLM runtime to generate code from schema context.
///
/// # Arguments
///
/// * `ctx` - The generation context with workspace info.
/// * `target` - The generation target ("mermaid", "indexer", "tz-episodes").
/// * `schema_context` - Serialized schema types as context.
/// * `instructions` - Additional instructions for the LLM.
///
/// # Returns
///
/// The generated code as a string.
///
/// # Second-wave implementation plan
///
/// 1. Construct a prompt with:
///    - System message describing the task
///    - Schema context (serialized JSON types)
///    - Instructions for output format
/// 2. Call the LLM provider via TensorZero gateway or direct API
/// 3. Parse the response and validate against expectations
/// 4. Return the generated code
pub fn invoke_llm(
    _ctx: &GenerationContext,
    target: &str,
    _schema_context: &str,
    _instructions: &str,
) -> anyhow::Result<String> {
    log::info!("[llm] Invoking LLM for target: {}", target);
    log::info!("[llm] LLM runtime not yet implemented (second wave)");

    // Stub: return a placeholder message
    Ok(format!(
        "# LLM-generated content for {target}\n\
         # This is a placeholder. In the second wave, this\n\
         # will connect to configured LLM providers via\n\
         # the TensorZero gateway or direct API.\n\
         #\n\
         # Configure with:\n\
         #   --llm-provider <provider>\n\
         #   --llm-model <model>\n\
         #   --llm-api-key <key>\n"
    ))
}

/// Check if LLM provider configuration is available.
pub fn llm_available() -> bool {
    // Check for environment variables or config
    std::env::var("LLM_API_KEY").is_ok()
        || std::env::var("OPENAI_API_KEY").is_ok()
        || std::env::var("ANTHROPIC_API_KEY").is_ok()
        || std::env::var("TENSORZERO_API_KEY").is_ok()
}

/// Build prompt context from a generation context for a specific target.
pub fn build_prompt_context(
    ctx: &GenerationContext,
    target: &str,
) -> anyhow::Result<PromptContext> {
    let schema_context = collect_schema_context(ctx)?;

    let instructions = match target {
        "mermaid" => "Generate a Mermaid sequence diagram showing the contract flow.".into(),
        "indexer" => {
            "Generate GraphQL indexer entities for all events emitted by this contract.".into()
        }
        "tz-episodes" => {
            "Extract temporal semantics from the schema and generate episode definitions.".into()
        }
        _ => format!("Generate {target} from the provided schema context."),
    };

    Ok(PromptContext {
        schema_context,
        target: target.to_string(),
        instructions,
    })
}

fn collect_schema_context(ctx: &GenerationContext) -> anyhow::Result<String> {
    let mut context = String::new();

    for contract in &ctx.contracts {
        if !contract.schema_dir.exists() {
            continue;
        }

        context.push_str(&format!("## Contract: {}\n\n", contract.name));

        for entry in std::fs::read_dir(&contract.schema_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "json") {
                let content = std::fs::read_to_string(&path)?;
                let parsed: serde_json::Value = serde_json::from_str(&content)?;
                context.push_str(&format!(
                    "### {}\n```json\n{}\n```\n\n",
                    path.file_stem()
                        .map(|s| s.to_string_lossy())
                        .unwrap_or_default(),
                    serde_json::to_string_pretty(&parsed)?
                ));
            }
        }
    }

    if context.is_empty() {
        context = "No schema context available.".to_string();
    }

    Ok(context)
}
