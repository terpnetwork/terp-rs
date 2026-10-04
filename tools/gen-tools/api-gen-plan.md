# Plan: gen-tools — single Rust runtime for Terp contract codegen

## Status: Phase 1 complete
- Unified CLI (gen-tools) managing 8 projects via gen-tools.yaml
- All outputs default to api/<lang>/<contract>/ per workspace
- Pipeline steps: schema, ts-codegen, proto, python, zod, go, openapi
- Optional (now enabled by default): trailmark, githem, QMD, tensorzero

## Githem / Trailmark integration (just completed)
1. Enabled by default — trailmark generator changed from opt-in to always-run
2. API types included — trailmark structural analysis and githem semantic graph now scan both workspace source AND generated API type directories (ts, py, zod, proto, go, openapi, tz)
3. Multi-target — run_trailmark and run_githem accept &[PathBuf] for multiple scan targets
4. Agent recipe updated — agent-recipe.json includes generated API types in the handoff contract

## Matrix path fix (completed)
- _devops/ folder moved into gen-tools crate
- deps_runner.rs now searches all locations: repo root, crates/, gen-tools/_devops/

## Next: api-gen trait for ict-rs / cw-orchestrator (planned)
- ApiGen trait in src/api_gen.rs (feature-gated as api-gen)
- GeneratorConfig.into_project_overrides() calls existing pipeline
- Test suites impl the trait, call generate_types() before deployment
- Single source of truth: matrix fields extended with gen-tools metadata

## Files changed this session
- src/trailmark.rs — enabled by default, multi-target API type scanning
- src/trailmark.rs — run_trailmark/run_githem updated signatures
- src/trailmark.rs — unused root variables replaced with targets[0]
- src/deps_runner.rs — added gen-tools/_devops matrix path