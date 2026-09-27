# Private Bridge (code + lab scripts)

**Published research pages** live in **`pages/`** (six MDX files). Vocs copies them at compile (`websites/terp-docs/lib/private-bridge-docs.ts`). Edit `pages/`, not terp-docs stubs.

- [Private bridge](https://docs.terp.network/research/private-bridge)

SPECs and USER-GUIDE in this folder are **crate engineering SSOT**. They are not imported to the public site. `websites/terp-docs/drafts/internal-specs/` is quarantine, not a second SoR.

Lab e2e: **`../../tests/e2e/private-bridge/`**.

Formerly under `docs/plans/spectrum/` (grant layout). **Do not put SSOT back in spectrum.**

## Layout

| Path | Contents |
|------|----------|
| `*.md` (root) | Freezes, SPECs, USER-GUIDE, DEMO |
| `pages/` | Public Vocs MDX (imported by terp-docs) |
| `../../tests/e2e/private-bridge/` | Local multi-net scripts, Zakura, preflight |
| `agents/` | Design/impl STATUS packs |
| `book/` | mdBook |
| `fixtures/README.md` | Pointers to pure crates under `../../crates/` |

## Pure libraries (workspace members)

```text
crates/terp-seams   # dex + bridge + corridor + compose + domain tags
```

## Contracts

| Contract | Path |
|----------|------|
| Private DEX settle | `../../contracts/revenue/private-dex` |
| Bridge mint / egress | `crates/headstash` (cw-headstash) — LC-adjacent |

## Commands

```bash
cd crates/terp-rs
cargo test -p terp-seams --lib

cd ../headstash
just preflight-corridor-local
just demo-corridor-ict-egress-d
```
