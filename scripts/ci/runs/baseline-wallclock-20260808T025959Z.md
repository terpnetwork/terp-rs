# CI wall-clock baseline — 20260808T025959Z

| Field | Value |
|-------|--------|
| date_utc | 20260808T025959Z |
| commit | `37ee97d` (`37ee97d4543db5a4204b85b3a49ac11c2e09ab59`) |
| cwd | `/Users/returniflost/abstract/terp-core/crates/terp-rs` |
| host | `Darwin 25.4.0 arm64` |
| rustc | `rustc 1.98.0-nightly (61d7280f3 2026-06-06)` |
| just | `just 1.34.0` |
| cargo | warm target/ (not clean) after first composite |
| purpose | Pre-parallelization baseline for offline/local CI |
| env_note | Restored missing path crates `crates/frost_escrow` + `crates/threshold_committee` (local stubs) so workspace resolves |

## Baseline table

| Command | Duration (s) | Result | Label |
|---------|-------------:|--------|-------|
| `just ci-core` | 6 | FAIL (exit 101) | just ci-core |
| `just scripts-ibc-preflight offline` | 1 | FAIL (exit 101) | scripts-ibc-preflight offline |
| `just scripts-ibc-offline` | 0 | FAIL (exit 101) | scripts-ibc-offline |
| `just scripts-ibc-validate` | 1 | FAIL (exit 101) | scripts-ibc-validate |
| `cargo test -p terp-auth --lib` | 1 | FAIL (exit 101) | cargo test -p terp-auth --lib |
| `cargo test -p terp-account --lib` | 1 | FAIL (exit 101) | cargo test -p terp-account --lib |
| `cargo test -p crosslink-light-client --lib` | 1 | FAIL (exit 101) | cargo test -p crosslink-light-client --lib |
| `cargo test -p cw721-nips --lib` | 3 | FAIL (exit 101) | cargo test -p cw721-nips --lib |
| `cargo test -p terp-rs --lib` | 2 | FAIL (exit 101) | cargo test -p terp-rs --lib |
| `just ci-extended` | 3 | FAIL (exit 101) | just ci-extended |
| `just act-wire` | 1 | FAIL (exit 1) | just act-wire |

## Copy-paste reproduction

```sh
cd /Users/returniflost/abstract/terp-core/crates/terp-rs
just ci-core          # primary Tier 0 gate
just ci-extended      # Tier 1, no Docker
just act-wire         # workflow list only (seconds)
# optional / fragile on Docker Desktop M-series:
# just act-core
```

## Notes

- Component times after full `just ci-core` are **warm** (second pass).
- Full `just act-core` not measured (Docker Desktop arm64 RWLayer risk; secondary YAML fidelity).
- `just ci-heavy` / multihop harness skipped (not Core gate).
- Live mainnet generate / MAIN_MNEMONIC out of scope.

## Residual GitHub-only risk

- GHA cache restore/save not represented.
- rust-toolchain + setup-just install overhead not in host baseline.
- Extended path filters only on GitHub; host always full tier.
