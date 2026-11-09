# scripts/just/skills.just
# Skill curation for LLM agents (zk-wasm specific)

# Sync skills to .claude/skills/ and .team/
sync-agents:
    @echo "🔄 Syncing zk-wasm skills to LLM agents..."
    @mkdir -p .claude/skills/zk-wasm .team/skills/zk-wasm
    @cat > .team/skills/zk-wasm/SKILL.md << 'MD'
    # zk-wasm Skill
    #
    # Tools and workflows for zero-knowledge proofs in CosmWasm/terp-core.
    #
    # Phases:
    # 1. Circuit Ingestion (terp-core, zk-airdrop)
    # 2. Proof Generation (halo2/arkworks in zk-wasmvm)
    # 3. Verification (zk-wasmd integration)
    # 4. SFT Data Export for LLM training
    # 5. Documentation + Example Generation
    #
    # Available Tools: zk_prove, gen_sft, test_zk
    MD
    @echo "✅ Skills synced to .team/skills/zk-wasm/ and .claude/skills/"

# Create skill for a specific module
skill module:
    @echo "Creating skill for {{module}}..."
    @mkdir -p .team/skills/{{module}}
    @echo "# {{module}} Skill" > .team/skills/{{module}}/SKILL.md
    @echo "✅ Skill created for {{module}}"
