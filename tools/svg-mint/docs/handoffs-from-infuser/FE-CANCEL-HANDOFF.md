# FE Cancel checkout — `GET /v1/cancel?recipient={terp1}`

Watch **Cancel checkout** while the desk is still waiting. Same simple CORS GET as `/v1/status` (no JSON POST, no preflight). Never POST `/v1/deposit`.

Call only when status is not Deposited / Entitled / Settled. After success the page drops the locked ZIP-321 QR and the next Pay asks for a new one-time `memo` / `zec_uri`. A 404 means runtime not shipped yet; the tab still unlocks locally. Users must not pay an abandoned QR.

## Runtime

`GET {DREGG}/v1/cancel?recipient={terp1}`

| Case | HTTP | Body |
|---|---|---|
| Unpaid pay-target for that dest | 200 | `{ ok: true, cancelled: true, phase: "waiting" }` — drop pending so next pay-targets cannot reuse `memo_commit` / `zec_uri` / `zec_zat` |
| Already Deposited / Entitled / Settled | 409 | `{ ok: false, error: "payment already seen", phase }` — do **not** drop the cell |
| Unknown dest | 200 | `{ ok: true, cancelled: false }` (idempotent) |

Do not flip global `checkout_ready`. Do not put `terp1` in memos or logs beyond the query.
