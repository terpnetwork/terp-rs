# svg-mint

The permissionless SVG mint desk. This is the source of the service that runs on groot2 (deployed Nov 6 build, groot2 `~/abstract/svg-mint` commit 618a917), ported onto the terp-rs workspace dependencies. `dregg`, `svg-mint-fed` and `cw-dregg-svg-gate` are root terp-rs workspace members. `svg-mint-zrt` is excluded from the root workspace and has its own manifest (see below). Zakura and ztreamer stay git submodules on groot2 (`runtimes/`); they are not vendored here.

| Path | What |
|---|---|
| `crates/dregg` | DREGG **library** (cell, checkout, commitments, watch helpers) |
| `crates/svg-mint-fed` | HTTP runtime. Uses `dregg` and `terp-onboard-authz` (`tools/onboard-authz`) |
| `crates/svg-mint-zrt` | Product ZEC wallet CLI (diversified UAs, `wallet.rs`). Standalone manifest, not a root member: it needs zakura-orchard =1.0.0, tonic 0.11 and zcli `zync-core` / `zcash-wasm`. It builds on groot2 inside `~/abstract/svg-mint` against `~/src/zcli` (a6f1a03). On groot-wan, `crates/zcli` (9386a35) pulls `zcash_keys` 0.16, whose `sha2 =0.11.0-pre.4` conflicts with dregg's k256 0.14, so it does not resolve here yet. |
| `crates/cw-dregg-svg-gate` | CosmWasm mint gate |
| `docs/vocs/pages` | Product docs (compile-time import into docs.permissionless.money) |
| `runtimes/zakura` | git submodule — house Zcash node |
| `runtimes/ztreamer` | git submodule — CompactTxStreamer |

```bash
git submodule update --init --depth 1
# from the terp-rs root
RUSTC_WRAPPER=kache cargo test -p dregg -p svg-mint-fed -p cw-dregg-svg-gate -p terp-onboard-authz
RUSTC_WRAPPER=kache cargo build --release -p svg-mint-fed
```

House bind: fed `127.0.0.1:8421`, zrt talks to the compact server on `127.0.0.1:9067`.

`docs/vocs/pages/wallet-gaps.mdx` records why ZEC recover breaks across restarts. `scripts/e2e-zrt-wallet.sh` is the wallet persistence gate. `scripts/e2e-local-prod.sh` is the groot-wan multi-chain suite (`e2e/`). It runs `svg-mint-zrt` in a temp directory. Do not init `~/.terp/svg-mint-zrt` on the house host with this build.

A checkout note matches that checkout's diversified address or its payment memo. An unmined note is receipt. The same note mints only after one block. The pay step shows the unpaid countdown (`pending_ttl_secs`, default 10 minutes) and hides it once the note is seen. One sentence per step. No raw health JSON on the desk.
