# Agent brief — DEMO corridor UI (dao-dao Module)

**Track:** UI  
**SPEC:** `docs/plans/spectrum/DEMO-CASHAPP-ZEC-CORRIDOR.md` §7  
**Repo:** `websites/dao-dao-ui`

## Goal

Ship a **dao-dao Module** wizard for the Cash App → private bridge → ZEC corridor demo. This is a **Module** container (like Calendar), **not** the Headstash airdrop claim module.

## Deliverables

1. New module package under `packages/stateful/modules/modules/PrivateCorridor/` (or `PrivateBridge/` if naming collision — prefer **PrivateCorridor** to avoid overloading “bridge”).
2. `ModuleId` entry (new id — do not reuse `Headstash`).
3. Export + register in `modules/index.ts` and `core.ts` `getModules()`.
4. Wizard UI:
   - Preauth diversified destination → display binding digest
   - Min out / slip / market id
   - Fresh deposit address + intent id
   - Status steps (deposit → bridge → swap → done)
   - Receipt view
5. Clearly label **Simulated assets** when `assetBackend === 'simulated'`.
6. Types shared with intent JSON from SPEC §3 (can stub API with local state first).

## Constraints

- Do not repurpose `ManageHeadstash` claim flow as this product.
- No requirement for live chain in v0 — mock props / local state OK.
- Match existing module style (Calendar, Marketplace).
- Terp chain gate optional like Calendar.

## Done when

- Module appears in module picker on Terp.
- Full wizard walkthrough with mock success without backend.
- README or module comment points at `DEMO-CASHAPP-ZEC-CORRIDOR.md`.

## Out of scope

- Full cw-orch e2e (E2E track).
- DepositIntent encoding pure crate (Corridor track) — consume their JSON shape when available; otherwise freeze TS types from SPEC.
