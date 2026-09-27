# Agent command catalog — package `terp-scripts` (path `tests/`)

**Read this file first** before inventing cargo flags.

| Layer | Package / path | Is |
|-------|----------------|-----|
| SDK | `terp-rs` (`crates/sdk`) | protos + clients |
| Scripts | **`terp-scripts`** (`tests/`) | orchestration + bins + lib |
| Harness | `ict-rs` (sibling crate) | Docker multi-chain framework |

**Stable names (current):** package `terp-scripts`, bin `terp-ibc`.  
Deprecated alias: `--bin ibc` still builds the same binary for one transition window.

## Capability modes

| Mode | Network | Docker | Mnemonic | Use |
|------|---------|--------|----------|-----|
| `offline` | no | no | no | pure lib, goldens, validate, rebuild-from-public |
| `live-query` | yes | no | **required today** (`MAIN_MNEMONIC`) | generate |
| `live-tx` | yes | no | required | deploys / TF mint |
| `docker-harness` | yes | yes | test keys | multihop ignore test |

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | ok |
| 1 | domain failure (invariants / compare) |
| 2 | preflight / wrong mode / missing env |

With `--format json`, stdout is a **RunReport** (`ok`, `exit_code`, `findings`, `artifacts`, …).

## Preferred entry: just

From `crates/terp-rs`:

```sh
just scripts-ibc-offline      # unit+golden + rebuild-from-public
just scripts-ibc-validate     # validate --format json
just scripts-ibc-preflight offline
just scripts-ibc-harness      # Docker, ignored
```

### CI parity (no “trust me bro”)

See `docs/ci.md`. Agents should prefer the same recipes CI runs:

| Tier | Local | When |
|------|-------|------|
| 0 Core | `just ci-core` | always (offline IBC + internal libs) |
| 1 Extended | `just ci-extended` | contracts / crates changed |
| 2 Heavy | `just ci-heavy` | maintainer / Docker only |

GitHub: workflows `ci-core.yml` (every PR), `ci-extended.yml` (paths/labels), `ci-heavy.yml` (dispatch / weekly / label `ci-heavy`).

Local act (Hermes profile `ci-act-tester`): CWD `/Users/returniflost/abstract/terp-core/crates/terp-rs` · brief `docs/superpowers/prompts/ci-act-tester.md` · scripts `scripts/ci/` · host first `just ci-core`.

## Commands

### ibc.preflight.offline

```sh
cargo run -p terp-scripts --bin terp-ibc -- preflight --mode offline --format json
```

- **mode:** offline  
- **inputs:** none  
- **outputs:** RunReport  
- **success:** exit 0, `ok: true`

### ibc.preflight.live-tx

```sh
cargo run -p terp-scripts --bin terp-ibc -- preflight --mode live-tx --format json
```

- **mode:** live-tx  
- **inputs:** env `MAIN_MNEMONIC`  
- **success:** exit 0 if set; exit 2 + `error: missing_env` if not (no cw-orch panic)

### ibc.validate

```sh
cargo run -p terp-scripts --bin terp-ibc -- validate --format json --out /abs/path/to/public
# default --out: repo-root public/ via CARGO_MANIFEST_DIR
```

- **mode:** offline  
- **inputs:** `{out}/ibc-data/*.json`  
- **outputs:** RunReport (findings from hard invariants)  
- **success:** exit 0 and no error findings

### ibc.compare

```sh
cargo run -p terp-scripts --bin terp-ibc -- compare --format json
cargo run -p terp-scripts --bin terp-ibc -- compare --format json --snapshot path/to/snapshot.json --strict
```

- **mode:** offline (fixture golden or local snapshot)  
- **success:** exit 0, no error findings

### ibc.rebuild-from-public

```sh
cargo run -p terp-scripts --bin terp-ibc -- rebuild-from-public --format json --out /abs/path/to/public
```

- **mode:** offline  
- **inputs:** `{out}/ibc-data/*.json` (+ optional `assetlist.json`)  
- **outputs:** atomic write of `ibc_lookup_table.json`, `ibc_routing_table.json`, `ibc_generation_meta.json`; RunReport with artifacts  
- **success:** exit 0, artifacts non-empty when not `--dry-run`  
- **note:** stages under `{out}/_staging/` then promotes; invariants must pass before promote

### ibc.generate

```sh
cargo run -p terp-scripts --bin terp-ibc -- generate --mode live-query
# default subcommand is generate
```

- **mode:** live-query  
- **inputs:** `MAIN_MNEMONIC`, live gRPC  
- **preflight:** missing env → exit 2 JSON (no Daemon panic)  
- **outputs:** live writes under `public/` (legacy relative paths in generate body — prefer rebuild offline)  
- **success:** exit 0 after invariants

### ibc.unit

```sh
cargo test -p terp-scripts --lib --test ibc_unit --test ibc_golden
```

- **mode:** offline  
- **success:** all tests pass

### ibc.harness

```sh
cargo test -p terp-scripts --test ibc_multihop_harness -- --ignored --nocapture
# or: just scripts-ibc-harness
```

- **mode:** docker-harness  
- **inputs:** Docker, Terp image tags  
- **success:** ignored test passes; **not** a substitute for mainnet authenticity without live-query audit

## Global flags (all verbs)

| Flag | Meaning |
|------|---------|
| `--format text\|json` | human vs RunReport JSON (default text) |
| `--out <dir>` | public/output dir (default: repo-root `public/`) |
| `--public-dir <dir>` | legacy alias for `--out` |
| `--dry-run` | no writes (rebuild) |
| `--require-env-file` | preflight fails if env file/mnemonic missing when mode needs it |

## Do not

- `cargo run --bin terp-ibc` without `-p terp-scripts` from random CWD  
- Treat unit green as mainnet authentic  
- Edit goldens silently without audited vs synthetic labels  
- Invent cargo flags when a `just scripts-ibc-*` target exists  
