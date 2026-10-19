//! Placeholder generators for future LLM-dependent features.
//!
//! These are second-wave generators that currently emit stubs.
//! The design is ready for LLM runtime invocation (see llm.rs).
//!
//! Placeholder generators:
//!   - mermaid: Protocol flow diagrams from contract schemas
//!   - indexer: Subgraph/GQL indexer formulas for event indexing
//!   - tz-episodes: Temporal zone episode definitions for time-aware contracts

use crate::config::GenerationContext;
use crate::{GenerationResult, Generator};

pub struct MermaidGenerator;

impl Generator for MermaidGenerator {
    fn name(&self) -> &'static str {
        "mermaid"
    }

    fn enabled_by_default(&self) -> bool {
        false // opt-in only; requires LLM in second wave
    }

    fn generate(&self, ctx: &GenerationContext) -> anyhow::Result<GenerationResult> {
        let mermaid_dir = ctx.workspace_root.join("docs").join("diagrams");
        std::fs::create_dir_all(&mermaid_dir)?;

        let mut total_files = 0;

        // For each contract, generate a placeholder mermaid diagram
        for contract in &ctx.contracts {
            let diagram = generate_mermaid_diagram(contract)?;
            let path = mermaid_dir.join(format!("{}.mmd", contract.name));
            std::fs::write(&path, &diagram)?;
            total_files += 1;
            log::info!("[mermaid] Wrote {:?}", path);
        }

        Ok(GenerationResult {
            name: "mermaid",
            success: true,
            files_generated: total_files,
            output_dir: Some(mermaid_dir.to_string_lossy().to_string()),
            message: Some(format!(
                "{} placeholder mermaid diagrams generated (LLM enhancement pending)",
                total_files
            )),
        })
    }
}

/// Generate a placeholder mermaid sequence diagram showing contract flow.
///
/// In the second wave, this will use LLM-based analysis of schema types
/// to produce accurate protocol flow diagrams.
fn generate_mermaid_diagram(contract: &crate::config::ContractMeta) -> anyhow::Result<String> {
    let mut output = String::new();

    output.push_str("sequenceDiagram\n");
    output.push_str("    participant User\n");
    output.push_str(&format!(
        "    participant {}Contract\n",
        pascal_case(&contract.name)
    ));
    output.push_str("    participant Blockchain\n\n");

    // Instantiate flow
    output.push_str("    User->>+{}Contract: instantiate()\n");
    output.push_str(&format!(
        "    {}Contract-->>-User: ContractAddress\n\n",
        pascal_case(&contract.name)
    ));

    // Execute flow
    output.push_str("    User->>+{}Contract: execute()\n");
    output.push_str("    {}Contract->>Blockchain: validate & store state\n");
    output.push_str(&format!(
        "    Blockchain-->>{}Contract: state update\n",
        pascal_case(&contract.name)
    ));
    output.push_str(&format!(
        "    {}Contract-->>-User: success response\n\n",
        pascal_case(&contract.name)
    ));

    // Query flow
    output.push_str("    User->>+{}Contract: query()\n");
    output.push_str(&format!(
        "    {}Contract-->>-User: query result\n\n",
        pascal_case(&contract.name)
    ));

    // Placeholder note
    output.push_str("    %% NOTE: This is a placeholder diagram.\n");
    output.push_str("    %% Run with LLM runtime enabled for semantic generation.\n");

    Ok(output)
}

pub struct IndexerGenerator;

impl Generator for IndexerGenerator {
    fn name(&self) -> &'static str {
        "indexer"
    }

    fn enabled_by_default(&self) -> bool {
        false // opt-in only; requires LLM in second wave
    }

    fn generate(&self, ctx: &GenerationContext) -> anyhow::Result<GenerationResult> {
        let indexer_dir = ctx.workspace_root.join("indexer");
        std::fs::create_dir_all(&indexer_dir)?;

        let mut total_files = 0;

        for contract in &ctx.contracts {
            let formula = generate_indexer_formula(contract)?;
            let path = indexer_dir.join(format!("{}.graphql", contract.name));
            std::fs::write(&path, &formula)?;
            total_files += 1;
            log::info!("[indexer] Wrote {:?}", path);
        }

        Ok(GenerationResult {
            name: "indexer",
            success: true,
            files_generated: total_files,
            output_dir: Some(indexer_dir.to_string_lossy().to_string()),
            message: Some(format!(
                "{} placeholder indexer formulas generated (LLM enhancement pending)",
                total_files
            )),
        })
    }
}

