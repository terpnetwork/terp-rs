# Agent brief — DEMO corridor preauth + asset backend + oracle bind

**Track:** CORRIDOR (types / pure seams)  
**SPEC:** `docs/plans/spectrum/DEMO-CASHAPP-ZEC-CORRIDOR.md` §3–4, §6  
**Primary:** `docs/plans/spectrum/fixtures/` (+ optional compose hook)

## Goal

Implement **DepositIntentV0** (preauthentication of designation address + rate policy) and the **pluggable asset backend** abstraction so E2E/UI share one packet. Wire **oracle bound** checks to intent policy in pure tests.

## Deliverables

1. Pure crate or module (prefer new `fixtures/cashapp_zec_corridor` **or** extend `compose_seams` / `bridge_auth_seams` if smaller — prefer **dedicated fixture crate** for clarity):
   - `DepositIntentV0` encode/canonical hash `domain_bind`
   - validate expiry, non-empty dest binding
   - `intent_allows_swap(intent, oracle_mid, out_value, actual_owner_binding) -> Result`
2. Tests I1–I6 from SPEC §3.3.
3. `CorridorAssetBackend` enum/trait with:
   - `Simulated` deposit success helper
   - `LightClient` variant present (mock attestation stub OK)
4. Re-export or document for E2E agent.
5. Short pointer in `DEMO-CASHAPP-ZEC-CORRIDOR.md` “Implementation” subsection (or agent fills paths).

## Constraints

- No Docker.
- No dao-dao UI.
- Do not invent second mint contract.
- Dest preauth is **mandatory** on happy path helpers.

## Done when

```bash
cd docs/plans/spectrum/fixtures/<crate> && cargo test
# I1–I6 green
```

## Out of scope

- Live LC networking.
- Full swap circuit.
