# scripts/just/docs.just
# Documentation and tool generation (adapted from CW-AGENT)

# Generate documentation for a specific module or all
gen-docs module="all":
    @echo "📚 Generating docs for module={{module}}..."
    @bash scripts/gen/gen-docs.sh docs "{{module}}"
    @echo "✅ Documentation generated"

# Generate LLM tool schemas + executor from docs
gen-tools:
    @echo "🔧 Generating LLM tool schemas..."
    @echo "→ Creating basic zk-wasm tools.json"
    @mkdir -p .team/tools
    @printf '%s\n' \
      '{' \
      '  "tools": [' \
      '    {"name": "zk_prove", "description": "Generate zero-knowledge proof using halo2 or arkworks", "parameters": {"type": "object", "properties": {"module": {"type": "string"}, "circuit": {"type": "string"}}}},' \
      '    {"name": "gen_sft", "description": "Generate SFT training data from zk workflows", "parameters": {"type": "object", "properties": {"module": {"type": "string"}}}},' \
      '    {"name": "test_zk", "description": "Run zk-specific tests for terp-core modules", "parameters": {"type": "object", "properties": {"module": {"type": "string"}}}}' \
      '  ]' \
      '}' \
      > .team/tools/zk_wasm_tools.json
    @echo "✅ LLM tools generated in .team/tools/"

# Generate Justfile reference documentation
gen-just:
    @echo "📖 Generating Justfile reference..."
    @just --list --unsorted > docs/just-reference.md
    @echo "✅ Justfile reference written to docs/just-reference.md"

# Full doc pipeline
docs: gen-docs gen-tools gen-just
    @echo "🎉 All documentation and LLM tools updated"