/// Generate a placeholder subgraph schema for indexing contract events.
///
/// Second wave: LLM generates accurate GraphQL entities from schema types.
fn generate_indexer_formula(contract: &crate::config::ContractMeta) -> anyhow::Result<String> {
    let mut output = String::new();

    output.push_str(&format!(
        "# Auto-generated placeholder indexer for {}\n",
        contract.name
    ));
    output.push_str("# Run with LLM runtime for semantic generation.\n\n");

    output.push_str("type Contract @entity {\n");
    output.push_str("  id: ID!\n");
    output.push_str("  address: Bytes!\n");
    output.push_str("  creator: Bytes!\n");
    output.push_str("  created_at: BigInt!\n");
    output.push_str("}\n\n");

    output.push_str("type Event @entity {\n");
    output.push_str("  id: ID!\n");
    output.push_str("  contract: Contract!\n");
    output.push_str("  name: String!\n");
    output.push_str("  data: String!\n");
    output.push_str("  block_number: BigInt!\n");
    output.push_str("  timestamp: BigInt!\n");
    output.push_str("  transaction_hash: Bytes!\n");
    output.push_str("}\n\n");

    output.push_str("# Event handlers (placeholder)\n");
    output.push_str("type EventHandler @entity {\n");
    output.push_str("  id: ID!\n");
    output.push_str("  event: Event!\n");
    output.push_str("  function: String!\n");
    output.push_str("}\n");

    Ok(output)
}

pub struct TzEpisodesGenerator;

impl Generator for TzEpisodesGenerator {
    fn name(&self) -> &'static str {
        "tz-episodes"
    }

    fn enabled_by_default(&self) -> bool {
        false // opt-in only
    }

    fn generate(&self, ctx: &GenerationContext) -> anyhow::Result<GenerationResult> {
        let tz_dir = ctx.workspace_root.join("tz-episodes");
        std::fs::create_dir_all(&tz_dir)?;

        let mut total_files = 0;

        for contract in &ctx.contracts {
            let episodes = generate_tz_episodes(contract)?;
            let path = tz_dir.join(format!("{}.tz.json", contract.name));
            std::fs::write(&path, &episodes)?;
            total_files += 1;
            log::info!("[tz-episodes] Wrote {:?}", path);
        }

        Ok(GenerationResult {
            name: "tz-episodes",
            success: true,
            files_generated: total_files,
            output_dir: Some(tz_dir.to_string_lossy().to_string()),
            message: Some(format!(
                "{} placeholder tz-episode definitions generated",
                total_files
            )),
        })
    }
}

/// Generate placeholder temporal zone episode definitions.
///
/// These define time windows or epochs for time-aware contract logic
/// (e.g., vesting schedules, auction rounds, staking epochs).
///
/// Second wave: LLM extracts temporal semantics from schema types.
fn generate_tz_episodes(contract: &crate::config::ContractMeta) -> anyhow::Result<String> {
    let output = serde_json::to_string_pretty(&serde_json::json!({
        "$schema": "https://terp.network/tz-episode.json",
        "contract": contract.name,
        "description": "Placeholder temporal zone episodes",
        "episodes": [
            {
                "id": "episode-0",
                "name": "Init",
                "start_block": 0,
                "duration_blocks": 100,
                "transitions": ["episode-1"]
            },
            {
                "id": "episode-1",
                "name": "Active",
                "duration_blocks": 1000,
                "transitions": ["episode-2"]
            },
            {
                "id": "episode-2",
                "name": "Settlement",
                "duration_blocks": 100,
                "transitions": []
            }
        ],
        "status": "placeholder",
        "note": "Regenerate with LLM runtime for semantically accurate episode definitions"
    }))?;

    Ok(output)
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
