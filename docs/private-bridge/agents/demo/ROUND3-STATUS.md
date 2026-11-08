# Round 3 status (plain language)

**Date:** 2026-07-20  
**Theme:** Real implementations + e2e (not just SPECs / pure math)

## What “works” now

### On-chain (real CosmWasm contract)

`cw-headstash` is the **mint router**. You can:

1. Instantiate the contract  
2. Configure bridge: reflection tip, assets, mock verify  
3. Call **`BridgeMintNote`**  
4. See double-mint and H-1 spent-only **rejected** on the real execute path  

**Proof:** `cargo test -p cw-headstash --test test_bridge_e2e` → **6 green**  
**Also:** `cargo test -p zk-test-press --lib --features interface suites::private_bridge` → **8 green** (includes L1 cw-orch Mock mint)

### Pure product path (no chain)

Burn-sketch → SEAM note (with rcm) → swap action → reserve/nullifier update:

```bash
cd docs/plans/spectrum/fixtures/compose_seams && cargo test product_path_burn_to_swap_sketch
```

### Fixture glue (Tacit → mint packet)

Golden JSON + loader + optional anvil docs:

- `docs/plans/spectrum/fixtures/bridge_mint_claim_happy.v1.json`
- `just demo-bridge-fixture` (when wired)

## Core products closed (2026-07-20)

See [CORE-PRODUCTS-CLOSE-2026-07-20.md](../reviews/CORE-PRODUCTS-CLOSE-2026-07-20.md).

| Product | Status |
|---------|--------|
| Poseidon public distro | CLOSED |
| cw-headstash mint + BridgeMintNote L1 Mock | CLOSED |
| SEAM-NOTE-OUT + pure seams L0 | CLOSED |
| Note persist (HeadstashStore + PUT + `put_note_after_mint` + L2 local) | CLOSED |

**Ops notes plane (stable basis):** CLI `headstash-notes` + `[notes_auth]` + optional ciphertext `sha256` + snap recover/PIR.  
Still deferred (not notes ops): H1 composite, Halo2 swap, joint stack e2e.

## What it is *not* yet

| Item | Status |
|------|--------|
| Real SP1 / reflection proof on mint | Mock only (`mock_verify`) |
| Production wallet PUT after mint | Call site closed; CLI/auth ops deferred |
| On-chain private note tree membership write | Attrs / sketch; bank/tree follow-up |
| Halo2 private swap circuit | Pure seams only |
| Anvil + terpd joint network (ict-rs) | Planned; not full compose |
| Headstash claim H1 MockProver | Still last-mile red (separate track) |

## How to talk about maturity

```text
L0 pure seams          ████████████  green (compose, bridge, dex)
L1 cw-headstash e2e    ██████████░░  real contract + Mock suite (mock ZK)
L2 Tacit anvil         ████░░░░░░░░  script/docs; not auto joint mint
L3 dual container      ██░░░░░░░░░░  stub/plan
L4 real LC             █░░░░░░░░░░░  types/mock only
Swap circuit prove     █░░░░░░░░░░░  pure only
Claim circuit H1       ███░░░░░░░░░  last-mile composite red
```

## Commands cheat sheet

```bash
# Contract multi-test e2e (deploy real cw-headstash)
cd crates/headstash && cargo test -p cw-headstash --test test_bridge_e2e

# cw-orch suite L0+L1
cd crates/headstash && cargo test -p zk-test-press --lib --features interface suites::private_bridge

# Pure burn→swap product path
cd docs/plans/spectrum/fixtures/compose_seams && cargo test product_path_burn_to_swap_sketch

# Optional full L0 bundle
cd crates/headstash && just demo-e2e-l0
```

## Agent reports

- `ROUND3-CONTRACT-E2E.md`
- `ROUND3-HARNESS-E2E.md`
- `ROUND3-COMPOSE-SWAP.md`
- `ROUND3-TACIT-GLUE.md`
