# Unified privacy-products e2e suite (single ict-rs cargo test)

Home: existing cw-orch suite crate `tests/suite/terp-orch` (no new crate).
- `src/suites/privacy.rs` (feature `privacy`): `ProductModule` trait, `SuiteCtx`, `PrivacySuite::standard()`.
- `tests/privacy_products.rs`: offline (MockBech32) run — **passes**.
- `tests/privacy_products_e2e.rs` (feature `privacy-live`, `#[ignore]`): ict-rs Terp (`120u-1`, uterp) + anvil, zakura via `ZAKURA_RPC_URL` (else synthetic), forces `ICT_PUBLISH_HOST=127.0.0.1`. **Compiles; never run** (needs Docker + `cw_lc_registry.wasm` artifact).

## Plug-in contract (Workstream A)
```rust
impl<C: CwEnv> ProductModule<C> for MyModule {
    fn id(&self) -> &'static str;              // e.g. "auctions-frost", "dao-zk-voting"
    fn workstream(&self) -> Workstream;        // Workstream::A
    fn chains(&self) -> &'static [ChainSlot];  // Terp, Juno, Osmosis, Zakura, Anvil
    fn deploy(&mut self, ctx: &mut SuiteCtx<C>) -> anyhow::Result<()>;
    fn exercise(&mut self, ctx: &mut SuiteCtx<C>) -> anyhow::Result<ProductReport>;
}
```
Replace the matching `PendingSlot` in `PrivacySuite::standard()` with your module; add new deps under the `privacy` feature only. `ctx.operators` gives 3 funded senders (hd 1..3) with ed25519 keys; `ctx.notes` for free-form output. Outcomes: Passed / Pending / Skipped.

Current slots (2026-11-10): lc-registry (B, Passed), auctions-frost (A, Passed), dao-zk-voting (A, Passed), bridge-fcr (B, Pending: FCR draft next, tacit on hold), seams-l0 (B, Passed).

Light-client committee coverage lives in `contracts/light-clients/cw-lc-aggregator/tests/committee.rs` (registry + rotation + aggregator on cw-multi-test). An `lc-committee` suite module is a follow-up.

## Run (on groot-wan, `RUSTC_WRAPPER=kache`, no CARGO_TARGET_DIR)
```
cargo test -p terp-orch --features privacy --test privacy_products
ICT_TEST_MNEMONIC=<from env, never committed> \
  cargo test -p terp-orch --features privacy-live --test privacy_products_e2e -- --ignored --nocapture
```
