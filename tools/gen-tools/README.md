# gen-tools — Terp Network contract type generation runtime

## TODO: generate all artifacts
- wasm contracts
- circuits
- runtime binaries
- docker containers
- metadata files


**gen-tools** is a single Rust binary + library that drives a configurable
pipeline of code generators against one or more CosmWasm workspace roots within
the [Terp Network](https://terp.network) monorepo.

Each pipeline step reads contract JSON schemas (from `cargo schema`) and emits
typed output in one or more target languages or formats.  Steps that find no
type definitions in their input are skipped — gen-tools never produces stub or
empty output files.

gen-tools is the core code-generation runtime for the Terp Network development
workflow.  It replaces the pattern of maintaining independent scripts per output
format with a single, deterministically ordered pipeline.

---

## Dependencies (Terp Network monorepo)

gen-tools lives at `crates/terp-rs/tools/gen-tools/` within the
[terp-core](https://github.com/terpnetwork/terp-core) monorepo.  It is NOT a
standalone crate — it consumes schemas and project structure from the monorepo
layout.

### Direct dependencies (Cargo.toml)

| Dependency | Version / Origin | Purpose |
|---|---|---|
| `serde` + `serde_json` | workspace | Schema JSON parsing |
| `clap` | workspace | CLI argument parsing |
| `anyhow` | workspace | Error propagation |
| `log` / `env_logger` | workspace | Diagnostic output |
| `serde_yaml` | workspace | gen-tools.yaml config loading |
| `glob` | workspace | Schema file discovery |
| `chrono` | workspace | Timestamps in generated headers |

All dependencies come from the workspace Cargo.toml in the terp-core root.
gen-tools does NOT import CosmWasm or Cosmos SDK crates directly — it works
purely from serialized JSON schema files.

### Runtime dependencies (external tools, invoked via subprocess)

| Tool | Required by step | Purpose |
|---|---|---|
| `cargo` + Rust toolchain | `schema` | Runs `cargo schema` per contract |
| `trailmark` binary | `trailmark` | AST hierarchical code graph |
| `githem` binary | `trailmark` | Semantic dependency graph |
| `qmd` binary | `trailmark` | QMD knowledge base probe |

These must be installed separately and on `$PATH`.  The `schema` step uses the
same Rust workspace that the contract lives in (inherits its toolchain).

---

## Getting started (fresh clone)

```bash
# 1. Clone the monorepo
git clone https://github.com/terpnetwork/terp-core.git
cd terp-core/crates/terp-rs/tools/gen-tools

# 2. Build gen-tools
cargo build --release

# 3. Run the full pipeline against ALL projects in gen-tools.yaml
#    (this runs cargo schema, then all generators for each project)
cargo run --release -- --config gen-tools.yaml --all

# 4. Run against a single project
cargo run --release -- --config gen-tools.yaml --project cw-infuser

# 5. Run against a standalone workspace (no config)
cd ../../../cw-infuser
cargo run --release -- --steps schema,go,python,openapi
```

The first run may take several minutes because each contract workspace in
`gen-tools.yaml` runs its own `cargo schema` compilation.  Subsequent runs are
fast — schema output is cached in `schema/` directories until you modify a
contract.

### Output location

Contract clients are written to `api/<lang>/<contract>/` under the terp-rs
workspace (`crates/terp-rs/api`). That directory is gitignored: it is local
output, and this README is the procedure that rebuilds it. Chain module
clients (generated from the terp-core proto module) use the same shape with
the name `terp-<lang>`:

```
crates/terp-rs/api/
  ts/<contract>/
    zod/                 ← Zod schemas
    esbundles/           ← ESM bundles (*.bundle.mjs)
    types/               ← TypeScript message types
    client/              ← query/execute client
    message-composer/
    filters/
    argus/
  ts/terp-ts/zod/        ← chain module Zod (prost-to-zod.sh)
  python/<contract>/     ← contract dataclasses
  python/terp-python/    ← chain module Python (prost-to-pyo3.sh)
  proto/<contract>/      ← contract .proto
  proto/terp-proto/      ← vendored terp-core proto/ plus gogoproto and google
  go/<contract>/
  openapi/<contract>/
  rust/<contract>/
```

`protoc` needs `-I crates/terp-rs/api/proto/terp-proto` so `gogoproto/gogo.proto`,
`google/protobuf/any.proto`, and `terp/...` imports resolve.

---

## Pipeline steps

Each step implements the `Generator` trait and runs in dependency order.
Steps are selected with `--steps <name1,<name2>,...>` or `--steps default`
for all default steps.

| Step | Default | Output dir | What it produces |
|---|---|---|---|
| `schema` | yes | `schema/` (contract-local) | Runs `cargo schema` to produce JSON schema files |
| `ts-codegen` | yes | `api/ts/<contract>/{types,client,message-composer}/` | TypeScript types, query/execute client class, message composers |
| `ts-bundles` | yes | `api/ts/<contract>/{zod,esbundles,client,filters,argus}/` | Zod schemas, ESM bundles, client, websocket filters, Argus formulas |
| `proto` | yes | `api/proto/<contract>/` and `api/proto/terp-proto/` | Contract `.proto` plus a vendor of the workspace `proto/` module |
| `python` | yes | `api/python/<contract>/` | Python dataclasses with JSON encode/decode methods and TYPE_URL fields |
| `zod` | yes | `api/ts/<contract>/zod/` | Zod validation schemas |
| `go` | yes | `api/go/<contract>/` | Go struct definitions with JSON tags using cosmwasm-go-gen type mappings |
| `rust` | yes | `api/rust/<contract>/` | Rust type definitions mirroring the contract's own types |
| `trailmark` | yes | `api/trailmark/` | Invokes Trailmark (AST graph), githem (dependency graph), QMD probe (knowledge base).  Writes agent recipe JSON.  Scans workspace sources and generated `api/` types.  Requires external tools on `$PATH`. |
| `readme` | yes | `docs/api/` | API documentation in Markdown with tables of instantiate/execute/query messages |
| `openapi` | yes | `api/openapi/<contract>/` | OpenAPI 3.0 specs in JSON + YAML with contract endpoints |
| `tensorzero` | opt-in | `api/tz/` | TensorZero function specs, model configs, recipe/episode definitions |
| `mermaid` | opt-in | `api/mermaid/` | Mermaid sequence/entity diagrams (placeholder) |
| `indexer` | opt-in | `api/indexer/` | Indexer formula definitions (placeholder) |
| `tz-episodes` | opt-in | `api/tz/` | TensorZero episode definitions (placeholder) |

### Skip-empty behavior

All generators check whether their input contains type definitions before
writing output.  If a contract has no execute messages, no execute-related
types are written.  If the entire contract has no type definitions at all, no
output file is created for that contract.  This means:

- `go/` and `python/` output directories only contain files with actual types
- No stub files with just package declarations or imports
- The `__init__.py` only re-exports modules that have real content

---

## Configuration

### gen-tools.yaml (multi-project mode)

Place a `gen-tools.yaml` at the gen-tools root.  The existing file at
`crates/terp-rs/tools/gen-tools/gen-tools.yaml` configures all projects in the
Terp Network monorepo:

```yaml
base_path: "../../../.."

projects:
  - name: cw-infuser
    path: crates/cw-infuser

  - name: dao-contracts
    path: crates/dao-contracts
    proto_modules: terp,cosmwasm,cosmos,ibc

  - name: polytone
    path: crates/polytone

  # ... additional projects
```

`base_path` resolves relative to the config file's directory.  Each `path` is
relative to `base_path`.  Output directories default to `api/<lang>/<contract>/`
within each project root. Chain module clients use `api/<lang>/terp-<lang>/`.

### Per-project overrides

```yaml
projects:
  - name: dao-contracts
    path: crates/dao-contracts
    steps: schema,proto,python,go    # Override pipeline steps
    skip_schema: false                # Skip cargo schema invocation
    proto_modules: terp,cosmos,ibc    # Proto module prefixes
    ts_out: custom/ts/src             # Per-step output dir overrides
    go_out: custom/go-types
    py_out: custom/python
    tz_heuristics: path/to/heuristics.toml
    tz_recipes_dir: path/to/recipes/
```

### CLI options

```
Usage: gen-tools [OPTIONS] [WORKSPACE_ROOT]

Arguments:
  [WORKSPACE_ROOT]          Workspace root (default: .)

Options:
  --config <PATH>           Path to gen-tools.yaml project config file
  --project <NAME>          Run only this project from the config
  --all                     Run all projects from the config
  -s, --steps <STEPS>       Comma-separated step names
  -o, --output-dir <DIR>    Base output directory root
  --ts-out <DIR>            TypeScript output directory override
  --py-out <DIR>            Python output directory override
  --go-out <DIR>            Go type output directory override
  --proto-out <DIR>         Proto output directory override
  --openapi-out <DIR>       OpenAPI output directory override
  --proto-modules <MODS>    Proto modules to generate (default: terp,osmosis,ibc,cosmos)
  --skip-schema             Skip cargo schema invocation
  -v, --verbose             Verbose logging
```

---

## Library usage

gen-tools exports the `Generator` trait and `pipeline::Pipeline` for embedding
generation into other Rust tools:

```rust
use gen_tools::config::{build_context, ProjectOverrides};
use gen_tools::pipeline::Pipeline;

let overrides = ProjectOverrides {
    steps: "schema,go,python,openapi".into(),
    skip_schema: true,
    ..Default::default()
};
let ctx = build_context(&workspace_root, &overrides, None)?;
let pipeline = Pipeline::new("default");
let results = pipeline.run(&ctx);

for r in &results {
    match r {
        Ok(rr) => println!("  OK  {} — {} files", rr.name, rr.files_generated),
        Err(e) => println!("  FAIL  {} — {}", r.as_ref().unwrap().name, e),
    }
}
```

---

## Adding a new step

1. Create `src/<name>.rs` implementing the `Generator` trait:

```rust
use crate::{GenerationResult, Generator, config::GenerationContext};

pub struct MyGenerator;

impl Generator for MyGenerator {
    fn name(&self) -> &'static str { "my-step" }
    fn enabled_by_default(&self) -> bool { false }
    fn generate(&self, ctx: &GenerationContext) -> anyhow::Result<GenerationResult> {
        // ... generate files from ctx.workspace_root ...
        Ok(GenerationResult {
            name: "my-step",
            success: true,
            files_generated: n,
            output_dir: Some(path),
            message: None,
        })
    }
}
```

2. Register it in `pipeline.rs` by adding to the `generators` vec in
   `Pipeline::new()`.
3. Add to the default step set in `config.rs` `parse_steps()` if it should run
   by default.
4. Add a row to the step table above.
5. Build and test: `cargo build && cargo test`.

---

## Architecture

```
                      ┌─────────────────────┐
                      │     gen-tools        │
                      │   (binary / lib)     │
                      └──────────┬───────────┘
                                 │
               ┌─────────────────┴──────────────────┐
               │              config                  │
               │   CLI args / gen-tools.yaml          │
               └─────────────────┬───────────────────┘
                                 │
               ┌─────────────────┴──────────────────┐
               │            pipeline                  │
               │      step filter + execution order   │
               └──┬──────────┬──────────┬──────────┬──┘
                  │          │          │          │
            ┌─────┴──┐  ┌───┴────┐  ┌──┴────┐  ┌──┴───────┐
            │schema  │  │ts-code │  │ts-bun │  │trailmark │
            │        │  │-gen    │  │-dles  │  │+ githem  │
            └───┬────┘  └───┬────┘  └──┬────┘  └──┬───────┘
                │           │          │          │
          ┌─────┴┐    ┌─────┴─────┐    │     ┌─────┴──────┐
          │cargo │    │ *.types.ts│    │     │trailmark   │
          │schema│    │ *.client  │    │     │structural  │
          │*.json│    │ *.message │    │     │githem graph│
          └──────┘    │ -composer │    │     │QMD probe   │
                      │ *.zod.ts  │    │     │agent recipe│
                      │ *.bundle  │    │     └────────────┘
                      │ *.filters │    │
                      │ *.argus   │    │
                      └───────────┘    │
                                ┌──────┴────────┐
                                │ ts-codegen -> │
                                │ ts-bundles    │
                                │ (overwrites   │
                                │ .client.ts)   │
                                └───────────────┘
```

`ts-codegen` and `ts-bundles` both produce `.client.ts` — ts-bundles runs after
ts-codegen and overwrites it with an enhanced transact+query client class.  All
other files are additive and don't conflict.

---

## Generated output conventions

### Python

Each contract becomes a Python module file.  Objects are `@dataclass` classes
with TYPE_URL fields.  Encode/decode methods use `contract_proto._native` for
Cosmos SDK protobuf encoding:

```python
from terp_api.python.cw_infuser import ExecuteMsg, QueryMsg, ExecuteMsgCreate

msg = ExecuteMsgCreate(recipient="terp1...", amount="1000")
encoded: bytes = msg.encode()
```

### Go

Each contract becomes a separate `.go` file with its own package.  Types use
the cosmwasm-go-gen mapping conventions (Addr -> string, Uint128 -> string,
Binary -> []byte).  OneOf enums are rendered as structs with nullable pointer
fields.

### TypeScript / Zod

TypeScript types are generated alongside Zod runtime validation schemas.
Lightweight ESM bundles allow direct browser/Node imports without a bundler:

```typescript
import { CwInfuserClient } from './api/ts/cw-infuser/client/CwInfuser.client';
import { CwInfuserExecuteMsg } from './api/ts/cw-infuser/zod/CwInfuser.zod';
```

---

## Second wave: LLM runtime

The `src/llm.rs` module defines the interface for LLM-powered generation
targets (mermaid diagrams, indexer formulas, tz-episodes).  Currently a stub
returning placeholders.

- Providers: TensorZero gateway (local port 3000), OpenAI, Anthropic
- Each prompt is constructed from contract schema types and project context
- Output is validated against expected formats before writing

Enable LLM mode by setting `LLM_API_KEY` or configuring a provider in the
TensorZero config under `_devops/tz-profiles/`.

---

## Related crates in the Terp Network ecosystem

- [cw-infuser](https://github.com/terpnetwork/terp-core/tree/main/crates/cw-infuser)
  — CosmWasm contract deployment framework.  Its TS codegen pattern inspired
  gen-tools' `ts-codegen` and `ts-bundles` steps.

- [dao-contracts](https://github.com/terpnetwork/terp-core/tree/main/crates/dao-contracts)
  — DAO framework contracts.  `generate_api_markdown()` informed the `readme`
  step.

- [O-Line](https://github.com/terpnetwork/o-line)
  — Terp Network deployment orchestrator (Akash phases A-E).

- [terp-rs](https://github.com/terpnetwork/terp-core/tree/main/crates/terp-rs)
  — Rust SDK for Terp Network, including NIP-{06,19,21,65} implementations and
  the `scripts/gen/` pipeline that gen-tools replaces.

- [Trailmark](https://github.com/terpnetwork/trailmark)
  — Semantic code graph analysis, invoked by the `trailmark` step.

- Argus indexer — Event indexing framework consuming `.argus.ts` formulas
  produced by `ts-bundles`.

---

## License

Apache 2.0 — see [LICENSE](../../LICENSE) in the terp-core repository root.