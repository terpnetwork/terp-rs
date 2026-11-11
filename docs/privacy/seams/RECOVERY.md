# terp-seams recovery (`terp_seams::dex`) — Workstream B

## Where the code was lost
- Loss: terp-rs `0afa462d` ("consolidate five seam fixture crates"); base `f3aae226`.
- Escrow-release API: `stash@{0}` (`ca327972`, 2026-08-06 "sync-auto-stash before preventive push") → `crates/private_dex_seams/src/egress.rs`.
- `lp_seed.rs` (723 lines) + real `threshold_committee` (ECDSA TC01, 514 lines) + real `frost_escrow` (577 lines + spend.rs): untracked-files parent `491ae651` of that stash.
- Harness imports: headstash repo `crates/headstash` @ `94fdce7`; terp-rs `92b81d87` landed them stripped.
- Recovered via `git show <sha>:<path>` only (no checkout/stash ops).

## Restored (uncommitted)
- `crates/terp-seams/src/dex/egress.rs`: 3-way `git merge-file` (clean) adds ThresholdEscrowAuth, authorize_escrow_release, EscrowReleaseError, MODE_*, reject_funder_only_as_product, is_committee_escrow_release_mode + test.
- `crates/terp-seams/src/dex/lp_seed.rs` (new; `crate::` → `crate::dex::`).
- `crates/terp-seams/src/dex/mod.rs`, `crates/terp-seams/Cargo.toml` (+threshold_committee, frost_escrow).
- `crates/threshold_committee`: stub (accepted any non-empty sig!) → real impl; k256 0.14 / rand_core06 adaptation; compat `encoded_public`.
- `crates/frost_escrow` (+ `src/spend.rs`): real impl; kept stub's `ReleaseSpend<'a>` API + `SpendParts`.
- zk-test-press harness (user-dirty files, import-only edits): `lab_pay_zec.rs`, `egress_d.rs`, `seam_negative_contrast.rs`. Pre-edit copies in `ledger/pre-edit/`, patch in `ledger/my-edits-to-user-files.patch`.

## Results
- terp-seams 83+1, threshold_committee 10, frost_escrow 8: pass; clippy -D warnings clean.
- zk-test-press `check --all-targets` (default and `seams-escrow`): ok (was 27 errors).
- Harness: 52 pass / 2 fail, with `ZAKURA_GOLDEN_DEST_BINDING=tests/private-bridge/zakura/golden-dest-binding.json` (default path still points to old `tests/e2e/private-bridge/...`; 7 golden-IO failures without the env var — not edited).
  - `egress_d::lab_degrade_labels_ssot` — original test contradicts `reject_funder_only_as_product_ssot` (pre-existing).
  - `option_d_after_settle_happy_pure_labeled` — gets `frost_escrow_spend_simulated` mode; logic left unchanged.

## Pairs (stabilization target)
zec/btc, zec/um, um/btc, terp/btc, terp/um, terp/zec, thiol/zec, thiol/btc, thiol/um — encoded as `SEAM_PAIRS` in the suite; `seams-l0` module exercises omni bootstrap + FROST 2-of-3 FE01 escrow release. Per-pair swap demo = next step (bridge+swap demo after the tacit mock).

## Fixes (2026-11-10)
Intended behaviour was read from the code's own SSOT docs and tests; logic is unchanged.

1. **Golden default path** (`harness/zakura_local.rs`, clean file, **applied**): c0a786d7 moved the vector to
   `tests/private-bridge/zakura/golden-dest-binding.json`; the default join and the two doc/error strings still said
   `tests/e2e/private-bridge/…` / `docs/plans/spectrum/e2e/…`. Now no `ZAKURA_GOLDEN_DEST_BINDING` is needed.
2. **`egress_d::lab_degrade_labels_ssot` vs `reject_funder_only_as_product_ssot`.** `product_celebrate_forbidden`
   is documented "Inventory funder modes always forbid product celebration" and ORs in
   `reject_funder_only_as_product` (which lists `lab_inventory_pay`). The test asserted the opposite for
   `(cw_bridge_egress, lab_inventory_pay)`. The **test** was wrong: only committee release modes may celebrate on
   the CW egress surface. Fix: assert forbidden for inventory pay; assert allowed for `threshold_escrow_release`
   and `frost_escrow_release` on that surface.
3. **`option_d_after_settle_happy_pure_labeled` → `frost_escrow_spend_simulated`.** It passes alone and fails
   intermittently in the full run (reproduced: 2/3 runs), so it is a process-env race, not a wrong expectation. The
   test pins `CORRIDOR_ZEC_RELEASE=lab_inventory` etc. with no lock, while omni_lp_seed / omni_dual_home /
   lab_pay_zec tests each pinned CORRIDOR_* under **separate per-module** mutexes. The expected
   `lab_inventory_pay_simulated` is correct for its pins. Fix: one crate-wide `CORRIDOR_TEST_ENV_LOCK`
   (defined in `omni_lp_seed.rs`). omni_lp_seed + omni_dual_home use it (clean files, **applied**); lab_pay_zec
   uses it, and the option_d test holds it and also restores `CORRIDOR_LAB_ZEC_FORCE_SIMULATED`, which it leaked.
   These two are user-dirty files, so this part is a **proposed patch only**: `ledger/B-proposed-harness-fixes.patch`.

Verification: a scratch rsync copy (no git, /tmp, deleted after) with the patch applied ran
`cargo +nightly test -p zk-test-press --lib --features seams-escrow` with no env var: **133 passed, 0 failed**, 6/6 runs.
The real tree without the patch gets 131–132 passed, with lab_degrade failing always and option_d flaky.
To apply: `cd crates/terp-rs && patch -p1 < ~/grok-bot/privacy-products/ledger/B-proposed-harness-fixes.patch`.
