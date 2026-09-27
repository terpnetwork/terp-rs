# Agent brief — CashApp corridor UI + Zakura dest (dao-dao)

**Track:** UI / PrivateCorridor  
**Goal:** Lock Cash App → private bridge → ZEC demo so **dao-dao-ui PrivateCorridor** uses a **Zakura-derived (or Zakura-compatible) ZEC destination** for preauth, and completes the front-end film against hash-market + host automation.

## Context (Cash App → ZEC conversation SSOT)

| Doc | Role |
|-----|------|
| `DEMO-CASHAPP-ZEC-CORRIDOR.md` | Product spine |
| `DESIGN-DECISIONS-CORRIDOR-ACCEPTED` | D1–D7 locked; D6 Zakura; D3 swap required |
| `handoffs/HANDOFF-PRIVATE-BRIDGE-PRODUCTION-READINESS-2026-07-20.md` | Production readiness scores |
| Module | `websites/dao-dao-ui/packages/stateful/modules/modules/PrivateCorridor/` |

## Deliverables

1. **Zakura dest integration in UI**
   - Prefer: fetch/generate UA or t-addr via env `ZAKURA_RPC` / lightwalletd if available
   - Fallback: “paste UA” + validate bech32m-ish prefix (`u1`, `t1`, `tm`, etc.)
   - Optional button: “Use local Zakura demo dest” when RPC returns address
   - Bind with existing `bindingFromDestDisplay` / shared domain

2. **Wire deposit_observed → reverify → automation**
   - On SSE `deposit_observed`, pass `depositReverify: { address, txid, network }` into `runProductionPostDepositSequence`
   - Lab: keep STATUS_FILM or lab banner path
   - Show oracle mid when `oracleBase` set

3. **Demo operator README** in PrivateCorridor or spectrum e2e:
   - Steps: Zakura up → open Private Bridge module → preauth dest from Zakura → BTC QR → fund (lab or Cash App) → observe → mint film

## Constraints

- Do not invent second note format (D1)
- Lab vs production labels honest (D5)
- No mainnet ZEC send required

## Done when

- Module builds (types clean)
- Documented path for Zakura dest
- Production sequence uses reverify when observation has txid

## Return

Files, how to run UI against local Zakura + hash-market, residual.
