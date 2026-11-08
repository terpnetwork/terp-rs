# svg-mint (deployed snapshot from groot2)

Exact source of the running svg-mint-fed / svg-mint-zrt service on groot2 (~/abstract/svg-mint, local git HEAD bfd7805 plus uncommitted work), copied 2026-11-08. It is not a member of the terp-rs workspace.

It has diverged from tools/svg-mint (the terp-rs workspace copy):
- groot2 adds svg-mint-zrt wallet.rs (diversified UAs), sessions.rs, and new fed endpoints.
- terp-rs has clippy/rustfmt cleanups, rand 0.9 and workspace dependencies.
- 20 files conflict on a 3-way merge. svg-mint-zrt also uses a different wallet stack (zync-core / zafu-wasm with absolute paths on groot2).

Next step: port this into tools/svg-mint, then delete this directory.

Build on groot2: cargo +stable build --release -p svg-mint-fed -p svg-mint-zrt.

Excluded from the copy: target/, .git, runtimes/* submodule contents (zakura v1.6.0, ztreamer), state, house-wallet*, .env*, keys, *.bak*, ._* files.
