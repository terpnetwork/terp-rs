# svg-mint

Own git workspace for the permissionless SVG mint desk. Not an internal package of `terp-core`.

| Path | What |
|---|---|
| `crates/dregg` | DREGG **library** (cell, checkout, commitments, watch helpers) |
| `crates/svg-mint-fed` | HTTP runtime. `dregg = { path = "../dregg" }` |
| `crates/svg-mint-zrt` | Product ZEC CLI (balance / sync / send). Never zec.rocks |
| `crates/cw-dregg-svg-gate` | CosmWasm mint gate |
| `docs/vocs/pages` | Product docs (compile-time import into docs.permissionless.money) |
| `runtimes/zakura` | git submodule — house Zcash node |
| `runtimes/ztreamer` | git submodule — CompactTxStreamer |

```bash
git submodule update --init --depth 1
cargo test -p dregg --lib
cargo build --release -p svg-mint-fed -p svg-mint-zrt
```

House bind: fed `127.0.0.1:8421`, zrt talks to ztreamer `127.0.0.1:9067`.
