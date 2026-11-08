# svg-mint

Own Cargo workspace for the permissionless SVG mint desk, nested under `terp-rs/tools/` like `hash-market`. Excluded from the parent terp-rs `[workspace]` members. Zakura and ztreamer stay git submodules (`runtimes/`); they are not vendored here.

| Path | What |
|---|---|
| `crates/dregg` | DREGG **library** (cell, checkout, commitments, watch helpers) |
| `crates/svg-mint-fed` | HTTP runtime. `dregg = { path = "../dregg" }` |
| `crates/svg-mint-zrt` | Product ZEC wallet. One seed, one sqlite ledger. Never zcash-devtool, zec.rocks, or rotko. Scan and prover are not linked. |
| `crates/cw-dregg-svg-gate` | CosmWasm mint gate |
| `docs/vocs/pages` | Product docs (compile-time import into docs.permissionless.money) |
| `runtimes/zakura` | git submodule — house Zcash node |
| `runtimes/ztreamer` | git submodule — CompactTxStreamer |

```bash
git submodule update --init --depth 1
cargo test -p dregg --lib
cargo build --release -p svg-mint-fed -p svg-mint-zrt
```

House bind: fed `127.0.0.1:8421`, zrt talks to the compact server on `127.0.0.1:9067`.

`docs/vocs/pages/wallet-gaps.mdx` records why ZEC recover breaks across restarts. `scripts/e2e-zrt-wallet.sh` is the wallet persistence gate. `scripts/e2e-local-prod.sh` is the groot-wan multi-chain suite (`e2e/`). It runs `svg-mint-zrt` in a temp directory. Do not init `~/.terp/svg-mint-zrt` on the house host with this build.

A checkout note matches that checkout's diversified address or its payment memo. An unmined note is receipt. The same note mints only after one block. The pay step shows the unpaid countdown (`pending_ttl_secs`, default 10 minutes) and hides it once the note is seen. One sentence per step. No raw health JSON on the desk.
