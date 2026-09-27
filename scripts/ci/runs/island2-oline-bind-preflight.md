# Island 2 — oline bind preflight report

**Task:** `t_cb7ea9df` (EIP E4 child)
**Date:** 2026-07-29T18:51Z
**Result:** ALL GREEN

## Prerequisites (P1-P6)

| # | Item | Result | Detail |
|---|------|--------|--------|
| P1 | oline binary on PATH | PASS | `/Users/returniflost/.cargo/bin/oline` v0.1.0 |
| P2 | Buzz reachable at 127.0.0.1:3000 | PASS | liveness=ok, readiness=`{"status":"ready"}` |
| P3 | Event profile exists | PASS | `crates/o-line/config/event-profiles/local-collab-harness.toml` |
| P4 | ~/.oline/ directory | PASS | exists, 17 previous event runs |
| P5 | Docker running | PASS | v29.6.1 (warn-only, non-fatal) |
| P6 | OLINE_HERMES_URL | PASS | unset (non-fatal for local-collab-harness) |

## Events (E1-E3)

| # | Event | Result | Detail |
|---|-------|--------|--------|
| E1 | `oline event ensure local-collab-harness --force` | PASS | all ✓, no ✗; health: green=200 (9ms, 6ms) |
| E2 | `oline event status local-collab-harness --probe` | PASS | liveness=200 (14ms), readiness=200 (9ms) |
| E3 | `oline elevated show buzz` | PASS | role=buzz, kind=external, protected=true, url=http://127.0.0.1:3000 |

## Success criteria

| # | Criterion | Status |
|---|-----------|--------|
| 1 | Buzz health probes pass | ✅ liveness=ok, readiness=ready |
| 2 | Elevated entry registered (role=buzz, kind=external, protected=true) | ✅ |
| 3 | No service failures (no ✗ lines) | ✅ |
| 4 | Run log written | ✅ `~/.oline/event-runs/local-collab-harness-1785351090.json` |
| 5 | Identity isolation intact (forbid_merge) | ✅ |

## Verification command

`just stack-bind` from `crates/o-line/` — exit 0, all 3 steps green.

**Island 2 (oline bind) is GREEN.**