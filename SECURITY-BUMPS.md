# Advisory bumps (`security/advisory-bumps`)

Local only. `cargo audit` 0.22.0, `--file` on the four locks this branch changes.
Other locks in the tree were not regenerated. Lock format is v4, so path
packages are not absolute paths and resolve from whichever checkout is the
workspace root.

Wasmer in these locks is crates.io `7.5.0` (it was `7.4.2`). That matches the
sibling `crates/cosmwasm` pin. The singlepass git rev `e1edbf4ff` lives in that
cosmwasm manifest, not in these crates.io rows. Zakura path packages moved from
`1.3.0-alpha.1` to the sibling `../zakura-common` tree at `5.0.0` because the
lock was stale against that path. A crates.io `1.3.0-alpha.1` copy can sit
beside the path copy. This is lock sync, not a new git pin.

| lock | before (vuln / warn) | after (vuln / warn) |
| --- | ---: | ---: |
| `Cargo.lock` | 46 / 30 | 30 / 30 |
| `crates/edgar-cayce/Cargo.lock` | 29 / 24 | 10 / 24 |
| `tools/gen-tools/Cargo.lock` | 1 / 2 | 0 / 2 |
| `tools/headstash/Cargo.lock` | 35 / 16 | 18 / 16 |

`cargo metadata --locked --no-deps` exited 0 for all four manifests.
`CARGO_TARGET_DIR=/tmp/terprs-advisory-target` so this did not write a `target/`.

## Cleared on the locks above

- **RUSTSEC-2026-0344** `ruzstd` `0.9.0` → `0.9.1` (root, edgar-cayce, headstash).
- **RUSTSEC-2026-0285**, **RUSTSEC-2026-0348** `rustls` `0.23.x` → `0.23.46` (root `0.23.43`, headstash `0.23.41`, edgar-cayce `0.23.36`). The `0.21.12` and `0.22.4` lines stay.
- **rustls-webpki** `0.103.x` → `0.103.16`. The `0.101.7` and `0.102.8` lines stay.
- **RUSTSEC-2026-0338**, **RUSTSEC-2026-0341** (and the other `0.11.16` quinn advisories in the before scan) `quinn-proto` → `0.11.18`.
- **RUSTSEC-2026-0383** `webpki-root-certs` `1.0.9` → `1.0.10` (root, edgar-cayce).
- **webpki-roots** `1.0.x` → `1.0.10` where that line was selected. `0.25.4` and `0.26.11` stay on the root lock.
- **RUSTSEC-2026-0414** `pulldown-cmark` `0.13.4` → `0.13.5` (root).
- **RUSTSEC-2026-0410** `zstd-sys` → `2.1.2+zstd.1.5.7`.
- **h2** `0.4.15` / `0.4.14` → `0.4.16`. **h2 `0.3.27` stays** (reqwest 0.11 requires `^0.3`).
- **RUSTSEC-2026-0204** `crossbeam-epoch` `0.9.18` → `0.9.20` (gen-tools, headstash). Root was already `0.9.20`.
- headstash `rkyv` / `rkyv_derive` `0.8.16` → `0.8.17`.

## Left in place

`cargo update -p <crate> --precise <fixed>` did not select these:

- **RUSTSEC-2026-0387** `crypto-bigint 0.5.5`. Fixed in `>= 0.7.6`, and `0.7.6` is yanked. `elliptic-curve 0.13.8` requires `^0.5`. Signature crates were not edited.
- **protobuf `2.28.0`** (RUSTSEC-2024-0437). `opentelemetry-prometheus 0.13` / hermes require `^2.14`, so `3.7.2` does not select.
- **pyo3 `0.24.2`** (RUSTSEC-2026-0176, RUSTSEC-2026-0177). The sdk requires `^0.24`.
- **zip `0.6.6` and `8.6.0`** (RUSTSEC-2026-0367, 0368, 0369). `svm-rs 0.5.29` via foundry-compilers requires `zip ^8`, so `9.0.0` does not select. **zstd-safe `5.0.2`** (RUSTSEC-2026-0403) is that `zip 0.6` / `zstd 0.11` line. `8.2.0` does not select.
- **rustls `0.21.12` and `0.22.4`**, matching **rustls-webpki**, and **webpki-roots `0.25` / `0.26`**. `tokio-rustls 0.25` and `reqwest 0.11` pin `^0.21` / `^0.22`. `ureq 2` pins `webpki-roots ^0.26`.
- **h2 `0.3.27`** (RUSTSEC-2026-0258). Same reqwest 0.11 pin.

Also still reported, not part of this bump: `curve25519-dalek 3.2.0`, `ed25519-dalek 1.0.1`, `ctutils 0.4.2`, and the allowed-warning set (unchanged counts on the root and headstash locks).

Raw logs: `/tmp/terprs-audit-before/`, `/tmp/terprs-audit-after/`.
