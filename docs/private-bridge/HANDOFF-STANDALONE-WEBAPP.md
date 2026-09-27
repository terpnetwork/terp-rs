# Plan: standalone Private Bridge webapp (not a DAO module)

**Goal.** One webapp users open to run the Cash App → BTC QR → Terp private note → optional ZEC dest protocol. It is **not** a dao-dao-ui plugin. It **may copy** the existing `PrivateCorridor` module.

**Do not.** Ship this as `ModuleId.PrivateCorridor` on a DAO. Do not block on passkey / Hermes 2FA. Do not claim mainnet BTC→ZEC until the honesty page says so.

## What already exists (source, don’t fork blindly)

`websites/dao-dao-ui/packages/stateful/modules/modules/PrivateCorridor/`

| File | Reuse |
|------|--------|
| `types.ts` | Intent / receipt / wizard step types |
| `btcAddress.ts`, `browserWallet.ts` | P2WPKH + HD deposit |
| `depositNotify.ts`, `btcReverify.ts` | Observe + re-verify |
| `zakuraDest.ts` | Dest seal (D6) |
| `notesRecover.ts` | PIR / note recover |
| `automationClient.ts` | hash-market lab HTTP |
| `PrivateCorridorWizard.tsx` | UX sequence to lift into a full page |

DAO-only: `index.ts` module registration, Editor/Renderer tab chrome, DAO `headstashContract` defaults.

## Proposed layout

```
websites/private-bridge-app/     # working name
  src/pages/                     # one flow: dest → QR → wait → mint receipt
  src/protocol/                  # copy then slim from PrivateCorridor (no @dao-dao/types)
  src/lab/                       # lab_simulated flag, hash-market base URL
```

Stack: same React/MUI as dao-dao-ui **only if** copy-paste cost is lower than a thin Vite app. Prefer Vite + existing TS files over a new DAO package.

## Product loop (must match crate honesty)

1. Seal ZEC dest (or lab dest).
2. Show **fresh** BTC P2WPKH QR + amount.
3. Poll observe (lab notify or Esplora).
4. Re-verify before mint.
5. Call mint-router / cw-headstash (code ids from public `state.json` when published).
6. Show note handle + optional recover.

Env: `VITE_CHAIN=morocco-1|lab`, `VITE_NOTES_BASE`, `VITE_ORACLE_BASE`, `VITE_HASH_MARKET`, `VITE_HEADSTASH_CODE` — never bake the deploy-script dummy authenticator instance.

## Review checklist for the frontend teammate

- [ ] App boots without a DAO address in the URL.
- [ ] Wizard steps match `for-users.mdx` (crate `pages/`).
- [ ] Lab mode labeled; production backend refuses synthetic observe.
- [ ] No Hermes mesh passkey 2FA mixed in.
- [ ] Copy from PrivateCorridor is a **file list + date**, not an unpublished submodule.

## Sequence

1. Extract protocol TS (types + btc + notify) with no DAO imports — compile in isolation.
2. One-page wizard using that package.
3. Wire lab hash-market (`just demo-corridor-lab`).
4. Wire morocco-1 only when honesty + code ids are public in `state.json`.
